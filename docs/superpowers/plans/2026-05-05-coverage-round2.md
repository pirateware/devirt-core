# Coverage Round 2: Array/Object Literals + `new` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lower three additional JS expression forms — array literals, object literals, and `new` — so functions that use them stop falling through to PassThrough.

**Architecture:** Each form gets a new `tree::Expr` variant, an OXC→tree path in `lower.rs`, an acceptance arm in `disposition.rs`, a tree→SSA path in `lower_ssa.rs`, an SSA→tree-back path in `lower_tree.rs`, and a JS-emit path in `emit.rs`. Effects: AllocArray/AllocObject are PURE (already), Construct is OPAQUE (already). Element/argument expressions evaluate left-to-right and their effects are sequenced before the alloc, mirroring how Call args sequence. Round-trip aesthetic recovery (rebuilding `[1,2,3]` from the alloc+init chain in ssa-back) is **explicitly out of scope** for this session — desugared output is acceptable as long as it's behaviorally equivalent.

**Tech Stack:** Rust, OXC AST, the existing tree-IR / SSA pipeline in `crates/mvp/`.

---

## File Structure

| File | Change |
|---|---|
| `crates/mvp/src/ir/tree.rs` | Add `Expr::Array`, `Expr::Object`, `Expr::New`, `ObjectProp`, `ObjectKey`. Update `dump_expr`. |
| `crates/mvp/src/ir/free_vars.rs` | Walk new variants. |
| `crates/mvp/src/coverage.rs` | Walk new variants. |
| `crates/mvp/src/ir/disposition.rs` | Accept `ArrayExpression`, `ObjectExpression`, `NewExpression` with the spec's restrictions. |
| `crates/mvp/src/lower.rs` | Lower OXC `ArrayExpression` / `ObjectExpression` / `NewExpression` to the new tree-IR variants. |
| `crates/mvp/src/ir/lower_ssa.rs` | `convert_expr` arms for Array → AllocArray + StoreIndex chain, Object → AllocObject + StoreProp/StoreIndex chain (computed keys take StoreIndex), New → Construct. |
| `crates/mvp/src/emit.rs` | Render the three new variants as JS source. |
| `crates/mvp/coverage_corpus/handwritten/` | Re-use existing programs; no new files needed. |
| `crates/mvp/tests/functional_equivalence.rs` | Add 4 new tests: array_literal, object_literal_property_forms, new_expression_user_ctor, new_with_builtin, mixed_three_features. |
| `crates/mvp/src/ir/snapshots/` | New `insta` snapshots for unit IR tests. |
| `docs/COVERAGE_AUDIT.md` | Append "Round 2" section after re-measurement. |

---

## Task 1: Add `Expr::Array` plumbing (no lowering yet)

**Files:**
- Modify: `crates/mvp/src/ir/tree.rs`
- Modify: `crates/mvp/src/ir/free_vars.rs`
- Modify: `crates/mvp/src/coverage.rs`
- Modify: `crates/mvp/src/emit.rs`

- [ ] **Step 1.1: Add `Expr::Array` variant to tree-IR**

In `crates/mvp/src/ir/tree.rs`, extend the `Expr` enum:
```rust
pub enum Expr {
    // ... existing variants ...
    Array {
        elements: Vec<Expr>,
    },
}
```
And in `dump_expr`:
```rust
Expr::Array { elements } => format!(
    "[{}]",
    elements.iter().map(dump_expr).collect::<Vec<_>>().join(", ")
),
```

- [ ] **Step 1.2: Walk `Expr::Array` in free_vars**

In `crates/mvp/src/ir/free_vars.rs::walk_expr` add:
```rust
Expr::Array { elements } => {
    for e in elements { self.walk_expr(e); }
}
```

- [ ] **Step 1.3: Walk `Expr::Array` in coverage**

In `crates/mvp/src/coverage.rs::walk_expr` add:
```rust
Expr::Array { elements } => {
    for e in elements { walk_expr(e, parent, rows); }
}
```

- [ ] **Step 1.4: Emit `Expr::Array` as JS source**

