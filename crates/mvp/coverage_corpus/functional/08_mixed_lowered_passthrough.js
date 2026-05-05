function add(a, b) { return a + b; }
function listify() {
    class Bag {
        constructor() { this.xs = []; }
        push(v) { this.xs.push(v); return this; }
        join(sep) { return this.xs.join(sep); }
    }
    return new Bag().push(1).push(2).push(3).join("-");
}
