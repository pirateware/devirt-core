function collect(n) {
    function* gen() {
        let i = 0;
        while (i < n) { yield i; i++; }
    }
    const out = [];
    for (const v of gen()) out.push(v);
    return out.join(",");
}
