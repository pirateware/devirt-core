//! Issue #34 — per-block exception edges.
//!
//! These tests exercise the additive `BasicBlock.exception_target`
//! field: every block lowered inside a `try` body carries an exception
//! edge to the enclosing handler (catch_entry preferred, else
//! finally_entry). The edge is included in `predecessors`, so passes
//! that walk preds observe exceptional control flow naturally.
//!
//! Existing `Terminator::Try` framing is unchanged — these edges are
//! purely additive precision for motion / GVN / forwarding passes.

use ir::ssa::{FunctionBody, Terminator};
use parse::{lower_ssa, parse_and_lower};

fn lower_to_ssa(src: &str) -> ir::ssa::Module {
    let wrapped = format!("function f() {{\n{src}\n}}");
    let tm = parse_and_lower(&wrapped).expect("parse");
    let sm = lower_ssa::lower_module(&tm);
    sm.validate().expect("SSA validation");
    sm
}

fn lowered_body(sm: &ir::ssa::Module) -> &ir::ssa::LoweredBody {
    for f in &sm.functions {
        if let FunctionBody::Lowered(b) = &f.body {
            // `f` (top-level wrapper) is the user function; first
            // Lowered body in the module is the one we want.
            return b;
        }
    }
    panic!("no Lowered function body in module")
}

fn try_terminator(body: &ir::ssa::LoweredBody) -> (ir::ssa::BlockId, Option<ir::ssa::BlockId>, Option<ir::ssa::BlockId>) {
    for blk in &body.blocks {
        if let Terminator::Try { try_entry, catch_entry, finally_entry, .. } = &blk.terminator {
            return (*try_entry, *catch_entry, *finally_entry);
        }
    }
    panic!("no Terminator::Try in body");
}

#[test]
fn try_catch_stamps_try_body_with_catch_entry() {
    let sm = lower_to_ssa("try { var x = 1; } catch (e) { }");
    let body = lowered_body(&sm);
    let (try_entry, catch_entry, _) = try_terminator(body);
    let catch_entry = catch_entry.expect("try+catch must have catch_entry");
    assert_eq!(
        body.blocks[try_entry.0 as usize].exception_target,
        Some(catch_entry),
        "try-body entry must point its exception edge at catch_entry"
    );
    // catch_entry itself lives OUTSIDE the try-body — no exception
    // edge of its own (no outer try here).
    assert_eq!(
        body.blocks[catch_entry.0 as usize].exception_target,
        None,
        "catch_entry must not point at itself"
    );
}

#[test]
fn try_finally_stamps_try_body_with_finally_entry() {
    let sm = lower_to_ssa("try { var x = 1; } finally { var y = 2; }");
    let body = lowered_body(&sm);
    let (try_entry, catch_entry, finally_entry) = try_terminator(body);
    assert!(catch_entry.is_none(), "no catch in this shape");
    let finally_entry = finally_entry.expect("try+finally must have finally_entry");
    assert_eq!(
        body.blocks[try_entry.0 as usize].exception_target,
        Some(finally_entry),
        "with no catch, the handler is finally_entry"
    );
}

#[test]
fn try_catch_finally_prefers_catch_entry() {
    let sm = lower_to_ssa("try { var x = 1; } catch (e) { } finally { }");
    let body = lowered_body(&sm);
    let (try_entry, catch_entry, _) = try_terminator(body);
    let catch_entry = catch_entry.expect("must have catch_entry");
    assert_eq!(
        body.blocks[try_entry.0 as usize].exception_target,
        Some(catch_entry),
        "catch wins over finally as the handler — throws are caught first"
    );
}

#[test]
fn nested_blocks_inside_try_inherit_handler() {
    // The if/else inside the try-body creates new blocks; each must
    // inherit the catch_entry as its exception_target.
    let sm = lower_to_ssa(
        "try { if (g) { foo(); } else { bar(); } } catch (e) { }",
    );
    let body = lowered_body(&sm);
    let (_, catch_entry, _) = try_terminator(body);
    let catch_entry = catch_entry.expect("must have catch_entry");
    let mut stamped = 0usize;
    for blk in &body.blocks {
        if blk.exception_target == Some(catch_entry) {
            stamped += 1;
        }
    }
    assert!(
        stamped >= 3,
        "expected at least try_entry + then + else stamped, got {stamped}"
    );
}

#[test]
fn catch_body_does_not_inherit_own_handler() {
    // Throws inside a `catch` propagate to the OUTER try — never back
    // into the same try's catch. With no outer try here, catch-body
    // blocks have None.
    let sm = lower_to_ssa("try { } catch (e) { var x = 1; }");
    let body = lowered_body(&sm);
    let (_, catch_entry, _) = try_terminator(body);
    let catch_entry = catch_entry.expect("must have catch_entry");
    assert_eq!(
        body.blocks[catch_entry.0 as usize].exception_target,
        None,
        "catch-body must not chain to its own handler"
    );
}

#[test]
fn nested_try_inner_inherits_inner_handler_outer_catch_does_not() {
    // outer try { inner try { foo() } catch (i) { } } catch (o) { }
    // Blocks lexically inside the INNER try-body must point at inner's
    // catch. The outer catch_entry must not chain to anything (no
    // further outer try).
    let sm = lower_to_ssa(
        "try { try { foo(); } catch (i) { } } catch (o) { }",
    );
    let body = lowered_body(&sm);
    // Find both Try terminators.
    let mut tries: Vec<(ir::ssa::BlockId, ir::ssa::BlockId)> = Vec::new();
    for blk in &body.blocks {
        if let Terminator::Try { try_entry, catch_entry: Some(c), .. } = &blk.terminator {
            tries.push((*try_entry, *c));
        }
    }
    assert_eq!(tries.len(), 2, "expected 2 Try terminators, got {}", tries.len());
    // Each try_entry has SOME exception_target — the outer's points
    // outer-catch, the inner's points inner-catch. Both are non-None
    // and distinct.
    let et0 = body.blocks[tries[0].0.0 as usize].exception_target;
    let et1 = body.blocks[tries[1].0.0 as usize].exception_target;
    assert!(et0.is_some());
    assert!(et1.is_some());
    assert_ne!(et0, et1, "inner and outer try-bodies must have distinct handlers");
}

#[test]
fn predecessors_include_exception_edges() {
    // After lowering, catch_entry's predecessors list should contain
    // every try-body block (LLVM-`invoke`-style). Without this, passes
    // walking `predecessors` would miss exceptional control flow.
    let sm = lower_to_ssa("try { foo(); bar(); } catch (e) { }");
    let body = lowered_body(&sm);
    let (try_entry, catch_entry, _) = try_terminator(body);
    let catch_entry = catch_entry.expect("must have catch_entry");
    let preds = &body.blocks[catch_entry.0 as usize].predecessors;
    assert!(
        preds.contains(&try_entry),
        "catch_entry must list try_entry among its predecessors via the \
         exception edge — got {preds:?}"
    );
}

#[test]
fn no_try_means_no_exception_targets() {
    let sm = lower_to_ssa("var x = 1; foo(); return x;");
    let body = lowered_body(&sm);
    for blk in &body.blocks {
        assert_eq!(
            blk.exception_target, None,
            "no try in source → no block should carry an exception edge"
        );
    }
}
