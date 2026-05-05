// Plain `=` in expression position (assignment used as a value).
// Each assignment yields the RHS; the loop condition observes the
// assigned value.
function read_until_zero(seq) {
    var i = 0;
    var x;
    var log = [];
    while ((x = seq[i])) {
        log.push(x);
        i = i + 1;
    }
    return log.join(',');
}
