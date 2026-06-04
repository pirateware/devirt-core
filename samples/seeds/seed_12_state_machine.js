// Switch-based state machine: a tiny tokenizer for arithmetic expressions.
function tokenize(input) {
  const tokens = [];
  let i = 0;
  let state = "START";
  let buffer = "";

  function flush(type) {
    if (buffer.length) {
      tokens.push({ type, value: buffer });
      buffer = "";
    }
  }

  while (i <= input.length) {
    const ch = i < input.length ? input[i] : "\0";
    switch (state) {
      case "START":
        if (ch >= "0" && ch <= "9") {
          buffer += ch;
          state = "NUMBER";
        } else if ("+-*/()".includes(ch)) {
          tokens.push({ type: "OP", value: ch });
        }
        i++;
        break;
      case "NUMBER":
        if (ch >= "0" && ch <= "9") {
          buffer += ch;
          i++;
        } else {
          flush("NUM");
          state = "START";
        }
        break;
    }
  }
  return tokens;
}

const result = tokenize("12+34*(5-6)");
console.log("token count =", result.length);
for (const t of result) {
  console.log(`${t.type}: ${t.value}`);
}
console.log("nums =", result.filter((t) => t.type === "NUM").map((t) => t.value).join(","));
