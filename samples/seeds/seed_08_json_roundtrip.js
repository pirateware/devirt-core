// JSON build + stringify + parse roundtrip with transformation.
function buildRecord(id, tags, scores) {
  return {
    id,
    tags,
    scores,
    meta: {
      count: scores.length,
      total: scores.reduce((a, b) => a + b, 0),
    },
  };
}

const records = [
  buildRecord(1, ["a", "b"], [10, 20, 30]),
  buildRecord(2, ["b", "c"], [5, 15]),
  buildRecord(3, ["a", "c", "d"], [100]),
];

const payload = JSON.stringify({ version: 1, records });
const parsed = JSON.parse(payload);

const enriched = parsed.records.map((r) => ({
  id: r.id,
  avg: r.meta.total / r.meta.count,
  tagString: r.tags.sort().join("|"),
}));

console.log("payload length =", payload.length);
console.log("roundtrip ok =", JSON.stringify(parsed) === payload);
for (const e of enriched) {
  console.log(`record ${e.id}: avg=${e.avg} tags=${e.tagString}`);
}
