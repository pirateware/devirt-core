function a() {
    let x = 1;
    function b() {
        function* c() { yield x; }
        return c;
    }
    return b;
}
