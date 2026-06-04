//! Integration tests for the container HTTP server's request handling.
//!
//! Uses `/bin/cat` as a stand-in `DEOB_BIN` so the routes that don't depend on
//! real deobfuscation output (health, 404, empty→400, oversize→413, and that a
//! normal POST reaches the child-process path and returns 200) are covered
//! without needing the `deob` binary built/located cross-crate.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command};
use std::time::Duration;

fn start(port: u16) -> Child {
    let child = Command::new(env!("CARGO_BIN_EXE_server"))
        .env("PORT", port.to_string())
        .env("DEOB_BIN", "/bin/cat")
        .env("MAX_BODY_BYTES", "1000")
        .env("MAX_CONCURRENCY", "8")
        .spawn()
        .expect("spawn server binary");
    // Wait for the listener to come up.
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return child;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("server did not start listening on port {port}");
}

fn request(port: u16, raw: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(raw.as_bytes()).unwrap();
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf); // server sends Connection: close
    buf
}

#[test]
fn routes_and_limits() {
    // Port derived from PID to avoid collisions between concurrent test runs.
    let port = 8000 + (std::process::id() % 1000) as u16;
    let mut child = start(port);

    let health = request(port, "GET /health HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(health.contains("200 OK"), "health: {health}");

    let not_found = request(port, "GET /nope HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(not_found.contains("404"), "404: {not_found}");

    let empty = request(
        port,
        "POST /deobfuscate HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\n\r\n",
    );
    assert!(empty.contains("400"), "empty body: {empty}");

    // Declared Content-Length exceeds MAX_BODY_BYTES (1000) → rejected pre-read.
    let oversize = request(
        port,
        "POST /deobfuscate HTTP/1.1\r\nHost: x\r\nContent-Length: 5000\r\n\r\n",
    );
    assert!(oversize.contains("413"), "oversize: {oversize}");

    // A normal small POST reaches the child-process path and returns 200.
    let body = "x";
    let ok = request(
        port,
        &format!(
            "POST /deobfuscate HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        ),
    );
    assert!(ok.contains("200 OK"), "post: {ok}");

    let _ = child.kill();
    let _ = child.wait();
}
