//! Minimal, dependency-free HTTP front end for the `deob` deobfuscator.
//!
//! Designed to run inside a Cloudflare Container, behind a Worker that terminates
//! TLS and handles edge concerns (WAF, DDoS, rate limiting, size limits). This
//! process therefore only ever talks to the Worker over Cloudflare's internal
//! network — it is not directly exposed to the raw internet — but it still applies
//! its own defensive limits as belt-and-suspenders.
//!
//! Design:
//!   * One OS thread per connection, bounded by a concurrency cap (excess → 503).
//!   * Each `POST /deobfuscate` runs the source through a fresh `deob --stdin
//!     --json` **child process**. The child is the only place untrusted JS is
//!     executed (inside `deob`'s QuickJS sandbox), and being a separate process
//!     means a runaway can be **SIGKILL**ed — the hard wall-clock guarantee an
//!     in-process thread can't give. On timeout we kill the child and return 504.
//!
//! Routes:
//!   GET  /health | /healthz | /ready  → 200 "ok"
//!   GET  /                            → 200 banner
//!   POST /deobfuscate                 → 200 application/json (the `deob` Report)
//!                                        413 too large · 504 timeout · 500 error
//!
//! Config (env):
//!   PORT                 (default 8080)
//!   MAX_BODY_BYTES       (default 5_242_880 = 5 MiB)
//!   TIMEOUT_SECS         (default 30)
//!   MAX_CONCURRENCY      (default = available CPU parallelism)
//!   DEOB_BIN             (default "deob"; in the image: /usr/local/bin/deob)
//!   SOCKET_TIMEOUT_SECS  (default 20; slowloris guard on header/body reads)
//!   MAX_HEADER_BYTES     (default 16_384)

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

