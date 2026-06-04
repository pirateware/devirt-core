// Regex matching, capture groups, and replace.
const text =
  "Contact: alice@example.com, bob_99@test.org; phone 555-1234 and 800-9999.";

const emailRe = /([a-z0-9_]+)@([a-z]+)\.([a-z]+)/g;
const emails = [];
let m;
while ((m = emailRe.exec(text)) !== null) {
  emails.push({ user: m[1], domain: m[2], tld: m[3] });
}

const phones = text.match(/\d{3}-\d{4}/g) || [];

const masked = text.replace(/([a-z0-9_]+)@([a-z]+)\.([a-z]+)/g, "***@$2.$3");

const words = text.match(/[A-Za-z]+/g) || [];
const longWords = words.filter((w) => w.length >= 5);

console.log("emails =", emails.map((e) => `${e.user}/${e.domain}`).join(", "));
console.log("phones =", phones.join(", "));
console.log("masked =", masked);
console.log("long words =", longWords.join(","));
