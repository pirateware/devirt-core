// Recursion: factorial, fibonacci, and a binary-tree depth/sum walk.
function factorial(n) {
  return n <= 1 ? 1 : n * factorial(n - 1);
}

function fib(n) {
  if (n < 2) return n;
  return fib(n - 1) + fib(n - 2);
}

function buildTree(depth, value) {
  if (depth === 0) return { value, left: null, right: null };
  return {
    value,
    left: buildTree(depth - 1, value * 2),
    right: buildTree(depth - 1, value * 2 + 1),
  };
}

function treeSum(node) {
  if (node === null) return 0;
  return node.value + treeSum(node.left) + treeSum(node.right);
}

function treeDepth(node) {
  if (node === null) return 0;
  return 1 + Math.max(treeDepth(node.left), treeDepth(node.right));
}

const tree = buildTree(4, 1);

console.log("factorial(6) =", factorial(6));
console.log("fib(10) =", fib(10));
console.log("fib sequence =", Array.from({ length: 10 }, (_, i) => fib(i)).join(","));
console.log("treeSum =", treeSum(tree), "treeDepth =", treeDepth(tree));
