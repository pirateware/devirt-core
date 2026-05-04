# T2 (closures-and-pass-through) — remaining work plan

Current branch: `main`. Tree green at 56/56 tests; the core T2 commitment
("pipeline never aborts on valid input") holds at top-level granularity.

This plan lays out the remaining work as four streams. **Stream A is
strictly sequential and blocks B/C/D.** Once A lands, B/C/D can run
concurrently in separate sessions/agents.

Read first: `memory/project_t2-status.md`,
`memory/project_closures-disposition-rule.md`. Those record the design
decisions already locked.

---

## What's already in place

| Capability | Where | Status |
|---|---|---|
| `Operand::Capture(usize)` + capture_count validation | `crates/mvp/src/ir/ssa.rs` | done |
| `FunctionBody::{Lowered(LoweredBody), PassThrough(PassThroughBody)}` | `crates/mvp/src/ir/ssa.rs` | done |
| `Function::{lowered(), lowered_mut(), as_lowered(), is_pass_through()}` | `crates/mvp/src/ir/ssa.rs` | done |
| `FuncLiteral { name, params, is_async, is_generator, is_arrow, body: FuncBody }` | `crates/mvp/src/ir/tree.rs` | done |
| `FuncBody::{Stmts, Expression, PassThrough}` | `crates/mvp/src/ir/tree.rs` | done |
| `Stmt::FuncDecl(FuncLiteral)` and `Expr::Func(Box<FuncLiteral>)` | `crates/mvp/src/ir/tree.rs` | unwired (lower.rs never emits these; lower_ssa panics if reached) |
| `Item::PassThroughFunction(FuncLiteral)` | `crates/mvp/src/ir/tree.rs` | wired |
| `disposition::{decide, Disposition}` for OXC AST | `crates/mvp/src/ir/disposition.rs` | done; called only for top-level functions in lower.rs |
| `lower.rs` slices source for top-level PassThrough via `Lowerer<'src>` | `crates/mvp/src/lower.rs` | done; doesn't yet recurse into nested functions |
| `PassManager::run_module` skips PassThrough | `crates/mvp/src/ir/pass.rs` | done |
| `emit::Printer::write_func_header / write_func_body` | `crates/mvp/src/emit.rs` | done |
| Tree-IR `dump` arms for closure variants | `crates/mvp/src/ir/tree.rs` | done |
| Tests: top-level PassThrough end-to-end (class / generator / try-catch / mixed) | `crates/mvp/src/tests.rs` | done |

---

## Stream A — Foundation refactor (sequential, blocks everything else)

**Scope:** Make recursive inner-function lowering possible. Today
`FunctionBuilder<'m>` holds `&'m mut Module`, so building an inner
function during an outer build is a borrow-checker conflict. This
stream solves that, plus adds the shared free-var analysis utility
and pre-pass FuncId allocation.

**One agent. ~3–5 hours.**

### Tasks
1. **Two-pass `lower_ssa::lower_module`.**
   - Pass 1: walk `tree::Module` in source pre-order, descending into
     `Stmt::FuncDecl` / `Expr::Func` / `FuncBody::Stmts|Expression`.
     For every function-shaped node (Item::Function, Item::PassThroughFunction,
     Stmt::FuncDecl, Expr::Func) assign a `FuncId` and record it in
     a `Vec<PendingFunction>`.
   - Pass 2: build each `PendingFunction` independently. Inner
     functions need only their captures list (just names) — they
     don't need the outer's lowering state.
   - Build order: any topological order works. Suggest source pre-order
     (FuncIds = build order). The push to `module.functions` happens at
     each `FunctionBuilder::finish()`, which already pushes in order.

2. **`PendingFunction`** carries:
   ```rust
   struct PendingFunction<'a> {
       fid: FuncId,
       name: String,                 // inner anon literals: synthesized
       params: Vec<String>,
       body: PendingBody<'a>,        // Stmts(&[Stmt]) | Expression(&Expr) | PassThrough(String)
       captures: Vec<String>,        // free var names in stable order (alphabetical)
       is_async: bool,
       is_generator: bool,
       is_arrow: bool,
   }
   ```
   Lifetime `'a` borrows the tree-IR Module, which lives for the call.

