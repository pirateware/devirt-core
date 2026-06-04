// Self-implemented hex/byte codec with typed arrays.
function bytesToHex(bytes) {
  let out = "";
  for (const b of bytes) {
    out += (b & 0xff).toString(16).padStart(2, "0");
  }
  return out;
}

function hexToBytes(hex) {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.substr(i * 2, 2), 16);
  }
  return out;
}

function strToBytes(str) {
  const out = new Uint8Array(str.length);
  for (let i = 0; i < str.length; i++) out[i] = str.charCodeAt(i) & 0xff;
  return out;
}

function xorBytes(bytes, key) {
  const out = new Uint8Array(bytes.length);
  for (let i = 0; i < bytes.length; i++) {
    out[i] = bytes[i] ^ key[i % key.length];
  }
  return out;
}

const original = strToBytes("Codec");
const hex = bytesToHex(original);
const back = hexToBytes(hex);

const key = new Uint8Array([0x10, 0x20, 0x30]);
const cipher = xorBytes(original, key);
const plain = xorBytes(cipher, key);

console.log("hex =", hex);
console.log("roundtrip =", bytesToHex(back) === hex);
console.log("cipher hex =", bytesToHex(cipher));
console.log("xor recovered =", String.fromCharCode(...plain));
