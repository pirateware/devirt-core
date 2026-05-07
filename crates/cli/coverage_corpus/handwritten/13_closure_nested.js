function outer(a) {
    function inner1(b) {
        function inner2(c) {
            return a + b + c;
        }
        return inner2;
    }
    return inner1;
}
console.log(outer(1)(2)(3));
console.log(outer(10)(20)(30));
