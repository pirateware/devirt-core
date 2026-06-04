// Self-implemented base64 encoder/decoder (ASCII input).
const ALPHA = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

function b64encode(str) {
  const bytes = [];
  for (let i = 0; i < str.length; i++) bytes.push(str.charCodeAt(i) & 0xff);
  let out = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const b0 = bytes[i];
    const b1 = i + 1 < bytes.length ? bytes[i + 1] : 0;
    const b2 = i + 2 < bytes.length ? bytes[i + 2] : 0;
    const triple = (b0 << 16) | (b1 << 8) | b2;
    out += ALPHA[(triple >> 18) & 63];
    out += ALPHA[(triple >> 12) & 63];
    out += i + 1 < bytes.length ? ALPHA[(triple >> 6) & 63] : "=";
    out += i + 2 < bytes.length ? ALPHA[triple & 63] : "=";
  }
  return out;
}

function b64decode(str) {
  const clean = str.replace(/=+$/, "");
  let out = "";
  let buffer = 0;
  let bits = 0;
  for (const ch of clean) {
    buffer = (buffer << 6) | ALPHA.indexOf(ch);
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out += String.fromCharCode((buffer >> bits) & 0xff);
    }
  }
  return out;
}

const inputs = ["Hello", "JavaScript", "base64!?", "x"];
for (const s of inputs) {
  const enc = b64encode(s);
  const dec = b64decode(enc);
  console.log(`${s} -> ${enc} -> ${dec} (${dec === s})`);
}
