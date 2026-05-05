function safe(x) {
    try {
        return x.foo.bar;
    } catch (e) {
        return "caught";
    }
}
