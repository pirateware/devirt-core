# Deploy — request-based service on Cloudflare Containers

A two-tier setup: a **Worker** (edge front door) routes `POST /deobfuscate` to a
**Container** running the `deob` deobfuscator behind a tiny HTTP server.

```
client ─► Cloudflare edge (WAF / DDoS / rate-limit) ─► Worker ─► Container ─► deob (QuickJS sandbox)
```

## Layout

| Path | What |
|------|------|
| `/Dockerfile` (repo root) | builds `deob` + `server` into a distroless image |
| `server/` (Rust crate) | the in-container HTTP server (`server/src/main.rs`) |
| `deploy/wrangler.jsonc` | Worker + container + DO + rate-limit config |
| `deploy/worker/index.ts` | the front-door Worker |

The container server shells out to `deob --stdin --json` per request, with a hard
wall-clock kill — see `server/src/main.rs`.

## Prerequisites

- A Cloudflare account on a **Workers Paid** plan (Containers require it).
- Docker running locally (wrangler builds the image), or use Cloudflare remote
  builds.
- Node 18+.

## Build & run locally (no Cloudflare)

```sh
# from the repo root
cargo build --release --workspace --bins
DEOB_BIN=$PWD/target/release/deob PORT=8080 ./target/release/server &
curl -s -X POST --data-binary @samples/sample_12.js localhost:8080/deobfuscate | jq .code
```

Or the whole container:

```sh
docker build -t deob .
docker run --rm -p 8080:8080 deob
```

## Deploy to Cloudflare

```sh
cd deploy
npm install
npx wrangler deploy        # builds the image, pushes it, wires Worker↔container
```

Test the deployed endpoint:

```sh
curl -s -X POST --data-binary @../samples/sample_12.js \
  https://js-deobfuscator.<your-subdomain>.workers.dev/deobfuscate | jq .code
```

## Sizing (read this)

`deob` is **CPU-bound**. Measured on a full modern core: ~4 s for a ~0.9 MB
reCAPTCHA bundle, ~1 s for ~0.7 MB, <0.3 s for ~0.3 MB. Cloudflare's vCPU is
**fractional below `standard-2`**, so latency scales ~linearly:

| instance | vCPU | mem | ~0.9 MB input |
|----------|------|-----|---------------|
| basic | 1/4 | 1 GiB | ~16 s ❌ |
| standard-1 | 1/2 | 4 GiB | ~8 s ⚠️ |
| **standard-2** | 1 | 6 GiB | ~4 s (min) |
| **standard-3** | 2 | 8 GiB | ~4 s + 2 concurrent ✅ default |

`wrangler.jsonc` ships `standard-3`. Memory is never the constraint here.

## Tuning knobs

Container server (env, set in the `Dockerfile` or `wrangler.jsonc` container vars):

| Var | Default | Meaning |
|-----|---------|---------|
| `PORT` | 8080 | listen port (must match the Worker's `defaultPort`) |
| `TIMEOUT_SECS` | 30 | wall-clock cap; child is SIGKILLed past this → 504 |
| `MAX_BODY_BYTES` | 5 MiB | request size cap → 413 |
| `MAX_CONCURRENCY` | #vCPU | concurrent deobs per instance; excess → 503 |
| `SOCKET_TIMEOUT_SECS` | 20 | slowloris guard on socket reads |

Worker (`deploy/worker/index.ts` / `wrangler.jsonc`):
- `POOL_SIZE` (worker) — number of warm instances to spread load across (≤ `max_instances`).
- `ratelimits` (wrangler) — per-IP `limit`/`period`; the public-abuse brake.
- `max_instances` (wrangler) — autoscaling ceiling.
- `sleepAfter` (worker) — how long an instance stays warm after the last request.

## Response contract

`POST /deobfuscate` (body = raw JS) → JSON:

```json
{ "code": "...", "error": null, "rounds": 4, "converged": true,
  "parse_errors": [], "changes": [{ "pass": "...", "rounds": 1 }] }
```

`deob` is best-effort: on a contained pipeline panic it returns the **input
unchanged** with `error` set and HTTP **200**. Surface `error`; don't treat it as
5xx. Other statuses: 400 (empty/bad), 413 (too large), 429 (rate limited),
504 (timeout — child killed), 502/500 (upstream/internal).

## Hardening notes (fully-untrusted input)

Layers, weakest → strongest: QuickJS sandbox (128 MiB / 2 MiB stack / 2 s per
eval) → subprocess-per-request with SIGKILL on timeout → distroless non-root
image → Cloudflare's per-container VM isolation → edge WAF/DDoS/rate-limit. The
container needs **no network egress** (neither `deob` nor QuickJS makes network
calls); leave egress unused.

**Before going public, run a hostile-input load test** (deeply nested JS,
non-terminating "decoders", 5 MiB blobs) and confirm 504/413/503 all fire and the
instance stays healthy.
