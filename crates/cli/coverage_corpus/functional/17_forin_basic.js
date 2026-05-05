// for-in walks the prototype chain — `child` enumerates both its own
// `b` and the inherited `a`. Round-trip preserves the loop
// syntactically so the JS engine handles the iteration order.
function basic_forin() {
    var parent = { a: 1 };
    var child = Object.create(parent);
    child.b = 2;
    var keys = [];
    for (var k in child) {
        keys.push(k);
    }
    return keys.sort().join(',');
}
