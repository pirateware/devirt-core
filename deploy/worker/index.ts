// Cloudflare Worker front door for the deobfuscation service.
//
// Responsibilities (the edge concerns kept OUT of the container):
//   * route + method/path validation
//   * per-IP rate limiting (the public-abuse brake)
//   * request size guard (before any container is touched)
//   * load-balance across a small pool of warm container instances
//
// The container itself runs `deob` per request in a sandboxed child process with
// a hard wall-clock kill (see server/src/main.rs). WAF / DDoS protection are
// configured at the zone level in the Cloudflare dashboard.

import { Container, getContainer } from "@cloudflare/containers";

const MAX_BODY = 5 * 1024 * 1024; // keep in sync with the container's MAX_BODY_BYTES
const POOL_SIZE = 5; // spread load across N warm instances (<= max_instances)

interface Env {
  DEOB_CONTAINER: DurableObjectNamespace<DeobContainer>;
  // Optional: present only if the `ratelimits` binding is configured.
  RATE_LIMITER?: { limit(opts: { key: string }): Promise<{ success: boolean }> };
}

export class DeobContainer extends Container<Env> {
  // Must match the server's PORT.
  defaultPort = 8080;
  // Keep an instance warm briefly after the last request so the next request
  // doesn't pay the 1–3 s cold start.
  sleepAfter = "5m";
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);

    if (request.method === "GET" && (url.pathname === "/health" || url.pathname === "/")) {
      return new Response("ok\n", { headers: { "content-type": "text/plain" } });
    }
    if (request.method !== "POST" || url.pathname !== "/deobfuscate") {
      return json({ error: "not found" }, 404);
    }

    // Per-IP rate limit (no-op if the binding isn't configured).
    if (env.RATE_LIMITER) {
      const ip = request.headers.get("cf-connecting-ip") ?? "unknown";
      const { success } = await env.RATE_LIMITER.limit({ key: ip });
      if (!success) return json({ error: "rate limited" }, 429);
    }

    // Buffer the body so we can (a) enforce the real size and (b) forward it with
    // an explicit Content-Length — the container server requires Content-Length
    // (it does not implement chunked transfer encoding).
    const body = await request.arrayBuffer();
    if (body.byteLength === 0) return json({ error: "empty body" }, 400);
    if (body.byteLength > MAX_BODY) return json({ error: "payload too large" }, 413);

    // Spread requests across a small pool of warm container instances.
    const slot = Math.floor(Math.random() * POOL_SIZE).toString();
    const container = getContainer(env.DEOB_CONTAINER, slot);

    try {
      return await container.fetch(
        new Request("http://container/deobfuscate", {
          method: "POST",
          body,
          headers: { "content-type": "application/javascript" },
        }),
      );
    } catch {
      return json({ error: "upstream container error" }, 502);
    }
  },
};

function json(obj: unknown, status: number): Response {
  return new Response(JSON.stringify(obj), {
    status,
    headers: { "content-type": "application/json" },
  });
}
