// Map/Set operations: word frequency, unique tokens, set algebra.
const text =
  "the quick brown fox the lazy dog the fox jumps over the lazy dog";

const words = text.split(/\s+/);
const freq = new Map();
for (const w of words) {
  freq.set(w, (freq.get(w) || 0) + 1);
}

const sortedFreq = [...freq.entries()].sort((a, b) =>
  b[1] - a[1] || a[0].localeCompare(b[0])
);

const unique = new Set(words);

const setA = new Set([1, 2, 3, 4, 5]);
const setB = new Set([4, 5, 6, 7]);
const union = new Set([...setA, ...setB]);
const intersection = new Set([...setA].filter((x) => setB.has(x)));
const difference = new Set([...setA].filter((x) => !setB.has(x)));

console.log("total words =", words.length, "unique =", unique.size);
for (const [w, c] of sortedFreq.slice(0, 3)) {
  console.log(`${w}: ${c}`);
}
console.log("union =", [...union].join(","));
console.log("intersection =", [...intersection].join(","));
console.log("difference =", [...difference].join(","));
