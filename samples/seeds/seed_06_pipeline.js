// Higher-order map/filter/reduce pipeline.
const people = [
  { name: "Alice", age: 30, city: "NY" },
  { name: "Bob", age: 17, city: "LA" },
  { name: "Carol", age: 25, city: "NY" },
  { name: "Dave", age: 42, city: "SF" },
  { name: "Eve", age: 19, city: "LA" },
  { name: "Frank", age: 55, city: "SF" },
];

const compose = (...fns) => (x) => fns.reduceRight((acc, fn) => fn(acc), x);

const adults = people.filter((p) => p.age >= 18);
const names = adults.map((p) => p.name.toUpperCase());
const totalAge = adults.reduce((sum, p) => sum + p.age, 0);
const avgAge = totalAge / adults.length;

const byCity = adults.reduce((acc, p) => {
  (acc[p.city] = acc[p.city] || []).push(p.name);
  return acc;
}, {});

const transform = compose(
  (n) => n + 1,
  (n) => n * 2,
  (n) => n - 3
);

console.log("adult names =", names.join(","));
console.log("total age =", totalAge, "avg =", avgAge.toFixed(2));
console.log("by city =", JSON.stringify(byCity));
console.log("compose(10) =", transform(10));