3. **Free-var analysis utility.** New module
   `crates/mvp/src/ir/free_vars.rs` (or inline in lower_ssa). Inputs:
   `&FuncLiteral` plus the function's own param names. Walks the body's
   tree-IR collecting `Expr::Ident(name)` references that are NOT
   bound by an enclosing `let`/`const`/param/inner-FuncDecl-name within
   the body. Crosses descent into nested function literals — those have
   their own free-var lists, but identifiers they capture from the
   outer body still count as free for THIS function's analysis only if
   they're then referenced. Simpler: descent stops at nested function
   boundaries; nested closures' free vars are computed for THEIR own
   PendingFunction.
   - **Determinism:** the captures list is sorted alphabetically by
     name. Both inner and outer agree on this ordering.
   - **Edge cases:** `Expr::Func`'s own name (named function expr)
     binds within itself; arrow's `this` is NOT captured here (we
     punt on `this` per the spec).

4. **Outer-function lowering encounters `Stmt::FuncDecl` / `Expr::Func`.**
   - Look up the inner's pre-allocated FuncId via the
     `PendingFunction` mapping. Recommendation: stash
     `func_id: Cell<Option<FuncId>>` on `FuncLiteral` (interior
     mutability so the tree-IR can stay `&`). Pass 1 fills it; Pass 2
     reads it.
   - For each capture name in `pending.captures`, call outer
     Converter's `lookup` to get the SymbolId, then `read_variable` to
     get the `Operand`. Build the captures vector.
   - Emit `InstKind::AllocClosure { func: inner_fid, captures }`.
   - For `Stmt::FuncDecl`: bind the result to the inner's name in the
     current outer scope. Tag the resulting Value's `source_name` so
     the renamer preserves the function's name.
   - For `Expr::Func`: return the `AllocClosure` value as the
     expression's operand.

5. **Inner-function lowering uses `Operand::Capture(idx)`.**
   - When the inner Converter is built, it receives the captures list.
     It pre-binds each capture name to a "capture symbol" that resolves
     to `Operand::Capture(idx)` instead of going through the normal
     `current_def` machinery.
   - Implementation: extend `Converter::lookup` (or a wrapping layer)
     to first check capture names. Cleanest: scope 0 (which currently
     holds module-level function names) is replaced/augmented for inner
     functions with capture bindings.

6. **`Function::capture_count`** is set from `pending.captures.len()` at
   build time. Validator already enforces `Operand::Capture(idx)` is
   in range.

### Acceptance for Stream A
- `cargo test` passes 56/56 (no behavior change yet — Stream A is purely
  refactoring infrastructure for B).
- `cargo clippy --all-targets -- -D warnings` clean.
- The two-pass scaffolding compiles and runs even though no
  `Stmt::FuncDecl` / `Expr::Func` nodes are yet emitted by `lower.rs`.

### Files touched
- `crates/mvp/src/ir/lower_ssa.rs` (major)
- `crates/mvp/src/ir/free_vars.rs` (new) or inline
- `crates/mvp/src/ir/mod.rs` (if new module)
- `crates/mvp/src/ir/tree.rs` (only if adding `func_id: Cell<Option<FuncId>>` to `FuncLiteral`)

---

## Stream B — Lowered nested closures (depends on A)

**Scope:** Wire `lower.rs` to emit `Stmt::FuncDecl` and `Expr::Func`
when nested functions / function expressions / arrows are encountered
AND their disposition is `Lowered`. PassThrough nested functions can
also flow through (their `FuncBody` is `PassThrough(slice)`); Stream A's
build path handles PassThrough directly.

**One agent. ~4–6 hours.**

### Tasks
1. **`lower.rs::lower_stmt` — nested `FunctionDeclaration`.**
   Replace the current `Err(Error::Unsupported("nested function declaration"))`.
   Run `disposition::decide(&f.params, body)`. If Lowered, recurse
   into the body (collecting params and `Vec<Stmt>`). If PassThrough,
   slice the body source. Emit `Stmt::FuncDecl(FuncLiteral)` either way.

2. **`lower.rs::lower_expr` — `FunctionExpression`.**
   Same logic; emit `Expr::Func(Box<FuncLiteral>)` with `name = f.id.as_ref().map(...)`.

