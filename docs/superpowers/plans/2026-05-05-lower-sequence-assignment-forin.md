# Lower SequenceExpr, AssignmentExpr, ForInStmt — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lower `SequenceExpr`, `AssignmentExpr`, and `ForInStmt` so they no longer appear in any function's blocker set. Predicted to unlock ≥220 of the 305 currently-passthrough corpus functions.

**Architecture:** Preserve all three constructs as first-class IR nodes (option (a) from the ticket, applied uniformly). Add `Expr::Sequence`, `Expr::Assign`, and `Stmt::ForIn` to the tree-IR. The disposition walker stops flagging them as blockers. Lowering, emit, free-var analysis, and pt-free-var analysis handle the new variants. SSA handles `Expr::Sequence` and `Expr::Assign` as ordinary side-effecting expressions; `Stmt::ForIn` cannot be expressed in SSA's iterator-protocol shape, so functions containing it are routed to PassThrough at the SSA boundary by attaching a verbatim source slice to `IrFunction` (mirroring the same fallback we add for `Stmt::Try`, fixing a latent panic at lower_ssa.rs:1697).

**Tech Stack:** Rust workspace; OXC parser; existing tree-IR / SSA-IR / emit pipeline.

**Order of work:** SequenceExpr → AssignmentExpr → ForInStmt. Each construct lands and runs the corpus before the next. Single feature branch, single PR at the end.

---

## Design decisions (locked before coding)

### Why preservation, not decomposition

For all three constructs the ticket recommends preservation. The shared rationale:

- **No statement-hoisting infrastructure exists.** The current AST→tree-IR lowering returns a single `Stmt` per source statement; there is no machinery to emit "extra preceding statements" out of an expression-position lowering. Building that infrastructure for `AssignmentExpr` decomposition is a large piece of work that adds a new invariant for every existing expression lowering to respect. Preserving as `Expr::Assign` sidesteps it entirely.
- **Setter-LHS spec subtlety dissolves.** `(obj.x = expr)` as an expression yields the RHS value, not the post-set property value (ECMA-262 13.15). When we render `Expr::Assign` syntactically as `target = value`, the JS engine handles this for us. A decomposition into temp + assign + use-of-target would need to know whether the LHS has a setter, which is not statically decidable.
- **`this`-binding for `(0, foo.bar)(args)` is preserved automatically.** The SSA layer's `Expr::Call { callee: Expr::Member, ... }` arm uses `CallMethod` (binding `this` to the receiver). When the callee is `Expr::Sequence { exprs: [_, Expr::Member { ... }] }`, the arm does **not** match — the call falls through to plain `Call` with `this === undefined`. This is exactly the bundler-interop semantics the ticket warns about. Preservation gets it right by construction; decomposition would have to recognize call-position SequenceExpr explicitly.
- **For-in's enumeration semantics are too gnarly to reimplement.** Walking the prototype chain, honoring shadowing-by-non-enumerable, and matching spec iteration order would require a runtime helper with non-trivial edge cases. Preserving `Stmt::ForIn` as syntax delegates this to the host JS engine.

### Why `Stmt::ForIn` falls back to PassThrough at SSA

`Stmt::ForOf` desugars to the iterator protocol in SSA. There is no equivalent for-in protocol in JS — for-in is a primitive. Lowering it to `Object.keys()` would lose prototype iteration; lowering it to a hand-rolled prototype walk is the runtime helper the ticket warns against.

We therefore mirror the precedent for `Stmt::Try`: tree-IR carries the construct, but functions containing it are routed to PassThrough at the SSA boundary using a verbatim source slice. This loses SSA-level optimization on those bodies but preserves semantics perfectly. The disposition walker still classifies the function as `Lowered` (no blocker reported), so the corpus metric counts the unlock; the actual SSA artifact is a PassThrough function.

To make this work we need `IrFunction` to carry an optional source slice (it currently does not — only `FuncLiteral` does, via `passthrough_source`). Adding the slice also fixes the latent panic for `Stmt::Try` at `lower_ssa.rs:1697`.

### What stays out of scope

- `CompoundAssignmentExpr` (`+=`, `-=`, etc.). Its own variant; separate ticket.
- `LogicalAssignOp` (`&&=`, `||=`, `??=`). Already its own variant; zero corpus occurrences.
- Destructuring assignment / destructuring binding. Gated by a different blocker.
- `BreakStmt`, `SwitchStmt`, `ThisExpr`, etc. — separate work per ticket.

---

## File map

Files modified by this plan, with the responsibility each takes on:

- **`crates/ir/src/tree.rs`** — Add `Expr::Sequence`, `Expr::Assign`, `Stmt::ForIn`. Add `IrFunction.source: Option<String>`. Extend `dump_stmt` and `dump_expr`.
- **`crates/parse/src/disposition.rs`** — Stop inserting `BlockerKind::SequenceExpr`, `BlockerKind::AssignmentExpr`, `BlockerKind::ForInStmt`; descend into children to surface other blockers.
- **`crates/parse/src/lib.rs`** — Lower `js::Expression::SequenceExpression` to `Expr::Sequence`; lower `js::Expression::AssignmentExpression` (in expression position, plain `=` only) to `Expr::Assign`; lower `js::Statement::ForInStatement` to `Stmt::ForIn`. Capture the body source slice for every Lowered top-level function and inner literal so SSA can fall back. Remove the `BlockerKind::SequenceExpr`/`AssignmentExpr` debug arms in the cross-check (no longer reachable).
- **`crates/parse/src/lower_ssa.rs`** — `convert_expr` arms for `Expr::Sequence` and `Expr::Assign`. Detect `Stmt::ForIn` (and pre-existing `Stmt::Try`) in the body up-front and route the function to PassThrough using `IrFunction.source`. Remove the `Stmt::Try` panic at line 1697.
- **`crates/parse/src/lower_tree.rs`** — `inst_kind_to_expr` / statement walking pieces that materialize SSA back to tree-IR continue to produce `Expr::Sequence`-free output (no SSA construct generates Sequence; nothing to do beyond not regressing). For `Expr::Assign` produced from new SSA arms, ensure roundtrip stability.
- **`crates/emit/src/lib.rs`** — Render `Expr::Sequence`, `Expr::Assign`, `Stmt::ForIn` as JS source.
- **`crates/ir/src/free_vars.rs`** — Walk arms for `Expr::Sequence`, `Expr::Assign`, `Stmt::ForIn`.
- **`crates/ir/src/pt_free_vars.rs`** — Already handles `js::*` AST nodes (for PT bodies). No change required if we don't change the AST-level walk; verify.
- **`crates/cli/src/coverage.rs`** — Stmt/Expr counters for the new variants. Verify no panic on unfamiliar variants.
- **Tests** — Add per-construct unit tests in `crates/parse/src/lower_ssa.rs` and `crates/emit/src/lib.rs`. Add functional fixtures in `crates/cli/coverage_corpus/functional/` covering the ticket's required semantics tests. Update corpus stability report.

