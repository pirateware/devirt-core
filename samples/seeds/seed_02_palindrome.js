// String reversal and palindrome detection.
function reverse(str) {
  let out = "";
  for (let i = str.length - 1; i >= 0; i--) {
    out += str[i];
  }
  return out;
}

function normalize(str) {
  let out = "";
  for (const ch of str.toLowerCase()) {
    if ((ch >= "a" && ch <= "z") || (ch >= "0" && ch <= "9")) {
      out += ch;
    }
  }
  return out;
}

function isPalindrome(str) {
  const norm = normalize(str);
  return norm === reverse(norm);
}

const samples = [
  "racecar",
  "A man, a plan, a canal: Panama",
  "Hello, World!",
  "Was it a car or a cat I saw?",
  "12321",
];

for (const s of samples) {
  console.log(`${isPalindrome(s) ? "YES" : "no "} | ${reverse(s)}`);
}
