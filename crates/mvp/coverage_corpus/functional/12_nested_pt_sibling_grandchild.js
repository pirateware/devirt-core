function a() {
    let x = 1;
    function* b() { yield x; }
    function c() {
        function* d() { yield x; }
        return d;
    }
    let cgen = c();
    return [b().next().value, cgen().next().value];
}
