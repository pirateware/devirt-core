// ES6 classes with inheritance, methods, and super calls.
class Shape {
  constructor(name) {
    this.name = name;
  }
  area() {
    return 0;
  }
  describe() {
    return `${this.name} with area ${this.area().toFixed(2)}`;
  }
}

class Rectangle extends Shape {
  constructor(w, h) {
    super("rectangle");
    this.w = w;
    this.h = h;
  }
  area() {
    return this.w * this.h;
  }
}

class Square extends Rectangle {
  constructor(side) {
    super(side, side);
    this.name = "square";
  }
}

class Circle extends Shape {
  constructor(r) {
    super("circle");
    this.r = r;
  }
  area() {
    return 3.14159 * this.r * this.r;
  }
}

const shapes = [new Rectangle(3, 4), new Square(5), new Circle(2)];
for (const s of shapes) {
  console.log(s.describe());
}
const totalArea = shapes.reduce((sum, s) => sum + s.area(), 0);
console.log("total area =", totalArea.toFixed(2));