---

## Task 1: Tree-IR additions for SequenceExpr

**Files:**
- Modify: `crates/ir/src/tree.rs`

- [ ] **Step 1: Add `Expr::Sequence` variant**

In `tree.rs`, inside `pub enum Expr`, add:

```rust
    /// Comma operator: `(a, b, c)`. Evaluates each operand left-to-right
    /// (side-effects of all but the last are discarded); the value of
    /// the expression is the last operand. Preserved as syntax through
    /// the IR rather than decomposed into a sequence of statements,
    /// because doing so collapses the bundler-interop pattern
    /// `(0, obj.method)(args)` (which strips the receiver and binds
    /// `this` to undefined) into a plain method call. See
    /// docs/superpowers/plans/2026-05-05-lower-sequence-assignment-forin.md
    /// for the rationale.
    Sequence {
        exprs: Vec<Expr>,
    },
```

- [ ] **Step 2: Extend `dump_expr` for Sequence**

In the `match e` block of `dump_expr`, add an arm before the `Expr::Update` arm:

```rust
        Expr::Sequence { exprs } => format!(
            "({})",
            exprs.iter().map(dump_expr).collect::<Vec<_>>().join(", ")
        ),
```

- [ ] **Step 3: Build**

Run: `cargo build -q`
Expected: clean build (some warnings may move).

- [ ] **Step 4: Commit**

```bash
git add crates/ir/src/tree.rs
git commit -m "ir(tree): add Expr::Sequence variant"
```

---

## Task 2: Lower JS SequenceExpression to Expr::Sequence

**Files:**
- Modify: `crates/parse/src/lib.rs:884` (`lower_expr`)
- Modify: `crates/parse/src/disposition.rs:528` (sequence arm)

- [ ] **Step 1: Add the lowering arm**

In `lower_expr` in `lib.rs`, add an arm before the catch-all `_ => Err(...)`:

```rust
            js::Expression::SequenceExpression(s) => {
                let mut exprs = Vec::with_capacity(s.expressions.len());
                for e in &s.expressions {
                    exprs.push(self.lower_expr(e)?);
                }
                Ok(Expr::Sequence { exprs })
            }
```

- [ ] **Step 2: Stop reporting SequenceExpr as a blocker**

In `disposition.rs`, replace the `SequenceExpression(s)` arm:

```rust
        SequenceExpression(s) => {
            for e in &s.expressions {
                collect_expr_blockers(e, blockers);
            }
        }
```

(Removes the `blockers.insert(BlockerKind::SequenceExpr)` line; still walks children.)

- [ ] **Step 3: Build & smoke-test**

Run: `cargo build -q`
Expected: clean build.

- [ ] **Step 4: Add a unit test for parse → tree-IR**

In `crates/parse/src/lib.rs` (in `mod tests` if present, or at the end of the file), add:

```rust
#[cfg(test)]
mod sequence_lowering_tests {
    use super::*;

    #[test]
    fn sequence_in_for_init_lowers() {
        let src = "function f(n) { for (let i = (0, n); i > 0; i--) {} }";
        let m = parse_and_lower(src).expect("must lower");
        // Should not contain a PassThrough function.
        for item in &m.items {
            assert!(matches!(item, Item::Function(_)), "expected Lowered, got {:?}", item);
        }
    }

    #[test]
    fn sequence_in_call_callee_lowers() {
        let src = "function f(o) { return (0, o.m)(); }";
        let m = parse_and_lower(src).expect("must lower");
        for item in &m.items {
            assert!(matches!(item, Item::Function(_)), "expected Lowered, got {:?}", item);
        }
    }
}
```

(Adapt the imports / module path if a `tests` mod with the same fixture pattern exists.)

- [ ] **Step 5: Run unit tests**

Run: `cargo test -q -p parse sequence_lowering`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/parse/src/lib.rs crates/parse/src/disposition.rs
git commit -m "parse: lower SequenceExpression to Expr::Sequence; remove SequenceExpr blocker"
```

---

## Task 3: SSA conversion for Expr::Sequence

**Files:**
- Modify: `crates/parse/src/lower_ssa.rs` — `convert_expr` and any walker that traverses `Expr` (e.g., `walk_reassigned_in_expr`, `discover_expr`, the auxiliary expression walkers).

- [ ] **Step 1: Add `convert_expr` arm for Sequence**

Locate `convert_expr` (in the `Converter` impl). Add an arm before the catch-all:

```rust
            Expr::Sequence { exprs } => {
                // Comma operator: evaluate each operand left-to-right for
                // side effects; the value of the expression is the last
                // operand. SSA simply walks the operands and discards all
                // but the final value.
                debug_assert!(!exprs.is_empty(), "Expr::Sequence must be non-empty");
                let mut last = None;
                for e in exprs {
                    last = Some(self.convert_expr(e));
                }
                last.expect("non-empty by debug_assert above")
            }
