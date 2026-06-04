// Generator functions producing sequences.
function* range(start, end, step) {
  for (let i = start; i < end; i += step) yield i;
}

function* fibGen(count) {
  let a = 0;
  let b = 1;
  for (let i = 0; i < count; i++) {
    yield a;
    [a, b] = [b, a + b];
  }
}

function* take(iter, n) {
  let i = 0;
  for (const v of iter) {
    if (i++ >= n) return;
    yield v;
  }
}

function* primes() {
  const found = [];
  let candidate = 2;
  while (true) {
    if (found.every((p) => candidate % p !== 0)) {
      found.push(candidate);
      yield candidate;
    }
    candidate++;
  }
}

const evens = [...range(0, 20, 2)];
const fibs = [...fibGen(10)];
const firstPrimes = [...take(primes(), 8)];

console.log("evens =", evens.join(","));
console.log("fibs =", fibs.join(","));
console.log("primes =", firstPrimes.join(","));
console.log("sum of fibs =", fibs.reduce((a, b) => a + b, 0));
