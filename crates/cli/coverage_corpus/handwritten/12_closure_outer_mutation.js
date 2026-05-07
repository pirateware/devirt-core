function withReassign() {
    var y = 10;
    var f = function (x) { return x + y; };
    y = 20;
    return f(1);
}
console.log(withReassign());