In `crates/mvp/src/emit.rs::emit_expr` add:
```rust
Expr::Array { elements } => {
    self.buf.push('[');
    for (i, e) in elements.iter().enumerate() {
        if i > 0 { self.buf.push_str(", "); }
        self.emit_expr(e);
    }
    self.buf.push(']');
}
```

- [ ] **Step 1.5: Build & test**

Run: `cargo build -p mvp` then `cargo test -p mvp`
Expected: PASS (no behavior change yet — the new variant is unreachable).

- [ ] **Step 1.6: Commit**
```bash
git add crates/mvp/src/ir/tree.rs crates/mvp/src/ir/free_vars.rs crates/mvp/src/coverage.rs crates/mvp/src/emit.rs
git commit -m "ir: add Expr::Array variant + dump/emit/walkers"
```

---

## Task 2: Lower OXC `ArrayExpression` to `Expr::Array`

**Files:**
- Modify: `crates/mvp/src/lower.rs:701` (the catch-all `_ => Err(...)` arm)
- Modify: `crates/mvp/src/ir/disposition.rs:262` (the `_ => false` in `expr_lowerable`)

- [ ] **Step 2.1: Accept `ArrayExpression` in the disposition walker**

In `crates/mvp/src/ir/disposition.rs::expr_lowerable`, before the catch-all `_ => false`:
```rust
ArrayExpression(arr) => {
    for el in &arr.elements {
        match el {
            js::ArrayExpressionElement::SpreadElement(_) => return false,
            js::ArrayExpressionElement::Elision(_) => return false,
            other => match other.as_expression() {
                Some(e) => if !expr_lowerable(e) { return false; },
                None => return false,
            },
        }
    }
    true
}
```

- [ ] **Step 2.2: Lower OXC ArrayExpression in lower.rs**

In `crates/mvp/src/lower.rs::lower_expr`, before the final `_ => Err(...)` arm:
```rust
js::Expression::ArrayExpression(arr) => {
    let mut elements = Vec::with_capacity(arr.elements.len());
    for el in &arr.elements {
        match el {
            js::ArrayExpressionElement::SpreadElement(_) => {
                return Err(Error::Unsupported("spread in array literal"));
            }
            js::ArrayExpressionElement::Elision(_) => {
                return Err(Error::Unsupported("elision in array literal"));
            }
            other => {
                let e = other.as_expression()
                    .ok_or(Error::Unsupported("non-expression array element"))?;
                elements.push(self.lower_expr(e)?);
            }
        }
    }
    Ok(Expr::Array { elements })
}
```

- [ ] **Step 2.3: Build**

Run: `cargo build -p mvp`
Expected: builds — ssa lowering will hit `unimplemented` for `Expr::Array` on use, but that's fine because nothing reaches it yet (disposition routes them through, but ssa hasn't a match arm).

Note: this commit will leave a known panic if anyone actually lowers an array literal. The next task closes that gap. Don't run the full test suite here — proceed.

- [ ] **Step 2.4: Commit**
```bash
git add crates/mvp/src/lower.rs crates/mvp/src/ir/disposition.rs
git commit -m "lower: parse OXC ArrayExpression to tree::Expr::Array"
```

---

## Task 3: SSA-lower `Expr::Array` to `AllocArray` + `StoreIndex` chain

