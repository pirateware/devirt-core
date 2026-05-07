function adder(x) {
    return function (y) { return x + y; };
}
var add5 = adder(5);
console.log(add5(3));
console.log(add5(10));
console.log(adder(100)(7));
