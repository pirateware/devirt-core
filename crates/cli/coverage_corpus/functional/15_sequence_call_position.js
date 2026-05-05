// Bundler-interop: `(0, obj.method)()` strips the receiver so `this`
// is undefined inside the call. A naive lowering that collapses
// the sequence to `obj.method()` silently rebinds `this` to obj and
// breaks CommonJS interop. This fixture is the regression test that
// catches such a collapse.
function check_this_binding() {
    var log = [];
    var obj = {
        v: 'OBJ',
        method: function() {
            log.push(typeof this === 'undefined' ? 'undef' : this.v);
        }
    };
    (0, obj.method)();   // expected: log 'undef' (this === undefined in non-strict global, here it is the global object — but the receiver was stripped)
    obj.method();        // expected: log 'OBJ'
    return log.join(',');
}
