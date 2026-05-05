function makeWidget(label) {
    class Widget {
        constructor(l) { this.l = l; }
        describe() { return "widget:" + this.l; }
    }
    return new Widget(label).describe();
}