struct Config {
    port: u16,
    max_body: usize,
    timeout: Duration,
    max_concurrency: usize,
    deob_bin: String,
    sock_timeout: Duration,
    max_header: usize,
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

impl Config {
    fn from_env() -> Config {
        let cpus = thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
        Config {
            port: env_parse("PORT", 8080u16),
            max_body: env_parse("MAX_BODY_BYTES", 5 * 1024 * 1024usize),
            timeout: Duration::from_secs(env_parse("TIMEOUT_SECS", 30u64)),
            max_concurrency: env_parse("MAX_CONCURRENCY", cpus),
            deob_bin: std::env::var("DEOB_BIN").unwrap_or_else(|_| "deob".to_string()),
            sock_timeout: Duration::from_secs(env_parse("SOCKET_TIMEOUT_SECS", 20u64)),
            max_header: env_parse("MAX_HEADER_BYTES", 16 * 1024usize),
        }
    }
}

fn main() {
    let cfg = Arc::new(Config::from_env());
    let addr = format!("0.0.0.0:{}", cfg.port);
    let listener = TcpListener::bind(&addr).unwrap_or_else(|e| {
        eprintln!("fatal: cannot bind {addr}: {e}");
        std::process::exit(1);
    });
    eprintln!(
        "deob-server listening on {addr} (max_body={} timeout={}s concurrency={} deob={})",
        cfg.max_body,
        cfg.timeout.as_secs(),
        cfg.max_concurrency,
        cfg.deob_bin
    );

    let active = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        // Admission: bounded concurrency. Excess connections are shed immediately
        // with 503 rather than queued, so a burst can't pile up unbounded.
        if active.fetch_add(1, Ordering::SeqCst) >= cfg.max_concurrency {
            active.fetch_sub(1, Ordering::SeqCst);
            let _ = write_response(&mut stream, "503 Service Unavailable", "text/plain", b"at capacity\n");
            continue;
        }
        let cfg = cfg.clone();
        let active = active.clone();
        thread::spawn(move || {
            if let Err(e) = handle(&cfg, stream) {
                // Connection-level I/O errors (client hangup, timeout) are routine.
                if e.kind() != io::ErrorKind::WouldBlock {
                    eprintln!("conn error: {e}");
                }
            }
            active.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

/// Parsed request head: method, path (query stripped later), and Content-Length.
struct Head {
    method: String,
    path: String,
    content_length: usize,
}

fn read_head(reader: &mut impl BufRead, max_header: usize) -> io::Result<Option<Head>> {
    let mut line = Vec::new();
    let mut consumed = reader.read_until(b'\n', &mut line)?;
    if consumed == 0 {
        return Ok(None); // client closed without sending anything
    }
    let request_line = String::from_utf8_lossy(&line);
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    let mut content_length = 0usize;
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        consumed += n;
        if consumed > max_header {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "header too large"));
        }
        let header = String::from_utf8_lossy(&line);
        let header = header.trim_end();
        if header.is_empty() {
            break; // end of headers
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    Ok(Some(Head {
        method,
        path,
        content_length,
    }))
}

fn handle(cfg: &Config, mut stream: TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(cfg.sock_timeout))?;
    stream.set_write_timeout(Some(cfg.sock_timeout))?;
    let started = Instant::now();

    let mut reader = BufReader::new(stream.try_clone()?);
    let head = match read_head(&mut reader, cfg.max_header) {
        Ok(Some(h)) => h,
        Ok(None) => return Ok(()),
        Err(_) => return write_response(&mut stream, "400 Bad Request", "text/plain", b"bad request\n"),
    };

    let path = head.path.split('?').next().unwrap_or("");
    let (status, in_bytes) = match (head.method.as_str(), path) {
        ("GET", "/health" | "/healthz" | "/ready") => {
            write_response(&mut stream, "200 OK", "text/plain", b"ok\n")?;
            ("200", 0)
        }
        ("GET", "/") => {
            write_response(&mut stream, "200 OK", "text/plain", b"js-deobfuscator: POST /deobfuscate\n")?;
            ("200", 0)
        }
        ("POST", "/deobfuscate") => {
            if head.content_length == 0 {
                write_response(&mut stream, "400 Bad Request", "text/plain", b"empty body\n")?;
                ("400", 0)
            } else if head.content_length > cfg.max_body {
                write_response(&mut stream, "413 Payload Too Large", "text/plain", b"payload too large\n")?;
                ("413", head.content_length)
            } else {
                let mut body = vec![0u8; head.content_length];
                if reader.read_exact(&mut body).is_err() {
                    write_response(&mut stream, "400 Bad Request", "text/plain", b"truncated body\n")?;
                    ("400", head.content_length)
                } else {
                    match run_deob(cfg, &body) {
                        Outcome::Ok(json) => {
                            write_response(&mut stream, "200 OK", "application/json", &json)?;
                            ("200", body.len())
                        }
                        Outcome::Timeout => {
                            write_response(
                                &mut stream,
                                "504 Gateway Timeout",
                                "application/json",
                                br#"{"error":"deobfuscation timed out"}"#,
                            )?;
                            ("504", body.len())
                        }
                        Outcome::Error(e) => {
                            eprintln!("deob error: {e}");
                            write_response(
                                &mut stream,
                                "500 Internal Server Error",
                                "application/json",
                                br#"{"error":"internal error"}"#,
                            )?;
                            ("500", body.len())
                        }
                    }
                }
            }
        }
        _ => {
            write_response(&mut stream, "404 Not Found", "text/plain", b"not found\n")?;
            ("404", 0)
        }
    };

    // Access log — never logs the body (attacker-controlled / possibly large).
    eprintln!(
        "{} {} {} {}b {}ms",
        head.method,
        path,
        status,
        in_bytes,
        started.elapsed().as_millis()
    );
    Ok(())
}

enum Outcome {
    Ok(Vec<u8>),
    Timeout,
    Error(String),
}

/// Run `src` through `deob --stdin --json` in a child process with a hard
/// wall-clock cap. On timeout the child is SIGKILLed (uncatchable), so a runaway
/// or non-terminating "decoder" cannot pin a worker thread forever.
fn run_deob(cfg: &Config, src: &[u8]) -> Outcome {
    let mut child = match Command::new(&cfg.deob_bin)
        .arg("--stdin")
        .arg("--json")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return Outcome::Error(format!("spawn {}: {e}", cfg.deob_bin)),
    };

    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");
    let src = src.to_vec();
    let (tx, rx) = mpsc::channel();

    // Writer+reader thread: `deob` reads all of stdin before emitting any stdout,
    // so writing the whole body then reading to EOF cannot deadlock on the pipe.
    thread::spawn(move || {
        let _ = stdin.write_all(&src);
        drop(stdin); // EOF — let the child finish reading
        let mut out = Vec::new();
        let result = stdout.read_to_end(&mut out).map(|_| out);
        let _ = tx.send(result); // receiver may be gone (timeout) — that's fine
    });

    match rx.recv_timeout(cfg.timeout) {
        Ok(Ok(out)) => {
            let _ = child.wait();
            Outcome::Ok(out)
        }
        Ok(Err(e)) => {
            let _ = child.kill();
            let _ = child.wait();
            Outcome::Error(format!("read child: {e}"))
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = child.kill();
            let _ = child.wait();
            Outcome::Timeout
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = child.kill();
            let _ = child.wait();
            Outcome::Error("worker thread disconnected".to_string())
        }
    }
}

fn write_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) -> io::Result<()> {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}