```

- [ ] **Step 2: Update auxiliary expression walkers**

In `lower_ssa.rs`, every `match expr` over `&Expr` that doesn't have a wildcard catch-all needs a `Sequence` arm. Search for `match expr {` and `match e {` in the file. The expected callsites: `walk_reassigned_in_expr`, `discover_expr`, plus any other helpers. For each, add:

```rust
        Expr::Sequence { exprs } => {
            for e in exprs {
                <recurse using the same fn>(e, ...);
            }
        }
```

(Use the same recursion helper the surrounding arms use. If the function uses a wildcard `_ => {}` arm, no change is needed there.)

- [ ] **Step 3: Same in `crates/ir/src/free_vars.rs`**

In `walk_expr`, add a `Sequence` arm:

```rust
            Expr::Sequence { exprs } => {
                for e in exprs {
                    self.walk_expr(e);
                }
            }
```

- [ ] **Step 4: Build**

Run: `cargo build -q`
Expected: clean build (no missing-arm warnings; the matches must be exhaustive).

- [ ] **Step 5: Run all tests**

Run: `cargo test -q`
Expected: all existing tests still PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/parse/src/lower_ssa.rs crates/ir/src/free_vars.rs
git commit -m "ssa(sequence): convert Expr::Sequence by walking operands; thread through walkers"
```

---

## Task 4: Emit and lower_tree for Expr::Sequence

**Files:**
- Modify: `crates/emit/src/lib.rs` — `emit_expr`
- Modify: `crates/parse/src/lower_tree.rs` — `render_expr_to_string`

- [ ] **Step 1: Emitter arm for Sequence**

In `crates/emit/src/lib.rs`, locate the function that renders `Expr` (around line 516+ where `Expr::Member`, `Expr::Index` etc. live). Add a `Sequence` arm:

```rust
            Expr::Sequence { exprs } => {
                self.out.push('(');
                for (i, e) in exprs.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.emit_expr(e);
                }
                self.out.push(')');
            }
```

(Adjust to match the existing emitter's API — it may use a `String` or a writer method instead of `self.out`.)

- [ ] **Step 2: lower_tree.rs — render_expr_to_string**

In `crates/parse/src/lower_tree.rs:1568` (`fn render_expr_to_string`), add an arm to mirror the emitter behavior so SSA → tree → emit roundtrip works for Sequence.

```rust
        Expr::Sequence { exprs } => {
            let parts: Vec<String> = exprs.iter().map(render_expr_to_string).collect();
            format!("({})", parts.join(", "))
        }
```

- [ ] **Step 3: Build and run tests**

Run: `cargo test -q`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/emit/src/lib.rs crates/parse/src/lower_tree.rs
git commit -m "emit: render Expr::Sequence as (a, b, c)"
```

---

## Task 5: SequenceExpr semantic test fixtures

**Files:**
- Create: `crates/cli/coverage_corpus/functional/15_sequence_call_position.js`
- Create: `crates/cli/coverage_corpus/functional/16_sequence_for_init.js`
- Create: `crates/cli/coverage_corpus/functional/17_sequence_eval_order.js`

- [ ] **Step 1: Write 15_sequence_call_position.js** (the bundler-interop `this`-binding test)

```js
function check_this_binding() {
    let log = [];
    let obj = {
        v: 'OBJ',
        method: function() { log.push(this && this.v); }
    };
    // (0, obj.method)() must call with this === undefined; log records undefined.
    (0, obj.method)();
    // obj.method() calls with this === obj.
    obj.method();
    return log.join(',');
}
```

- [ ] **Step 2: Write 16_sequence_for_init.js**

```js
function for_init_sequence(n) {
    let total = 0;
    for (let i = 0, j = n; i < j; i = i + 1, j = j - 1) {
        total = total + i + j;
    }
    return total;
}
```

- [ ] **Step 3: Write 17_sequence_eval_order.js**

```js
function eval_order() {
    let log = [];
    function f(x) { log.push(x); return x; }
    let last = (f(1), f(2), f(3));
    return log.join(',') + '|' + last;
}
```

- [ ] **Step 4: Run via the CLI to verify they Lower and roundtrip**

```bash
cargo run -q --bin cli -- crates/cli/coverage_corpus/functional/15_sequence_call_position.js
cargo run -q --bin cli -- crates/cli/coverage_corpus/functional/16_sequence_for_init.js
cargo run -q --bin cli -- crates/cli/coverage_corpus/functional/17_sequence_eval_order.js
```

Expected: each prints `===== TREE-IR =====` showing a Lowered function (no `passthrough` marker), then emitted JS at the end without panic. Manually verify the emitted code preserves `(0, obj.method)()` (parens around the sequence; not collapsed to `obj.method()`).

- [ ] **Step 5: Behavioral roundtrip with node**

If the existing test infrastructure has a behavior-equivalence helper (`assert_behavior_equivalent` in `crates/parse/src/lower_tree.rs:2010`), add a Rust test that drives those three fixtures through it. Otherwise run them by hand:

```bash
node crates/cli/coverage_corpus/functional/15_sequence_call_position.js  # baseline
# (and confirm the deobfuscator output matches when piped through node)
```

Expected for fixture 15: the call-position invocation logs `undefined`, the direct invocation logs `OBJ`. If the deobfuscator's emitted output collapses `(0, obj.method)` to `obj.method`, fixture 15 fails behaviorally — that is the regression the ticket warns about.

- [ ] **Step 6: Commit**

```bash
git add crates/cli/coverage_corpus/functional/15_sequence_call_position.js \
        crates/cli/coverage_corpus/functional/16_sequence_for_init.js \
        crates/cli/coverage_corpus/functional/17_sequence_eval_order.js
git commit -m "test(corpus): SequenceExpr functional fixtures (call position, for-init, eval order)"
```

---

## Task 6: Run the corpus and validate SequenceExpr unlock

- [ ] **Step 1: Run the stability test runner**

The repo has a corpus-roundtrip test setup in `corpus/realworld`. Find the test harness that produces `STABILITY_REPORT.md`:

```bash
find . -path ./target -prune -o -name "*.rs" -print 2>/dev/null | xargs grep -l "STABILITY_REPORT\|roundtrip" | head
```

Identify the runner (likely a binary in `crates/cli` or a test). Run it:

```bash
cargo test -q --release --workspace
# or whatever the existing invocation is
```

- [ ] **Step 2: Verify roundtrip stays at 308/308**

Open the regenerated `corpus/realworld/STABILITY_REPORT.md`. Confirm:
- Total files: 308
- Roundtrip success: 308/308
- No panics

If roundtrip drops, stop and investigate before moving on.

- [ ] **Step 3: Verify SequenceExpr unlock count**

In the regenerated blocker analysis table, confirm:
- `SequenceExpr` row no longer appears (or shows 0 occurrences).
- Total passthrough function count drops by approximately 52 (the ticket's counterfactual prediction).

Record the actual count in the commit message in the next step.

- [ ] **Step 4: Commit corpus update**

```bash
git add corpus/realworld/STABILITY_REPORT.md
git commit -m "corpus: regen after SequenceExpr lowering (unlocks N functions)"
```

(Substitute N with the actual delta.)

---

## Task 7: Tree-IR additions for AssignmentExpr

**Files:**
- Modify: `crates/ir/src/tree.rs`

- [ ] **Step 1: Add `Expr::Assign`**

```rust
    /// Plain `=` assignment used as an expression: `if (x = foo()) {}`,
    /// `a = b = c`, `arr[i++] = expr`. Statement-level `=` continues to
    /// lower to `Stmt::Assign`; this variant is reserved for assignments
    /// in expression position. The value of the expression is the RHS
    /// (per ECMA-262 13.15) — preserved by rendering syntactically as
    /// `target = value` so the JS engine handles spec semantics.
    /// Compound (`+=` etc.) and logical (`||=` etc.) operators are NOT
    /// represented here; they remain blocked.
    Assign {
        target: Box<Expr>,
        value: Box<Expr>,
    },
```

- [ ] **Step 2: Extend `dump_expr`**

```rust
        Expr::Assign { target, value } => {
            format!("({} = {})", dump_expr(target), dump_expr(value))
        }
```

- [ ] **Step 3: Build & commit**

```bash
cargo build -q
git add crates/ir/src/tree.rs
git commit -m "ir(tree): add Expr::Assign variant for assignment-in-expression-position"
```

---

## Task 8: Lower JS AssignmentExpression in expression position

**Files:**
- Modify: `crates/parse/src/lib.rs:884` (`lower_expr`)
- Modify: `crates/parse/src/disposition.rs:565` (the `AssignmentExpression` arm)

- [ ] **Step 1: Add the lowering arm in `lower_expr`**

Add before the catch-all:

```rust
            js::Expression::AssignmentExpression(a) => {
                // Only plain `=` is supported in expression position.
                // Compound (`+=` etc.) and logical (`||=` etc.) remain blocked
                // by disposition; reaching here with a non-Assign operator
                // means the walker is too permissive.
                if !matches!(a.operator, js::AssignmentOperator::Assign) {
                    return Err(Error::Unsupported(
                        "compound/logical assignment in expression position",
                    ));
                }
                let target = self.lower_assign_target(&a.left)?;
                let value = self.lower_expr(&a.right)?;
                Ok(Expr::Assign {
                    target: Box::new(target),
                    value: Box::new(value),
                })
            }
```

- [ ] **Step 2: Stop reporting AssignmentExpr as a blocker**

In `disposition.rs`, modify the `AssignmentExpression(a)` arm in `collect_expr_blockers`:

```rust
        AssignmentExpression(a) => {
            // Plain `=` in expression position is now lowerable.
            // Compound and logical-assign remain blocked by their own variants.
            if !assignment_op_lowerable(a.operator) {
                blockers.insert(BlockerKind::LogicalAssignOp);
            } else if !matches!(a.operator, js::AssignmentOperator::Assign) {
                blockers.insert(BlockerKind::CompoundAssignmentExpr);
            }
            // No `BlockerKind::AssignmentExpr` insertion: plain `=` lowerable.
            collect_assignment_target_blockers(&a.left, blockers);
            collect_expr_blockers(&a.right, blockers);
        }
```

- [ ] **Step 3: Build**

Run: `cargo build -q`
Expected: clean build.

- [ ] **Step 4: Add unit tests**

In `crates/parse/src/lib.rs` (alongside the SequenceExpr tests):

```rust
    #[test]
    fn assign_in_condition_lowers() {
        let src = "function f(g) { let x; if (x = g()) { return x; } return 0; }";
        let m = parse_and_lower(src).expect("must lower");
        for item in &m.items {
            assert!(matches!(item, Item::Function(_)));
        }
    }

    #[test]
    fn assign_chained_lowers() {
        let src = "function f() { let a, b, c; a = b = c = 1; return a + b + c; }";
        let m = parse_and_lower(src).expect("must lower");
        for item in &m.items {
            assert!(matches!(item, Item::Function(_)));
        }
    }

    #[test]
    fn assign_compound_still_blocks() {
        // Compound assignment in expression position remains a blocker.
        let src = "function f(o) { return (o.x += 1); }";
        let m = parse_and_lower(src).expect("must parse");
        for item in &m.items {
            assert!(matches!(item, Item::PassThroughFunction(_)));
        }
    }
```

- [ ] **Step 5: Run unit tests**

Run: `cargo test -q -p parse`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/parse/src/lib.rs crates/parse/src/disposition.rs
git commit -m "parse: lower plain-= AssignmentExpression in expression position to Expr::Assign"
```

---

## Task 9: SSA conversion for Expr::Assign

**Files:**
- Modify: `crates/parse/src/lower_ssa.rs`
- Modify: `crates/ir/src/free_vars.rs`

- [ ] **Step 1: `convert_expr` arm for Assign**

In the `Converter::convert_expr` `match expr {` block, add:

```rust
            Expr::Assign { target, value } => {
                // Plain `=` in expression position: evaluate target receiver/key
                // first (LHS-eval-before-RHS per ECMA-262 13.15.4), then RHS,
                // then emit the store. The expression's value is the RHS.
                match target.as_ref() {
                    Expr::Ident(name) => {
                        let val_op = self.convert_expr(value);
                        self.assign_to_ident(name, val_op.clone());
                        val_op
                    }
                    Expr::Member { obj, prop } => {
                        let obj_op = self.convert_expr(obj);
                        let val_op = self.convert_expr(value);
                        let key = self.builder.intern(prop);
                        self.emit(InstKind::StoreProp {
                            obj: obj_op,
                            key,
                            value: val_op.clone(),
                        });
                        val_op
                    }
                    Expr::Index { obj, index } => {
                        let obj_op = self.convert_expr(obj);
                        let idx_op = self.convert_expr(index);
                        let val_op = self.convert_expr(value);
                        self.emit(InstKind::StoreIndex {
                            obj: obj_op,
                            index: idx_op,
                            value: val_op.clone(),
                        });
                        val_op
                    }
                    _ => panic!("Expr::Assign with unsupported target — disposition gap"),
                }
            }
```

The exact instruction names (`StoreProp`, `StoreIndex`, `assign_to_ident`) must match the ones used by the existing `Stmt::Assign` lowering at `lower_ssa.rs:1550`. Read that block first and copy the exact API. The only difference is that `Stmt::Assign` discards the value; `Expr::Assign` returns it.

- [ ] **Step 2: Other `Expr` walkers**

Add `Expr::Assign` arms to every non-wildcard `match` over `&Expr` in `lower_ssa.rs`. The walkers to update: `walk_reassigned_in_expr`, `discover_expr`, and any other helper that takes an `&Expr`. Each arm should recurse into both `target` and `value`. For `walk_reassigned_in_expr` specifically, an Ident target is a reassignment — record it the same way the function records `Stmt::Assign` to an Ident:

```rust
        Expr::Assign { target, value } => {
            if let Expr::Ident(name) = target.as_ref() {
                out.insert(name.clone());
            }
            walk_reassigned_in_expr(target, out);
            walk_reassigned_in_expr(value, out);
        }
```

- [ ] **Step 3: free_vars.rs — walk_expr arm**

```rust
            Expr::Assign { target, value } => {
                self.walk_expr(target);
                self.walk_expr(value);
            }
```

- [ ] **Step 4: lower_tree.rs `render_expr_to_string`**

```rust
        Expr::Assign { target, value } => {
            format!("({} = {})", render_expr_to_string(target), render_expr_to_string(value))
        }
```

- [ ] **Step 5: Emit arm**

In `crates/emit/src/lib.rs`, add to the expression renderer:

```rust
            Expr::Assign { target, value } => {
                self.out.push('(');
                self.emit_expr(target);
                self.out.push_str(" = ");
                self.emit_expr(value);
                self.out.push(')');
            }
```

- [ ] **Step 6: Build & run**

```bash
cargo build -q
cargo test -q
```
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/parse/src/lower_ssa.rs crates/parse/src/lower_tree.rs \
        crates/ir/src/free_vars.rs crates/emit/src/lib.rs
git commit -m "ssa+emit: handle Expr::Assign as side-effecting value-yielding store"
```

---

## Task 10: AssignmentExpr semantic test fixtures

**Files:**
- Create: `crates/cli/coverage_corpus/functional/18_assign_in_condition.js`
- Create: `crates/cli/coverage_corpus/functional/19_assign_chained.js`
- Create: `crates/cli/coverage_corpus/functional/20_assign_lhs_eval_order.js`
- Create: `crates/cli/coverage_corpus/functional/21_assign_setter_lhs.js`

- [ ] **Step 1: 18_assign_in_condition.js**

```js
function read_until_zero(seq) {
    let i = 0;
    let x;
    let log = [];
    while ((x = seq[i])) {
        log.push(x);
        i = i + 1;
    }
    return log.join(',');
}
```

- [ ] **Step 2: 19_assign_chained.js**

```js
function chained() {
    let a, b, c;
    a = b = c = 7;
    return a + ',' + b + ',' + c;
}
```

- [ ] **Step 3: 20_assign_lhs_eval_order.js**

```js
function lhs_eval_order() {
    let log = [];
    let i = 0;
    function f() { log.push('rhs-i=' + i); i = i + 1; return 'V'; }
    let arr = [null, null];
    arr[(log.push('lhs-i=' + i), i++)] = f();
    // Expected log: lhs-i=0 (LHS index evaluated first), rhs-i=1
    return log.join('|') + '|' + JSON.stringify(arr);
}
```

- [ ] **Step 4: 21_assign_setter_lhs.js**

```js
function setter_value() {
    let stored;
    let obj = {
        set x(v) { stored = v + '!'; }
    };
    // The value of the assignment expression is the RHS, NOT what the setter stored.
    let result = (obj.x = 'hi');
    return result + '|' + stored;  // expected 'hi|hi!'
}
```

- [ ] **Step 5: Verify all four roundtrip and behave**

Run each through `cargo run -- <file>` and confirm Lowered. Then run with `node` against the original AND the deobfuscated output to confirm behavior matches.

- [ ] **Step 6: Commit**

```bash
git add crates/cli/coverage_corpus/functional/18_*.js \
        crates/cli/coverage_corpus/functional/19_*.js \
        crates/cli/coverage_corpus/functional/20_*.js \
        crates/cli/coverage_corpus/functional/21_*.js
git commit -m "test(corpus): AssignmentExpr functional fixtures (cond, chained, eval order, setter)"
```

---

## Task 11: Run corpus and validate AssignmentExpr unlock

- [ ] **Step 1: Run stability**

Same harness as Task 6.

- [ ] **Step 2: Verify**

- Roundtrip 308/308.
- Cumulative unlock through `AssignmentExpr` row ≈ 218 (per the ticket's table).

- [ ] **Step 3: Commit**

```bash
git add corpus/realworld/STABILITY_REPORT.md
git commit -m "corpus: regen after AssignmentExpr lowering (cumulative unlock N)"
```

---

## Task 12: IrFunction.source field for SSA fallback

**Files:**
- Modify: `crates/ir/src/tree.rs` (`IrFunction`)
- Modify: `crates/parse/src/lib.rs` (`lower_function_lowered`)
- Modify: `crates/parse/src/lower_ssa.rs` (consume the field for ForIn/Try fallback)

- [ ] **Step 1: Add the field**

In `tree.rs`, modify `IrFunction`:

```rust
pub struct IrFunction {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    /// Verbatim body source slice (between `{` and `}`, exclusive).
    /// Populated at AST→tree-IR time. Used by the SSA layer to fall back
    /// to a PassThrough function when the body contains a construct that
    /// cannot be expressed in SSA form (currently `Stmt::ForIn` and
    /// `Stmt::Try`). `None` for hand-built tree-IR in tests.
    pub source: Option<String>,
}
```

- [ ] **Step 2: Populate at lowering**

In `lib.rs:lower_function_lowered`, slice the body source the same way `passthrough_literal` does (using `slice_body_inner(body.span())`). Stash it in the `IrFunction` constructor:

```rust
        let body_span = body.span();
        let source = self.slice_body_inner(body_span).map(|s| s.to_string());
        let stmts = self.lower_stmts_list(&body.statements)?;
        Ok(Item::Function(IrFunction {
            name: name.to_string(),
            params,
            body: stmts,
            source,
        }))
```

- [ ] **Step 3: Update every other `IrFunction { ... }` constructor**

Search for `IrFunction {` to find every callsite (tests included). Add `source: None` to hand-built ones.

- [ ] **Step 4: Build**

```bash
cargo build -q
cargo test -q
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ir/src/tree.rs crates/parse/src/lib.rs
git commit -m "ir(tree): add IrFunction.source for SSA-fallback PassThrough"
```

---

## Task 13: Tree-IR additions for ForInStmt

**Files:**
- Modify: `crates/ir/src/tree.rs`

- [ ] **Step 1: Add `Stmt::ForIn`**

```rust
    /// `for (kind binding in obj) body`. Iterates string-named enumerable
    /// properties of `obj`, walking the prototype chain. Preserved
    /// syntactically rather than desugared (the iteration semantics are
    /// hard to reimplement faithfully). Functions containing this
    /// statement do not enter SSA optimization — see
    /// `lower_ssa::lower_top_level_function`'s ForIn-fallback path.
    /// Does enter pass-through emit cleanly.
    ForIn {
        /// `var`, `let`, or `const`. Stored as a string for parity with `ForOf`.
        kind: String,
        /// Plain identifier binding.
        binding: String,
        /// The object being enumerated.
        obj: Box<Expr>,
        /// Loop body.
        body: Vec<Stmt>,
    },
```

- [ ] **Step 2: Extend `dump_stmt`**

Add an arm mirroring `Stmt::ForOf`'s dump.

- [ ] **Step 3: Build & commit**

```bash
cargo build -q
git add crates/ir/src/tree.rs
git commit -m "ir(tree): add Stmt::ForIn variant"
```

---

## Task 14: Lower JS ForInStatement to Stmt::ForIn

**Files:**
- Modify: `crates/parse/src/lib.rs` (add `lower_for_in_stmt`, wire into `lower_stmt`)
- Modify: `crates/parse/src/disposition.rs` (stop reporting ForInStmt)

- [ ] **Step 1: lower_for_in_stmt**

Mirror `lower_for_of_stmt`:

```rust
    fn lower_for_in_stmt(&self, f: &js::ForInStatement) -> Result<Stmt, Error> {
        let (kind, binding) = match &f.left {
            js::ForStatementLeft::VariableDeclaration(decl) => {
                if decl.declarations.len() != 1 {
                    return Err(Error::Unsupported("for-in with multiple declarators"));
                }
                let d = &decl.declarations[0];
                let name = match &d.id.kind {
                    js::BindingPatternKind::BindingIdentifier(id) => id.name.to_string(),
                    _ => return Err(Error::Unsupported("for-in with destructuring binding")),
                };
                let kind = match decl.kind {
                    js::VariableDeclarationKind::Const => "const".to_string(),
                    js::VariableDeclarationKind::Let => "let".to_string(),
                    js::VariableDeclarationKind::Var => "var".to_string(),
                    _ => return Err(Error::Unsupported("for-in with exotic binding kind")),
                };
                (kind, name)
            }
            _ => return Err(Error::Unsupported("for-in with non-declaration left side")),
        };
        let obj = self.lower_expr(&f.right)?;
        let body = self.stmts_from(&f.body)?;
        Ok(Stmt::ForIn {
            kind,
            binding,
            obj: Box::new(obj),
            body,
        })
    }
```

- [ ] **Step 2: Wire into `lower_stmt`**

In `lower_stmt`, add:

```rust
            js::Statement::ForInStatement(f) => self.lower_for_in_stmt(f),
```

- [ ] **Step 3: Disposition**

In `disposition.rs:167`, modify the `ForInStatement(f)` arm:

```rust
        ForInStatement(f) => {
            // The for-in loop's binding must be a single plain identifier
            // (matching ForOfStatement). Multi-declarator and destructuring
            // remain blocked because the IR can only represent the simple form.
            let binding_ok = match &f.left {
                js::ForStatementLeft::VariableDeclaration(decl) => {
                    decl.declarations.len() == 1
                        && matches!(
                            decl.declarations[0].id.kind,
                            js::BindingPatternKind::BindingIdentifier(_)
                        )
                        && matches!(
                            decl.kind,
                            js::VariableDeclarationKind::Const
                                | js::VariableDeclarationKind::Let
                                | js::VariableDeclarationKind::Var
                        )
                }
                _ => false,
            };
            if !binding_ok {
                blockers.insert(BlockerKind::ForInStmt);
            }
            collect_expr_blockers(&f.right, blockers);
            collect_stmt_blockers(&f.body, blockers);
        }
```

(We keep `BlockerKind::ForInStmt` only when the binding shape is too exotic for our IR; the common case stops being a blocker.)

- [ ] **Step 4: Stmt walkers in lower_ssa.rs and free_vars.rs**

Add `Stmt::ForIn` arms to every non-wildcard `match stmt {` over `&Stmt` (the same set of walkers updated in Task 3 / 9, but for statements). For `lower_ssa::lower_stmt` specifically, the arm will trigger the SSA fallback (see Task 15) — for now, panic with a clear "fallback not yet wired" message so the build stays exhaustive.

- [ ] **Step 5: Build**

```bash
cargo build -q
```
Expected: clean build.

- [ ] **Step 6: Commit**

```bash
git add crates/parse/src/lib.rs crates/parse/src/disposition.rs \
        crates/parse/src/lower_ssa.rs crates/ir/src/free_vars.rs
git commit -m "parse: lower ForInStatement to Stmt::ForIn; disposition stops blocking the simple shape"
```

---

## Task 15: SSA fallback for Stmt::ForIn (and Stmt::Try)

**Files:**
- Modify: `crates/parse/src/lower_ssa.rs`

- [ ] **Step 1: Body-scan helper**

Add a helper that detects `Stmt::ForIn` or `Stmt::Try` anywhere in a function body (recursing into nested control flow but NOT into nested function literals — those have their own disposition):

```rust
fn body_needs_ssa_fallback(stmts: &[Stmt]) -> bool {
    fn visit(s: &Stmt) -> bool {
        match s {
            Stmt::ForIn { .. } | Stmt::Try { .. } => true,
            Stmt::If { then_body, else_body, .. } => {
                then_body.iter().any(visit)
                    || else_body.as_ref().is_some_and(|b| b.iter().any(visit))
            }
            Stmt::While { body, .. }
            | Stmt::For { body, .. }
            | Stmt::ForOf { body, .. }
            | Stmt::Block(body) => body.iter().any(visit),
            Stmt::FuncDecl(_) => false,
            _ => false,
        }
    }
    stmts.iter().any(visit)
}
```

- [ ] **Step 2: Route top-level Lowered functions through fallback**

In `lower_module` (around line 124, the `PendingKind::TopLevel(f)` match arm), check the body before calling `lower_top_level_function`. If `body_needs_ssa_fallback(&f.body)`, instead push a synthesized PassThrough using `f.source`:

```rust
            PendingKind::TopLevel(f) => {
                if body_needs_ssa_fallback(&f.body) {
                    let src = f.source.clone().unwrap_or_default();
                    push_top_level_passthrough_from_lowered(&mut sm, f, src);
                    continue;
                }
                let pt_refs = pt_referenced_for_fid
                    .get(&p.fid)
                    .cloned()
                    .unwrap_or_default();
                lower_top_level_function(/* ... */);
            }
```

Implement `push_top_level_passthrough_from_lowered` to mirror `push_passthrough_function` but synthesize from `IrFunction` instead of `FuncLiteral`. Build a one-shot `FuncLiteral` with `body: FuncBody::PassThrough(src)`, `name: Some(f.name)`, `params: f.params.clone()`, etc. Then call the existing `push_passthrough_function`.

- [ ] **Step 3: Same fallback for inner literals**

In `lower_inner_literal`, before lowering `FuncBody::Stmts(ss)`, check `body_needs_ssa_fallback(ss)` and route to `push_inner_passthrough_function_with_source` using `lit.passthrough_source` (already exists for inner literals).

- [ ] **Step 4: Remove the `Stmt::Try` panic at line 1697**

Replace with `unreachable!("body_needs_ssa_fallback should have routed try-bearing functions to PassThrough")` and add the same for ForIn:

```rust
            Stmt::Try { .. } | Stmt::ForIn { .. } => {
                unreachable!(
                    "Stmt::{{Try,ForIn}} reached SSA conversion — \
                     body_needs_ssa_fallback should have routed this function to PassThrough"
                );
            }
```

- [ ] **Step 5: Build & test**

```bash
cargo build -q
cargo test -q
```

Run the previously-failing try fixture to confirm the latent bug is fixed:

```bash
cargo run -q --bin cli -- crates/cli/coverage_corpus/functional/07_try_catch_in_body.js
```

Expected: completes without panic. Output preserves the function as PassThrough in SSA but disposition reports it as Lowered.

- [ ] **Step 6: Commit**

```bash
git add crates/parse/src/lower_ssa.rs
git commit -m "ssa: fall back to PassThrough for functions containing ForIn or Try (fixes latent panic)"
```

---

## Task 16: Emit and free_vars for Stmt::ForIn

**Files:**
- Modify: `crates/emit/src/lib.rs`
- Modify: `crates/ir/src/free_vars.rs`

- [ ] **Step 1: Emitter arm**

Mirror `Stmt::ForOf` (around line 292 of `emit/src/lib.rs`):

```rust
            Stmt::ForIn { kind, binding, obj, body } => {
                self.indent();
                self.out.push_str("for (");
                self.out.push_str(kind);
                self.out.push(' ');
                self.out.push_str(binding);
                self.out.push_str(" in ");
                self.emit_expr(obj);
                self.out.push_str(") {\n");
                self.depth += 1;
                for s in body {
                    self.emit_stmt(s);
                }
                self.depth -= 1;
                self.indent();
                self.out.push_str("}\n");
            }
```

(Adapt to whatever indent/output API the emitter uses.)

- [ ] **Step 2: free_vars.rs walk_stmt arm**

```rust
            Stmt::ForIn { binding, obj, body, .. } => {
                self.walk_expr(obj);
                self.push_scope();
                self.bind(binding);
                for s in body {
                    self.walk_stmt(s);
                }
                self.pop_scope();
            }
```

(Match the actual scope-management API of the file.)

- [ ] **Step 3: Build & test**

```bash
cargo build -q
cargo test -q
```

- [ ] **Step 4: Commit**

```bash
git add crates/emit/src/lib.rs crates/ir/src/free_vars.rs
git commit -m "emit+free_vars: handle Stmt::ForIn"
```

---

## Task 17: ForInStmt semantic test fixtures

**Files:**
- Create: `crates/cli/coverage_corpus/functional/22_forin_basic.js`
- Create: `crates/cli/coverage_corpus/functional/23_forin_var_scope.js`
- Create: `crates/cli/coverage_corpus/functional/24_forin_break_continue.js`

- [ ] **Step 1: 22_forin_basic.js**

```js
function basic_forin() {
    let parent = { a: 1 };
    let child = Object.create(parent);
    child.b = 2;
    let keys = [];
    for (let k in child) {
        keys.push(k);
    }
    return keys.sort().join(',');  // expected "a,b" (inherited + own)
}
```

- [ ] **Step 2: 23_forin_var_scope.js**

```js
function var_scope() {
    let obj = { a: 1, b: 2 };
    for (var k in obj) { /* k hoists to fn scope */ }
    return typeof k === 'string' ? k : 'unbound';  // 'b' or 'a' depending on order
}
```

- [ ] **Step 3: 24_forin_break_continue.js**

```js
function break_continue() {
    let obj = { a: 1, b: 2, c: 3, d: 4 };
    let log = [];
    for (let k in obj) {
        if (k === 'b') continue;
        if (k === 'd') break;
        log.push(k);
    }
    return log.join(',');  // expected "a,c"
}
```

- [ ] **Step 4: Verify each Lowers (no longer PT) and roundtrips**

```bash
for f in 22 23 24; do
  cargo run -q --bin cli -- crates/cli/coverage_corpus/functional/${f}_*.js | tail -20
done
```

Each should show the function as Lowered in the tree-IR dump (or as a PassThrough body via the SSA fallback — but the disposition row should be Lowered). The emitted JS should preserve `for (... in ...)` syntactically.

- [ ] **Step 5: Behavioral check via node**

Diff `node original.js` vs `node deobfuscated.js` for each fixture. Output must match.

- [ ] **Step 6: Commit**

```bash
git add crates/cli/coverage_corpus/functional/22_*.js \
        crates/cli/coverage_corpus/functional/23_*.js \
        crates/cli/coverage_corpus/functional/24_*.js
git commit -m "test(corpus): ForInStmt functional fixtures (basic, var-scope, break/continue)"
```

---

## Task 18: Final corpus run and acceptance check

- [ ] **Step 1: Regenerate stability report**

Same harness used in Task 6 / 11.

- [ ] **Step 2: Verify acceptance criteria from the ticket**

From `corpus/realworld/STABILITY_REPORT.md`:

- Roundtrip 308/308 (hard CI gate).
- Total passthrough function count: ≥220 functions moved out of PassThrough vs. the pre-ticket 305 baseline. The cumulative target through ForInStmt is 228; ≥220 leaves margin.
- The blocker analysis table no longer shows rows for `SequenceExpr`, `AssignmentExpr`, or `ForInStmt` (or shows them with 0 occurrences).

- [ ] **Step 3: If unlock count is below 220, investigate**

Likely causes:
- A lowering produces a tree-IR shape that doesn't roundtrip cleanly (other passes panic or emit wrong code, fooling roundtrip), causing functions to fall back to PassThrough by some upstream mechanism.
- The disposition logic still flags one of the three blockers because of an arm-ordering issue.
- The greedy projection had an edge case (less likely; the table predicts 228 cumulative).

Fix the cause, or revert and shrink the ticket scope.

- [ ] **Step 4: Commit corpus update**

```bash
git add corpus/realworld/STABILITY_REPORT.md
git commit -m "corpus: regen after ForInStmt lowering — total unlock N functions"
```

---

## Task 19: Push branch and open PR

- [ ] **Step 1: Push**

```bash
git push -u origin feat/lower-sequence-assignment-forin
```

- [ ] **Step 2: Open PR**

```bash
gh pr create --title "feat: lower SequenceExpr, AssignmentExpr, ForInStmt" --body "$(cat <<'EOF'
## Summary

Lowers three high-frequency blockers via syntactic preservation:

- `Expr::Sequence` for the comma operator. Preserves bundler-interop pattern `(0, obj.method)(args)` correctly.
- `Expr::Assign` for plain-`=` assignment used in expression position. Setter-LHS spec subtlety handled by syntactic round-trip.
- `Stmt::ForIn` for `for (k in obj)`. Functions containing ForIn route to PassThrough at the SSA boundary (mirrors precedent for `Stmt::Try`; also fixes the latent panic at lower_ssa.rs:1697 for try-bearing functions).

Cumulative unlock vs. pre-ticket baseline: **N functions** (target ≥220).

## Test plan

- [ ] Corpus roundtrip 308/308
- [ ] Stability report blocker table no longer includes SequenceExpr / AssignmentExpr / ForInStmt
- [ ] Functional fixtures 15-24 pass behavior-equivalence vs. `node` baseline
- [ ] `(0, obj.method)()` correctly logs `undefined` (this-binding regression test)
- [ ] Existing `cargo test -q` continues to pass
EOF
)"
```

- [ ] **Step 3: Comment the actual unlock delta on the PR**

Per ticket guidance: "Comment the actual unlock count on the PR before merging." Pull the number from the regenerated stability report and add it as a PR comment.

---

## Self-review

- **Spec coverage:** Every requirement in the ticket is mapped:
  - SequenceExpr lowering → Tasks 1-5
  - AssignmentExpr lowering (plain-`=` only) → Tasks 7-10
  - ForInStmt lowering → Tasks 12-17
  - Statement-hoisting infrastructure: NOT NEEDED under the preservation-as-IR-node design
  - Evaluation-order tests: fixtures 17, 20, 22-24
  - `this`-binding test for SequenceExpr: fixture 15
  - Setter-LHS test: fixture 21
  - For-in `var` vs `let` scoping: fixture 23
  - For-in break/continue: fixture 24
  - Predicted-unlock validation ≥220: Task 18
  - Feature-branch + PR workflow: Tasks 0 (already done) and 19
- **Out-of-scope items in the ticket:** CompoundAssignmentExpr, LogicalAssignOp, destructuring assignment, BreakStmt double-counting, ThisExpr — none touched.
- **Risk if SequenceExpr balloons:** the ticket's escape hatch (ship SequenceExpr alone, re-ticket the rest) is preserved by the per-construct ordering and the per-construct corpus-validation tasks (6, 11, 18). If Task 6's roundtrip drops, stop and re-evaluate before starting Task 7.
