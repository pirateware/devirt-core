// Linked list + stack + queue implementations.
class Node {
  constructor(value) {
    this.value = value;
    this.next = null;
  }
}

class LinkedList {
  constructor() {
    this.head = null;
    this.size = 0;
  }
  push(value) {
    const node = new Node(value);
    if (!this.head) {
      this.head = node;
    } else {
      let cur = this.head;
      while (cur.next) cur = cur.next;
      cur.next = node;
    }
    this.size++;
    return this;
  }
  reverse() {
    let prev = null;
    let cur = this.head;
    while (cur) {
      const next = cur.next;
      cur.next = prev;
      prev = cur;
      cur = next;
    }
    this.head = prev;
    return this;
  }
  toArray() {
    const out = [];
    let cur = this.head;
    while (cur) {
      out.push(cur.value);
      cur = cur.next;
    }
    return out;
  }
}

const list = new LinkedList();
[1, 2, 3, 4, 5].forEach((v) => list.push(v));
console.log("list =", list.toArray().join(","));
list.reverse();
console.log("reversed =", list.toArray().join(","), "size =", list.size);

const stack = [];
[10, 20, 30].forEach((v) => stack.push(v));
const popped = [stack.pop(), stack.pop()];
console.log("popped =", popped.join(","), "remaining =", stack.join(","));

const queue = [];
[100, 200, 300].forEach((v) => queue.push(v));
const dequeued = [queue.shift(), queue.shift()];
console.log("dequeued =", dequeued.join(","), "remaining =", queue.join(","));
