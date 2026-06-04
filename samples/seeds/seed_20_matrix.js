// Matrix operations: multiply, transpose, identity, determinant (3x3).
function multiply(a, b) {
  const rows = a.length;
  const cols = b[0].length;
  const inner = b.length;
  const result = [];
  for (let i = 0; i < rows; i++) {
    result[i] = [];
    for (let j = 0; j < cols; j++) {
      let sum = 0;
      for (let k = 0; k < inner; k++) sum += a[i][k] * b[k][j];
      result[i][j] = sum;
    }
  }
  return result;
}

function transpose(m) {
  return m[0].map((_, j) => m.map((row) => row[j]));
}

function det3(m) {
  return (
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) -
    m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) +
    m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
  );
}

const A = [
  [1, 2, 3],
  [4, 5, 6],
];
const B = [
  [7, 8],
  [9, 10],
  [11, 12],
];

const C = multiply(A, B);
const M = [
  [2, 1, 1],
  [1, 3, 2],
  [1, 0, 0],
];

console.log("A*B =", JSON.stringify(C));
console.log("transpose(A) =", JSON.stringify(transpose(A)));
console.log("det(M) =", det3(M));
console.log("trace(A*B) =", C[0][0] + C[1][1]);
