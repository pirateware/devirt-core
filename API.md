# Deobfuscator API

Base URL: **`https://api.devirt.dev`**

Send obfuscated JavaScript, get readable JavaScript back. The transform is
semantics-preserving and deterministic.

## `POST /deobfuscate`

Deobfuscate a JS source.

- **Body:** the raw JS source (not JSON-wrapped). `Content-Type: application/javascript`.
- **Max body:** 5 MiB → `413` if larger.
- **Rate limit:** 30 requests / 60 s per IP → `429` if exceeded.

### Response `200` (JSON)

```json
{
  "code": "console.log(\"Hello, world!\");\n",
  "error": null,
  "rounds": 4,
  "converged": true,
  "parse_errors": [],
  "changes": [{ "pass": "decoder-lift", "rounds": 1 }]
}
```

| field | type | meaning |
|-------|------|---------|
| `code` | string | the deobfuscated source (the main output) |
| `error` | string \| null | non-null = the pipeline declined/panicked; `code` is then the **input unchanged** (best-effort, still `200`) |
| `rounds` | number | fixpoint rounds run |
| `converged` | boolean | whether the pipeline reached a fixpoint |
| `parse_errors` | string[] | parser diagnostics (input may be partially invalid) |
| `changes` | `{pass,rounds}[]` | per-pass activity (how many rounds each pass changed something) |

> **Best-effort contract:** `error != null` with HTTP `200` means deobfuscation
> safely declined — you get your input back, never broken code. Treat it as a
> soft outcome, not a failure.

### Status codes

| code | meaning |
|------|---------|
| `200` | success (see `error` for soft declines) |
| `400` | empty or truncated body |
| `413` | body over 5 MiB |
| `429` | rate limited |
| `504` | deobfuscation exceeded the 30 s wall-clock cap |
| `502` / `500` | upstream container / internal error |

## `GET /health`

Liveness check. Returns `200` with body `ok`.

## Examples

**curl**

```sh
curl -s -X POST --data-binary @obfuscated.js \
  -H 'content-type: application/javascript' \
  https://api.devirt.dev/deobfuscate | jq -r .code
```

**JavaScript (fetch)**

```js
const res = await fetch("https://api.devirt.dev/deobfuscate", {
  method: "POST",
  headers: { "content-type": "application/javascript" },
  body: obfuscatedSource,
});
const { code, error } = await res.json();
if (error) console.warn("declined:", error); // `code` is the original input
console.log(code);
```

## Notes

- **Latency** is roughly proportional to input size and obfuscation complexity:
  sub-second for small/medium files, ~4 s for a ~1 MB heavily-obfuscated bundle.
- The service runs untrusted JS inside a sandbox; it makes **no network calls** on
  your behalf and stores nothing.

See [`deploy/README.md`](deploy/README.md) for deployment/operations.