3. **`lower.rs::lower_expr` — `ArrowFunctionExpression`.**
   - Block-body arrow: `FuncBody::Stmts(...)` with `is_arrow=true`.
   - Expression-body arrow `(x) => x+1`: `FuncBody::Expression(Box::new(...))` with `is_arrow=true`.
   - PassThrough fallback (e.g. arrow body uses `await`): slice the body span.
     Note: arrow expression-body PassThrough requires care — the slice
     must capture the expression text, not be wrapped in braces.
     Suggest: stash a flag on FuncLiteral or a separate `FuncBody::PassThroughExpression(String)` variant if needed.

4. **`lower_ssa.rs` — Converter handlers for `Stmt::FuncDecl` and `Expr::Func`.**
   See Stream A task 4. The body is built independently in Stream A's
   Pass 2; the outer's job here is purely:
   - look up FuncId,
   - resolve captures from outer scope by name,
   - emit `AllocClosure`,
   - bind name (FuncDecl) or return operand (Func).

5. **`lower_tree.rs` — `AllocClosure` resugaring.**
   Currently `lower_tree.rs:445-446` panics on `AllocClosure`. Replace
   with: look up the target `Function` in the SSA module. If
   `FunctionBody::Lowered`: recursively lower its body to
   `Vec<Stmt>` (call `lower_function` on it), then wrap in a
   `FuncLiteral` with `body: FuncBody::Stmts(...)`. If
   `FunctionBody::PassThrough(pt)`: emit `FuncLiteral` with
   `body: FuncBody::PassThrough(pt.source.clone())`.
   - For an `AllocClosure` inside a statement context (the result is
     bound to a let), emit `Stmt::FuncDecl` if the binding name matches
     the function's source name; otherwise emit a `Stmt::Const { name, init: Expr::Func }`.
     Pragmatic choice: always emit `Const` + `Expr::Func`; the renamer
     produces correct JS either way.
   - For `AllocClosure` inside an expression context, emit
     `Expr::Func(Box::new(literal))`.
   - **Capture resolution at emit time:** inside the inner function's
     body (recursively lowered), `Operand::Capture(idx)` translates to
     `Expr::Ident(captures[idx])` where `captures` is the names list
     resolved from the outer scope. The outer scope's name for each
     capture is the source name of the corresponding `AllocClosure`
     operand. **Stream C** wires the EmitConfig pin to ensure this
     name doesn't get suffix-renamed.

6. **Walker rule (forward-ref / reassigned-capture → PassThrough).**
   Per `memory/project_closures-disposition-rule.md`. Recommended
   implementation: at `lower_ssa.rs` Pass 1 time, when computing
   `PendingFunction.captures`, also check the outer scope for each
   capture name:
   - **Forward-ref:** capture name is referenced in inner body but not
     yet bound at the inner function's textual position in the outer.
     Track outer's bindings-seen-so-far during Pass 1's source-pre-order walk.
   - **Reassigned:** capture name is the LHS of any `Stmt::Assign`
     in the outer body (anywhere, not just before the closure).
     Pre-compute outer's reassigned-name set in a fast first-pass.
   - If either holds: re-classify the inner function as PassThrough.
     This requires the source slice for the inner — Stream B should
     stash a parallel `passthrough_source: Option<String>` on every
     nested FuncLiteral at lower.rs time so this re-classification is
     cheap.

### New tests (add to `crates/mvp/src/tests.rs`)
From the original ticket's ACCEPTANCE list:
- IIFE: `(function(x){ return x+1; })(2)` → SSA contains AllocClosure + Call.
- function expression bound: `let f = function(x){return x*2;}; f(5)`.
- arrow: `let f = (x) => x*2; f(5)`.
- nested decl: `function outer(){ function inner(x){return x+1;} return inner(2); }`.
- capture: `function outer(){ let y=10; return (x)=>x+y; }`.
- forward-ref: `function outer(){ let f = (x) => x+y; let y=10; return f(1); }` — inner becomes PassThrough.
- reassigned: `function outer(){ let y=10; let f = (x) => x+y; y=20; return f(1); }` — inner becomes PassThrough.

### Acceptance for Stream B
- All Stream A tests still pass.
- New tests pass.
- `cargo clippy --all-targets -- -D warnings` clean.

### Files touched
- `crates/mvp/src/lower.rs` (major)
- `crates/mvp/src/ir/lower_ssa.rs` (medium — adds Stmt::FuncDecl/Expr::Func arms)
- `crates/mvp/src/ir/lower_tree.rs` (major — AllocClosure handling)
- `crates/mvp/src/ir/tree.rs` (minor — possible new variant for arrow-expression PassThrough)
- `crates/mvp/src/tests.rs` (additions)

