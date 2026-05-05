# js-deobfuscator

A JavaScript deobfuscator built as a compiler.

## Approach

Most JS deobfuscators work by recognizing specific obfuscation patterns and writing targeted rewrites. They break whenever the obfuscator changes its output.

This project takes the opposite approach: don't recognize the obfuscator, just be a strong enough optimizer that obfuscation collapses as a side effect of normal compilation. Same idea as [Mergen](https://github.com/NaC-L/Mergen) for x86 binaries — lift to an IR, run a real optimizer, lower back.

The theoretical justification is the [Futamura projection](https://en.wikipedia.org/wiki/Partial_evaluation): a sufficiently powerful partial evaluator, applied to a VM interpreter against its bytecode, recovers the original program. You don't need to know what the VM does. You just need to be a good enough specializer.

## Architecture

JS source → AST → tree IR → SSA IR → optimization passes → SSA IR → AST → JS source

- **Parser** ([oxc](https://github.com/oxc-project/oxc)-based): JS to AST.
- **IR**: typed SSA with explicit control-flow graph and effects lattice. Models JS hazards (getters, Proxies, prototype chain) explicitly rather than syntactically.
- **Pass framework**: fixpoint pass manager with validation between passes.
- **Optimization passes**: dataflow-driven, not syntax-driven. Each pass operates on semantic properties of the IR (effects, dataflow, control flow), never on the shape of the input.
- **Symbolic execution + Z3**: planned. Path-feasibility queries collapse opaque predicates and dispatcher loops generically.
- **Emitter**: SSA → AST → JS, with control-flow reconstruction (relooper).

## Why an IR

ASTs describe what code looks like. IRs describe what it means. Every serious compiler optimizer operates on an IR because:

- Syntactic variants (`while`/`for`/`do-while`, `if`/ternary, parenthesization) collapse to one canonical form.
- Control flow becomes an explicit graph instead of being implicit in tree shape.
- SSA makes "what value does this use refer to" a pointer lookup instead of a tree walk.
- Effects are explicit per-instruction instead of inferred from syntax every time.
- Passes compose cleanly in a fixpoint loop instead of fighting context-sensitivity.

Optimizing on the AST means rebuilding most of an IR's infrastructure ad hoc, less cleanly, while still carrying the AST around. Every serious deobfuscator on the durable-architecture path eventually builds an IR; this project starts with one.

## Build

```
cargo build --workspace
cargo test --workspace
cargo run -p cli -- input.js -o output.js
```
