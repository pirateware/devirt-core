// Nested object/array transform: flatten, group, restructure.
const catalog = {
  electronics: {
    phones: [
      { name: "P1", price: 600 },
      { name: "P2", price: 900 },
    ],
    laptops: [{ name: "L1", price: 1200 }],
  },
  books: {
    fiction: [
      { name: "B1", price: 15 },
      { name: "B2", price: 20 },
    ],
  },
};

function flatten(cat) {
  const items = [];
  for (const [dept, subcats] of Object.entries(cat)) {
    for (const [sub, products] of Object.entries(subcats)) {
      for (const p of products) {
        items.push({ dept, sub, name: p.name, price: p.price });
      }
    }
  }
  return items;
}

const flat = flatten(catalog);
const totalByDept = flat.reduce((acc, it) => {
  acc[it.dept] = (acc[it.dept] || 0) + it.price;
  return acc;
}, {});

const expensive = flat
  .filter((it) => it.price >= 100)
  .map((it) => `${it.name}@${it.price}`)
  .sort();

console.log("flat count =", flat.length);
console.log("total by dept =", JSON.stringify(totalByDept));
console.log("expensive =", expensive.join(","));
console.log("grand total =", flat.reduce((s, it) => s + it.price, 0));
