var result = (function() {
    var nums = [1, 2, 3];
    var sum = 0;
    for (var i = 0; i < nums.length; i++) {
        sum += nums[i];
    }
    return sum;
})();
console.log(result);
