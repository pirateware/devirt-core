// Arithmetic loops: sums, products, primes via trial division.
function sumOfSquares(n) {
  let total = 0;
  for (let i = 1; i <= n; i++) {
    total += i * i;
  }
  return total;
}

function factorialProduct(n) {
  let prod = 1;
  for (let i = 2; i <= n; i++) prod *= i;
  return prod;
}

function isPrime(n) {
  if (n < 2) return false;
  for (let d = 2; d * d <= n; d++) {
    if (n % d === 0) return false;
  }
  return true;
}

function primesBelow(limit) {
  const out = [];
  for (let i = 2; i < limit; i++) {
    if (isPrime(i)) out.push(i);
  }
  return out;
}

const primes = primesBelow(40);
const harmonicTimes1000 = primes.reduce((acc, p) => acc + Math.floor(1000 / p), 0);

console.log("sumOfSquares(10) =", sumOfSquares(10));
console.log("factorial(8) =", factorialProduct(8));
console.log("primes < 40 =", primes.join(","));
console.log("count =", primes.length, "harmonic*1000 =", harmonicTimes1000);
