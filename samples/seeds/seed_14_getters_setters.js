// Getters/setters via Object.defineProperty and class accessors.
function makeTemperature(celsius) {
  const obj = { _c: celsius };
  Object.defineProperty(obj, "celsius", {
    get() {
      return this._c;
    },
    set(v) {
      this._c = v;
    },
    enumerable: true,
  });
  Object.defineProperty(obj, "fahrenheit", {
    get() {
      return this._c * 9 / 5 + 32;
    },
    set(f) {
      this._c = (f - 32) * 5 / 9;
    },
    enumerable: true,
  });
  return obj;
}

class Rect {
  constructor(w, h) {
    this._w = w;
    this._h = h;
  }
  get area() {
    return this._w * this._h;
  }
  set width(v) {
    this._w = v;
  }
  get width() {
    return this._w;
  }
}

const t = makeTemperature(25);
console.log("25C =", t.fahrenheit, "F");
t.fahrenheit = 212;
console.log("212F =", t.celsius, "C");

const r = new Rect(4, 5);
console.log("area =", r.area);
r.width = 10;
console.log("new area =", r.area, "width =", r.width);
