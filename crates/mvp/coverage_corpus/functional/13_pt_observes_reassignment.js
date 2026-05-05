function a() {
    let x = 1;
    x = 2;
    function* b() { yield x; }
    return b;
}
