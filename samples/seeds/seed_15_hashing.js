// Bitwise hashing: djb2 and FNV-1a over strings.
function djb2(str) {
  let hash = 5381;
  for (let i = 0; i < str.length; i++) {
    hash = ((hash << 5) + hash + str.charCodeAt(i)) >>> 0;
  }
  return hash >>> 0;
}

function fnv1a(str) {
  let hash = 0x811c9dc5;
  for (let i = 0; i < str.length; i++) {
    hash ^= str.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash >>> 0;
}

function toHex(n) {
  return (n >>> 0).toString(16).padStart(8, "0");
}

const inputs = ["", "a", "abc", "hello world", "deobfuscator"];
for (const s of inputs) {
  console.log(`"${s}" djb2=${toHex(djb2(s))} fnv1a=${toHex(fnv1a(s))}`);
}

const combined = inputs.reduce((acc, s) => (acc ^ djb2(s)) >>> 0, 0);
console.log("combined xor =", toHex(combined));
