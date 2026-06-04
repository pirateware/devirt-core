// Closures: counter factory, accumulator, memoization.
function makeCounter(start, step) {
  let value = start;
  return {
    next() {
      const cur = value;
      value += step;
      return cur;
    },
    reset() {
      value = start;
    },
  };
}

function makeAccumulator() {
  let sum = 0;
  return (x) => (sum += x);
}

function memoize(fn) {
  const cache = new Map();
  return (n) => {
    if (cache.has(n)) return cache.get(n);
    const result = fn(n);
    cache.set(n, result);
    return result;
  };
}

const counter = makeCounter(10, 5);
const ticks = [counter.next(), counter.next(), counter.next()];
counter.reset();
ticks.push(counter.next());

const acc = makeAccumulator();
const sums = [acc(1), acc(2), acc(3), acc(4)];

let calls = 0;
const square = memoize((n) => {
  calls++;
  return n * n;
});
const squares = [square(3), square(3), square(4), square(4), square(5)];

console.log("ticks =", ticks.join(","));
console.log("running sums =", sums.join(","));
console.log("squares =", squares.join(","), "calls =", calls);
