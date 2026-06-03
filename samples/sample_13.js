var VM = (function () {
  var P = [0x01, 0x07, 0x01, 0x23, 0x02, 0x03], S = [];
  var OPS = {
    0x01: function (a) { S.push(a); },
    0x02: function () { var b = S.pop(), a = S.pop(); S.push(a + b); },
    0x03: function () { return S.pop(); }
  };
  var ip = 0;
  while (ip < P.length) {
    var op = P[ip++];
    if (op === 0x03) return OPS[op]();
    if (op === 0x01) OPS[op](P[ip++]);
    else OPS[op]();
  }
})();
console.log(VM);
