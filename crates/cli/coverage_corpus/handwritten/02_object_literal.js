function describe(user) {
    var obj = { name: user, kind: "guest" };
    return obj.name + ":" + obj.kind;
}
console.log(describe("alice"));
