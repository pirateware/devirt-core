function outer() {
    let y = 10;
    let f = function (x) { return x + y; };
    y = 20;
    return f(1);
}