---

## Stream C — Capture-by-passthrough pin + oxc_codegen

**Scope:** When a Lowered outer function captures a value into a
PassThrough inner, the captured Value's source name must appear
unchanged in the emitted output (the PassThrough body's text
references the name lexically). Also: replace the verbatim source
slice emit with an `oxc_codegen` re-parse so PassThrough bodies survive
local IR transformations of the surrounding code.

**Depends on Stream A's foundations. Mostly independent of Stream B's
specifics — can be drafted before B lands and integrated after.**

**One agent. ~2–4 hours.**

### Tasks
1. **Add `oxc_codegen = "0.40"` to `crates/mvp/Cargo.toml`.**

2. **Mark capture-by-passthrough at lower-time.**
   - In `lower_ssa.rs`, when emitting an `AllocClosure { func, captures }`
     where `func` resolves to a PassThrough function, walk the captures.
     For each `Operand::Value(v)`, mark the underlying Value as
     captured-by-passthrough. Need a side-table on the
     in-progress function (`Vec<bool>` indexed by Value or a `HashSet<u32>`).
   - For `Operand::Const`, no marking needed.
   - For `Operand::Capture(idx)` in this position (the outer is
     itself an inner Lowered function): the outer's capture-by-passthrough
     marking propagates to ITS containing AllocClosure as well. Track
     this transitively.
   - Constraint check: a captured-by-passthrough Value must have a
     `source_name`. If it doesn't (synthesized intermediate), panic
     with the clear message from the spec — the lowering must never
     capture a synthesized intermediate into a PassThrough body.

3. **EmitConfig wiring at SSA→tree time.**
   - In `lower_tree.rs::lower_function`, retrieve the function's
     captured-by-passthrough set (stored on `Function` — extend the
     struct with `pub captured_by_passthrough: Vec<u32>` or similar).
   - For each Value in that set with `source_name = Some(strid)`,
     populate `EmitConfig.immovable_names` with `(value_id, strid_resolved)`.
   - The renamer's pin behavior (already implemented in Stream T1)
     does the rest.

4. **PassThrough emit via oxc_codegen.**
   - Replace `emit::Printer::write_func_body`'s `FuncBody::PassThrough(src)`
     branch with a re-parse + codegen path:
     ```text
     wrapper = format!("(function ({}) {{ {} }})", params.join(", "), src);
     parse wrapper with oxc_parser;
     extract the FunctionBody node;
     run oxc_codegen on it;
     splice into output.
     ```
   - For arrows / generators / async, the wrapper changes:
     - arrow block: `(function ({}) {{ {} }})` is fine for re-parse;
       the wrapper is just the parser harness — output codegen renders
       the body in the OUTER's intended shape (which the caller
       constructs around it).
     - generator: wrapper `(function* ({}) {{ {} }})`.
     - async: wrapper `(async function ({}) {{ {} }})`.
   - Acceptable degradation: comments are dropped (re-parse + codegen
     does not preserve them). Per the spec, this is in scope.

5. **Round-trip stability test.**
   - Pipe a PassThrough function through the pipeline twice; assert
     byte-identical output between run 1 and run 2.

### Acceptance for Stream C
- All Stream A/B tests still pass.
- New test: pin pathology (two values both prefer "tmp", one is
  captured-by-passthrough, the other emits as `tmp_1`).
- New test: synthesized-capture into PassThrough panics with the
  specified message.
- New test: round-trip stability is byte-identical.
- `cargo clippy --all-targets -- -D warnings` clean.

### Files touched
- `crates/mvp/Cargo.toml`
- `crates/mvp/src/ir/ssa.rs` (Function gains captured_by_passthrough)
- `crates/mvp/src/ir/lower_ssa.rs` (marking)
- `crates/mvp/src/ir/lower_tree.rs` (EmitConfig wiring)
- `crates/mvp/src/emit.rs` (oxc_codegen re-parse path)
- `crates/mvp/src/tests.rs` (additions)

---

## Stream D — Cross-check + functional-equivalence tests

**Scope:** Asymmetric debug cross-check (catches walker-too-permissive)
and Node-shellout tests for behavioral equivalence.

**Depends on Stream A. Independent of B/C in implementation, though
some test cases want B's nested closures to be useful.**

**One agent. ~2 hours.**

