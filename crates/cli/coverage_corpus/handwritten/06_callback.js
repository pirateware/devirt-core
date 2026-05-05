function each(xs, cb) {
    let i = 0;
    while (i < xs.length) {
        cb(xs[i]);
        i = i + 1;
    }
}
each([10, 20, 30], function(v) {
    console.log(v);
});
