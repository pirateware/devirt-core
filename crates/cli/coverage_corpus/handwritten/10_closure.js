function makeCounter() {
    let n = 0;
    return function() {
        n = n + 1;
        return n;
    };
}
var c = makeCounter();
console.log(c());
console.log(c());
