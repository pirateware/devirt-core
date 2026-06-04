// Array sort + dedup + binary search.
function dedupeSorted(arr) {
  const sorted = arr.slice().sort((a, b) => a - b);
  const out = [];
  for (const v of sorted) {
    if (out.length === 0 || out[out.length - 1] !== v) {
      out.push(v);
    }
  }
  return out;
}

function binarySearch(arr, target) {
  let lo = 0;
  let hi = arr.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (arr[mid] === target) return mid;
    if (arr[mid] < target) lo = mid + 1;
    else hi = mid - 1;
  }
  return -1;
}

const raw = [9, 3, 7, 3, 1, 9, 5, 7, 2, 8, 1, 6, 4, 5];
const clean = dedupeSorted(raw);

console.log("sorted unique =", clean.join(","));
console.log("length =", clean.length);

for (const t of [1, 5, 8, 10]) {
  console.log(`find ${t} -> index ${binarySearch(clean, t)}`);
}