### Tasks
1. **Asymmetric cross-check in `lower_ssa.rs`.**
   - In `cfg!(debug_assertions)` blocks, when the disposition walker
     says `Lowered`, run the lowering and intercept
     `Error::Unsupported`. If hit, panic with a clear message naming
     the construct.
   - Place this around `lower.rs::lower_top_function` and the Stream-B
     equivalents for nested functions.
   - The other direction (walker too conservative — disposition said
     PassThrough but lowering would have succeeded) is documented as
     uncaught per the design conversation. Add a comment at the
     cross-check site noting this and pointing to the
     scratch-module fallback option.

2. **Cross-check tests.**
   - Hand-craft a test that LIES — a custom is_lowerable_node that
     returns Lowered for `class Foo {}`. Assert the cross-check panics
     in debug builds.
   - Symmetric "lying" walker check: walker too conservative for a
     fully-lowerable construct. Today this is uncaught — assert it
     does NOT panic (documenting the gap).
   - Release-build assertion: `cfg!(not(debug_assertions))` path
     compiles out cleanly (no perf cost). Test via a feature flag or
     just by inspection.

3. **Functional-equivalence tests (Node shellout).**
   - Existing `node-tests` feature in Cargo.toml is the gate.
   - Add ~5–10 small programs covering: Lowered closures, PassThrough
     bodies, captures across the boundary, mixed Lowered/PassThrough
     at module level.
   - For each, run input through Node, run output through Node (with
     the same stdin if needed), assert stdout/stderr/exit-code match.
   - Test helper: `fn assert_functional_equivalence(input: &str, stdin: &str)`.

### Acceptance for Stream D
- All previous tests still pass.
- Cross-check tests pass.
- `cargo test --features node-tests` passes (requires Node).
- `cargo clippy --all-targets -- -D warnings` clean.

### Files touched
- `crates/mvp/src/ir/lower_ssa.rs` (cross-check sites)
- `crates/mvp/src/lower.rs` (cross-check sites)
- `crates/mvp/src/tests.rs` (cross-check tests)
- `crates/mvp/tests/functional_equivalence.rs` (new file, gated by `node-tests`)

---

## Suggested parallelization

```
                  ┌──────────────┐
                  │   Stream A   │  (foundation, blocks others)
                  │  ~3-5 hours  │
                  └──────┬───────┘
                         │
        ┌────────────────┼────────────────┐
        │                │                │
        ▼                ▼                ▼
┌──────────────┐ ┌──────────────┐ ┌──────────────┐
│   Stream B   │ │   Stream C   │ │   Stream D   │
│  Lowered     │ │  Pin +       │ │  Cross-check │
│  closures    │ │  oxc_codegen │ │  + functional│
│  ~4-6 hours  │ │  ~2-4 hours  │ │  ~2 hours    │
└──────┬───────┘ └──────┬───────┘ └──────┬───────┘
       │                │                │
       └────────────────┴────────────────┘
                        │
                        ▼
              merge + final integration
```

- **B + C share a few files** (`lower_tree.rs`, `lower_ssa.rs`,
  `tests.rs`). Coordinate via a merge convention: B owns the closure
  resugaring path; C owns the pin marking + oxc_codegen path.
  Conflicts will be small if each writes to disjoint sections.
- **D is fully independent** — only touches cross-check sites and
  functional-equivalence tests.
- **B is the longest** — start it first if running serially after A.

## Coordination contract for parallel agents

If multiple agents work concurrently on B/C/D:

1. **Each agent works on its own branch** (`t2-stream-b`, `t2-stream-c`,
   `t2-stream-d`) branched from the post-Stream-A commit.
2. **Each commits often** — atomic commits with a clear scope tag in
   the message: `feat(t2/B): ...`, `feat(t2/C): ...`, `feat(t2/D): ...`.
3. **Each writes its tests in its own section** of `tests.rs` to
   minimize merge conflicts.
4. **Final merge** is sequential: B → C → D. C touches B's closure
   resugaring; D touches cross-check sites that B may have introduced.

## Definition of done for full T2

- All 56 existing tests pass.
- All new tests in B/C/D pass.
- `cargo clippy --all-targets -- -D warnings` clean.
- `cargo fmt --check` clean.
- `cargo test --features node-tests` passes (Node available).
- Memory file `project_t2-status.md` updated to "fully landed";
  point to the commit.
