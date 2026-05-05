function outer(label) {
    function inner() {
        class Box { describe() { return label; } }
        return new Box().describe();
    }
    return inner();
}