**Files:**
- Modify: `crates/mvp/src/ir/lower_ssa.rs:1620` (end of `convert_expr` match)
- Modify: `crates/mvp/src/ir/lower_tree.rs` — no change needed; existing `AllocArray` arm renders as `Array()` and `StoreIndex` as a separate `t[i]=v` assignment, which is behaviorally correct.
- Test: `crates/mvp/src/tests.rs` (or wherever `insta` snapshot tests live — adapt path; if there's no existing snapshot harness, skip the snapshot step and rely on the functional test in Task 7).

- [ ] **Step 3.1: Add a failing functional test for array literals**

Create file: `crates/mvp/coverage_corpus/handwritten/11_array_literal_returned.js` (only if a corresponding test runs the corpus; otherwise skip). The functional_equivalence test in Task 7 covers this — proceed there.

- [ ] **Step 3.2: SSA-lower `Expr::Array`**

In `crates/mvp/src/ir/lower_ssa.rs::convert_expr`, add a new arm before `Expr::Func`:
```rust
Expr::Array { elements } => {
    // Effects: element expressions sequence left-to-right (each
    // emit() is in-order); AllocArray itself is PURE — it's a
    // fresh allocation. We could allocate first and then store,
    // but that exposes an empty array between stores; the safer
    // pattern (matching evaluation order in JS) is to evaluate
    // all elements first, allocate, then store in order.
    let element_ops: Vec<Operand> = elements.iter()
        .map(|e| self.convert_expr(e)).collect();
    let arr = self.emit(InstKind::AllocArray {
        length: Some(elements.len() as u32),
    });
    for (i, op) in element_ops.into_iter().enumerate() {
        let idx = Operand::Const(Constant::Number(i as f64));
        self.builder.add_inst(InstKind::StoreIndex {
            obj: arr.clone(),
            index: idx,
            val: op,
            purity: Purity::Pure,
        });
    }
    arr
}
```

Note: `Purity::Pure` on the StoreIndex is sound because the target is a freshly allocated array with no escape between alloc and stores. The fresh-allocation argument is what makes the elision under DCE later correct.

- [ ] **Step 3.3: Build**
```bash
cargo build -p mvp
```
Expected: PASS.

- [ ] **Step 3.4: Run the existing test suite**
```bash
cargo test -p mvp
```
Expected: PASS — existing tests didn't use array literals; the new path is covered in Task 7.

- [ ] **Step 3.5: Commit**
```bash
git add crates/mvp/src/ir/lower_ssa.rs
git commit -m "ssa: lower Expr::Array to AllocArray + StoreIndex chain"
```

---

## Task 4: Add `Expr::Object` plumbing

**Files:**
- Modify: `crates/mvp/src/ir/tree.rs`
- Modify: `crates/mvp/src/ir/free_vars.rs`
- Modify: `crates/mvp/src/coverage.rs`
- Modify: `crates/mvp/src/emit.rs`

- [ ] **Step 4.1: Add `Expr::Object` types to tree-IR**

In `crates/mvp/src/ir/tree.rs`:
```rust
#[derive(Debug, Clone)]
pub enum ObjectKey {
    Static(String),       // identifier or string literal key
    Computed(Box<Expr>),  // [expr] : value
}

#[derive(Debug, Clone)]
pub struct ObjectProp {
    pub key: ObjectKey,
    pub value: Expr,
}
```
Then add to `Expr`:
```rust
Object {
    props: Vec<ObjectProp>,
},
```
And in `dump_expr`:
```rust
Expr::Object { props } => {
    let parts: Vec<String> = props.iter().map(|p| {
        let k = match &p.key {
            ObjectKey::Static(s) => s.clone(),
            ObjectKey::Computed(e) => format!("[{}]", dump_expr(e)),
        };
        format!("{k}: {}", dump_expr(&p.value))
    }).collect();
    format!("{{{}}}", parts.join(", "))
}
```

- [ ] **Step 4.2: Walk `Expr::Object` in free_vars and coverage**

In `crates/mvp/src/ir/free_vars.rs::walk_expr`:
```rust
Expr::Object { props } => {
    for p in props {
        if let ObjectKey::Computed(e) = &p.key { self.walk_expr(e); }
        self.walk_expr(&p.value);
    }
}
```
And the analogous arm in `crates/mvp/src/coverage.rs::walk_expr`.

- [ ] **Step 4.3: Emit `Expr::Object` as JS source**

In `crates/mvp/src/emit.rs::emit_expr`:
```rust
Expr::Object { props } => {
    self.buf.push('{');
    for (i, p) in props.iter().enumerate() {
        if i > 0 { self.buf.push_str(", "); }
        match &p.key {
            ObjectKey::Static(s) => {
                // Quote if not a valid JS identifier; for now, always
                // emit as a quoted string for safety.
                self.buf.push('"');
                self.buf.push_str(s);
                self.buf.push('"');
            }
            ObjectKey::Computed(e) => {
                self.buf.push('[');
                self.emit_expr(e);
                self.buf.push(']');
            }
        }
        self.buf.push_str(": ");
        self.emit_expr(&p.value);
    }
    self.buf.push('}');
}
```

- [ ] **Step 4.4: Build & test**
```bash
cargo build -p mvp && cargo test -p mvp
```
Expected: PASS.

- [ ] **Step 4.5: Commit**
```bash
git add crates/mvp/src/ir/tree.rs crates/mvp/src/ir/free_vars.rs crates/mvp/src/coverage.rs crates/mvp/src/emit.rs
git commit -m "ir: add Expr::Object variant + dump/emit/walkers"
```

---

## Task 5: Lower OXC `ObjectExpression` to `Expr::Object`

**Files:**
- Modify: `crates/mvp/src/ir/disposition.rs::expr_lowerable`
- Modify: `crates/mvp/src/lower.rs::lower_expr`
- Modify: `crates/mvp/src/ir/lower_ssa.rs::convert_expr`

- [ ] **Step 5.1: Accept `ObjectExpression` in disposition walker**

In `crates/mvp/src/ir/disposition.rs::expr_lowerable`:
```rust
ObjectExpression(obj) => {
    for prop in &obj.properties {
        match prop {
            js::ObjectPropertyKind::ObjectProperty(p) => {
                if p.method || p.kind != js::PropertyKind::Init {
                    return false; // methods, getters, setters out of scope
                }
                // shorthand: p.shorthand is true; the key is an Identifier
                // and the value is also an Identifier with same name.
                // Both lower fine via the regular paths below.
                if !expr_lowerable(&p.value) { return false; }
                if p.computed {
                    // computed key: PropertyKey is an Expression
                    if let Some(e) = p.key.as_expression() {
                        if !expr_lowerable(e) { return false; }
                    } else {
                        return false;
                    }
                } else {
                    // static key must be Identifier or StringLiteral or NumericLiteral
                    match &p.key {
                        js::PropertyKey::StaticIdentifier(_)
                        | js::PropertyKey::StringLiteral(_)
                        | js::PropertyKey::NumericLiteral(_) => {}
                        _ => return false,
                    }
                }
            }
            js::ObjectPropertyKind::SpreadProperty(_) => return false,
        }
    }
    true
}
```
(Adapt field names — verify against actual OXC types via `cargo doc` or local `grep`.)

- [ ] **Step 5.2: Lower OXC ObjectExpression in lower.rs**

In `crates/mvp/src/lower.rs::lower_expr`:
```rust
js::Expression::ObjectExpression(obj) => {
    let mut props = Vec::with_capacity(obj.properties.len());
    for prop in &obj.properties {
        match prop {
            js::ObjectPropertyKind::ObjectProperty(p) => {
                if p.method || p.kind != js::PropertyKind::Init {
                    return Err(Error::Unsupported("object method/getter/setter"));
                }
                let value = self.lower_expr(&p.value)?;
                let key = if p.computed {
                    let e = p.key.as_expression()
                        .ok_or(Error::Unsupported("non-expression computed key"))?;
                    ObjectKey::Computed(Box::new(self.lower_expr(e)?))
                } else {
                    match &p.key {
                        js::PropertyKey::StaticIdentifier(id) => {
                            ObjectKey::Static(id.name.to_string())
                        }
                        js::PropertyKey::StringLiteral(s) => {
                            ObjectKey::Static(s.value.to_string())
                        }
                        js::PropertyKey::NumericLiteral(n) => {
                            ObjectKey::Static(n.value.to_string())
                        }
                        _ => return Err(Error::Unsupported("object key kind")),
                    }
                };
                props.push(ObjectProp { key, value });
            }
            js::ObjectPropertyKind::SpreadProperty(_) => {
                return Err(Error::Unsupported("spread in object literal"));
            }
        }
    }
    Ok(Expr::Object { props })
}
```

- [ ] **Step 5.3: SSA-lower `Expr::Object`**

In `crates/mvp/src/ir/lower_ssa.rs::convert_expr`:
```rust
Expr::Object { props } => {
    // Per JS: each prop's key (if computed) and value is evaluated
    // left-to-right. Effects of element expressions sequence before
    // the alloc here as well — same argument as Expr::Array. We
    // emit alloc first then stores, since a partial empty object is
    // never visible (no observation between alloc and the stores).
    let evaluated: Vec<(Either<StrId, Operand>, Operand)> = props.iter().map(|p| {
        let v = self.convert_expr(&p.value);
        let k = match &p.key {
            ObjectKey::Static(s) => Either::Left(self.builder.intern(s)),
            ObjectKey::Computed(e) => Either::Right(self.convert_expr(e)),
        };
        (k, v)
    }).collect();
    let obj = self.emit(InstKind::AllocObject);
    for (k, v) in evaluated {
        match k {
            Either::Left(key) => {
                self.builder.add_inst(InstKind::StoreProp {
                    obj: obj.clone(), key, val: v, purity: Purity::Pure,
                });
            }
            Either::Right(idx) => {
                self.builder.add_inst(InstKind::StoreIndex {
                    obj: obj.clone(), index: idx, val: v, purity: Purity::Pure,
                });
            }
        }
    }
    obj
}
```

If `Either` isn't already in scope, replace with a small local enum or inline the match.

`Purity::Pure` is sound here: the object is freshly allocated and no observer can intercept the stores (no Proxy on a brand-new `{}`).

Note: the static-key evaluation order above evaluates value first then key; the JS spec evaluates key first then value. For static keys (string-interning) this is observably equivalent; for computed keys, intervene: evaluate key then value. Adjust:
```rust
let k = match &p.key {
    ObjectKey::Static(s) => Either::Left(self.builder.intern(s)),
    ObjectKey::Computed(e) => Either::Right(self.convert_expr(e)),
};
let v = self.convert_expr(&p.value);
```

- [ ] **Step 5.4: Build & test**
```bash
cargo build -p mvp && cargo test -p mvp
```
Expected: PASS.

- [ ] **Step 5.5: Commit**
```bash
git add crates/mvp/src/ir/disposition.rs crates/mvp/src/lower.rs crates/mvp/src/ir/lower_ssa.rs
git commit -m "lower+ssa: object literals — AllocObject + StoreProp/StoreIndex chain"
```

---

## Task 6: Lower OXC `NewExpression` to `Expr::New` → `Construct`

**Files:**
- Modify: `crates/mvp/src/ir/tree.rs`
- Modify: `crates/mvp/src/ir/free_vars.rs`
- Modify: `crates/mvp/src/coverage.rs`
- Modify: `crates/mvp/src/emit.rs`
- Modify: `crates/mvp/src/ir/disposition.rs`
- Modify: `crates/mvp/src/lower.rs`
- Modify: `crates/mvp/src/ir/lower_ssa.rs`
- Modify: `crates/mvp/src/ir/lower_tree.rs:611` (the existing `Construct` arm currently emits `ctor(args)` instead of `new ctor(args)` — that's a behavioral bug latent until now since nothing produced Construct; fix as part of this task).

- [ ] **Step 6.1: Add `Expr::New` to tree-IR**

In `tree.rs`:
```rust
New {
    callee: Box<Expr>,
    args: Vec<Expr>,
},
```
Add `dump_expr` arm:
```rust
Expr::New { callee, args } => format!(
    "new {}({})",
    dump_expr(callee),
    args.iter().map(dump_expr).collect::<Vec<_>>().join(", ")
),
```

- [ ] **Step 6.2: Walk `Expr::New` in free_vars + coverage**

In both walkers, mirror the `Expr::Call` arm.

- [ ] **Step 6.3: Emit `Expr::New` as JS source**

In `emit.rs`, mirror the `Expr::Call` arm but prefix with `new `:
```rust
Expr::New { callee, args } => {
    self.buf.push_str("new ");
    self.emit_expr(callee);
    self.buf.push('(');
    for (i, a) in args.iter().enumerate() {
        if i > 0 { self.buf.push_str(", "); }
        self.emit_expr(a);
    }
    self.buf.push(')');
}
```

- [ ] **Step 6.4: Accept `NewExpression` in disposition walker**

In `disposition.rs::expr_lowerable`:
```rust
NewExpression(n) => {
    if !expr_lowerable(&n.callee) { return false; }
    for a in &n.arguments {
        match a {
            js::Argument::SpreadElement(_) => return false,
            other => match other.as_expression() {
                Some(e) => if !expr_lowerable(e) { return false; },
                None => return false,
            },
        }
    }
    true
}
```

- [ ] **Step 6.5: Lower OXC NewExpression in lower.rs**

```rust
js::Expression::NewExpression(n) => {
    let callee = self.lower_expr(&n.callee)?;
    let mut args = Vec::with_capacity(n.arguments.len());
    for a in &n.arguments {
        match a {
            js::Argument::SpreadElement(_) => {
                return Err(Error::Unsupported("spread in new args"));
            }
            other => {
                let e = other.as_expression()
                    .ok_or(Error::Unsupported("non-expression new arg"))?;
                args.push(self.lower_expr(e)?);
            }
        }
    }
    Ok(Expr::New { callee: Box::new(callee), args })
}
```

- [ ] **Step 6.6: SSA-lower `Expr::New` to `Construct`**

In `lower_ssa.rs::convert_expr`:
```rust
Expr::New { callee, args } => {
    let ctor = self.convert_expr(callee);
    let arg_ops: Vec<Operand> = args.iter().map(|a| self.convert_expr(a)).collect();
    self.emit(InstKind::Construct { ctor, args: arg_ops })
}
```

- [ ] **Step 6.7: Fix `Construct` round-trip in lower_tree.rs**

In `crates/mvp/src/ir/lower_tree.rs` change the existing `Construct` arm (around line 611):
```rust
Construct { ctor, args } => Expr::New {
    callee: Box::new(self.operand_to_expr(ctor)),
    args: args.iter().map(|a| self.operand_to_expr(a)).collect(),
},
```

- [ ] **Step 6.8: Build & test**
```bash
cargo build -p mvp && cargo test -p mvp
```
Expected: PASS.

- [ ] **Step 6.9: Commit**
```bash
git add crates/mvp/src/ir/tree.rs crates/mvp/src/ir/free_vars.rs crates/mvp/src/coverage.rs \
        crates/mvp/src/emit.rs crates/mvp/src/ir/disposition.rs crates/mvp/src/lower.rs \
        crates/mvp/src/ir/lower_ssa.rs crates/mvp/src/ir/lower_tree.rs
git commit -m "lower+ssa: new expression — Construct + emit-as-new round-trip fix"
```

---

## Task 7: Functional equivalence tests

**Files:**
- Modify: `crates/mvp/tests/functional_equivalence.rs`

- [ ] **Step 7.1: Skim the test file to find the "add a new test" pattern**

Run: `head -150 crates/mvp/tests/functional_equivalence.rs` and examine the existing test macros / functions to learn the harness shape. Reuse exactly. Don't invent a new pattern.

- [ ] **Step 7.2: Add 5 functional tests**

Append these in the same style as the existing tests:

**Test A — array_literal_returned:**
```js
function make() { return [1, 2, 3]; }
console.log(make());
console.log(make().length);
```
Expected output: `[ 1, 2, 3 ]` then `3` (or whatever node prints — the harness's existing comparator handles this).

**Test B — nested_array_literal:**
```js
function make() { return [[1,2],[3,4]]; }
var x = make();
console.log(x[0][1] + x[1][0]);
```

**Test C — object_literal_property_forms:**
```js
function build(x) {
  var k = 'dynamic';
  return { a: 1, "b": 2, 3: 'three', [k]: 'd', x };
}
var o = build('shorthand');
console.log(o.a, o.b, o[3], o.dynamic, o.x);
```

**Test D — new_expression_user_ctor:**
```js
function Foo(v) { this.v = v; }
var f = new Foo(42);
console.log(f.v);
```

**Test E — new_with_builtin:**
```js
var d = new Date(2026, 0, 1);
console.log(d.getFullYear());
var a = new Array(3);
console.log(a.length);
```

**Test F — mixed_three_features (the spec's combined test):**
```js
function make() {
  var pairs = [{k: 1, v: 'a'}, {k: 2, v: 'b'}];
  var d = new Date();
  return pairs;
}
var p = make();
console.log(p[0].k, p[1].v);
```

- [ ] **Step 7.3: Run the new tests**
```bash
cargo test -p mvp --test functional_equivalence
```
Expected: all PASS.

If `Test C`'s shorthand property `x` fails because the disposition walker rejects shorthand: add shorthand handling — OXC sets `p.shorthand = true` and stores the same Identifier as both key and value, which the lowering above already handles correctly. Verify by tracing `lower.rs::lower_expr` for the shorthand prop.

- [ ] **Step 7.4: Commit**
```bash
git add crates/mvp/tests/functional_equivalence.rs
git commit -m "tests: functional equivalence for array/object literals + new"
```

---

## Task 8: Re-measure coverage and update audit doc

**Files:**
- Modify: `docs/COVERAGE_AUDIT.md`

- [ ] **Step 8.1: Re-run coverage stats on the handwritten corpus**

Run:
```bash
for f in crates/mvp/coverage_corpus/handwritten/*.js; do
    echo "=== $f ==="
    cargo run -p mvp -q -- --coverage-stats "$f"
done > /tmp/round2_handwritten.txt 2>&1
cat /tmp/round2_handwritten.txt
```

- [ ] **Step 8.2: Re-run on the functional corpus**

Run:
```bash
for f in crates/mvp/coverage_corpus/functional/*.js; do
    echo "=== $f ==="
    cargo run -p mvp -q -- --coverage-stats "$f"
done > /tmp/round2_functional.txt 2>&1
```

- [ ] **Step 8.3: Append "Round 2" section to docs/COVERAGE_AUDIT.md**

Append a new top-level section after the existing content:

```markdown
## Round 2 (2026-05-05): array/object/new lowered

After implementing array literals, object literals, and `new`:

### Handwritten corpus (after)

| program | lowered fns | PT fns | top-level PT stmts | PT share |
|---|---:|---:|---:|---:|
[fill from /tmp/round2_handwritten.txt]

**Handwritten average passthrough share: ~XX%** (was 58%).

### Functional_equivalence corpus (after)

[fill from /tmp/round2_functional.txt]

### Verdict

[Three bullets per the spec's decision rule:
- if <25%: recommend Milestone 4
- if >35%: recommend items 4–6 first
- if 25–35%: surface and ask the user]
```

- [ ] **Step 8.4: Commit**
```bash
git add docs/COVERAGE_AUDIT.md
git commit -m "docs: coverage audit round 2 — array/object/new"
```

- [ ] **Step 8.5: STOP. Report to the user.**

Report:
1. The new passthrough share for the handwritten corpus.
2. Which decision branch from the spec triggered.
3. Wait for user direction. Do NOT proceed to Milestone 4 or items 4–6.

---

## Self-Review Checklist

- All three forms covered: ✓ array (Tasks 1–3), object (Tasks 4–5), new (Task 6).
- Effects discipline: AllocArray/AllocObject left as PURE; StoreIndex/StoreProp on freshly-alloc'd targets uses `Purity::Pure` per the no-escape argument; Construct stays OPAQUE per IR's existing classification. Member-access for read isn't touched (already works); member-access for write isn't extended (out of scope per spec).
- Spread, methods, getters/setters, computed-key methods, elision: each blocked at the disposition walker AND returns `Error::Unsupported` from lower.rs as a defense-in-depth check.
- Round-trip aesthetic recovery is deliberately deferred — the `Construct` round-trip fix in Step 6.7 is the only ssa-back change; arrays and objects round-trip as the desugared alloc+store sequence, which is behaviorally equivalent (functional tests verify).
- The `new` round-trip bug (Construct → bare call) is fixed inline in Step 6.7 because it would otherwise silently miscompile. This is in scope because we're now actually producing `Construct` instructions.
