// try/catch/finally with custom thrown errors.
class ValidationError extends Error {
  constructor(message, field) {
    super(message);
    this.name = "ValidationError";
    this.field = field;
  }
}

function parsePositiveInt(s) {
  const n = Number(s);
  if (Number.isNaN(n)) throw new ValidationError("not a number", s);
  if (!Number.isInteger(n)) throw new ValidationError("not an integer", s);
  if (n <= 0) throw new ValidationError("not positive", s);
  return n;
}

function safeDivide(a, b) {
  let cleanup = "no";
  try {
    if (b === 0) throw new RangeError("divide by zero");
    return a / b;
  } catch (e) {
    return `error: ${e.message}`;
  } finally {
    cleanup = "ran";
    log.push(`finally ${cleanup}`);
  }
}

const log = [];
const inputs = ["42", "abc", "3.5", "-7", "100"];
const results = [];
for (const s of inputs) {
  try {
    results.push(`${s}=ok(${parsePositiveInt(s)})`);
  } catch (e) {
    results.push(`${s}=${e.name}:${e.field}`);
  }
}

console.log("parse =", results.join(" "));
console.log("10/2 =", safeDivide(10, 2));
console.log("5/0 =", safeDivide(5, 0));
console.log("finally count =", log.length);
