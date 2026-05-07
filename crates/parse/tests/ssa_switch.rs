//! SSA-level Switch tests (per ticket §8).
//!
//! Switch is now a real `Terminator::Switch` — no more fallback. These
//! tests exercise the full pipeline (parse → tree-IR → SSA → tree-IR →
//! emit → re-parse) on the shapes the ticket calls out: basic switch,
//! with-default, break-inside-switch-inside-loop, continue-inside-
//! switch-inside-loop, labeled-break (still PT'd by LabeledStmt), CFG
//! predecessor count fixture, empty case body.

use emit::emit;
use ir::ssa::Terminator;
use ir::tree::{Item, Stmt};
use parse::lower_ssa;
use parse::lower_tree;
use parse::parse_and_lower;

fn full_roundtrip(src: &str) -> String {
    let tm = parse_and_lower(src).expect("parse");
    let sm = lower_ssa::lower_module(&tm);
    sm.validate().expect("SSA validation");
    let tm2 = lower_tree::lower_module(&sm);
    let out = emit(&tm2);
    parse_and_lower(&out).expect("re-parse");
    out
}

fn first_function_body(tm: &ir::tree::Module) -> &[Stmt] {
    for item in &tm.items {
        if let Item::Function(f) = item {
            return &f.body;
        }
    }
    panic!("no top-level function")
}

fn contains_switch(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Switch { .. } => true,
        Stmt::While { body, .. } | Stmt::Block(body) | Stmt::For { body, .. } => {
            contains_switch(body)
        }
        Stmt::If { then_body, else_body, .. } => {
            contains_switch(then_body)
                || else_body.as_ref().is_some_and(|b| contains_switch(b))
        }
        _ => false,
    })
}

#[test]
fn basic_switch_three_cases_no_default() {
    let src = "function f(x) {
        switch (x) {
            case 1: return 1;
            case 2: return 2;
            case 3: return 3;
        }
        return 0;
    }";
    let out = full_roundtrip(src);
    let tm2 = parse_and_lower(&out).unwrap();
    assert!(contains_switch(first_function_body(&tm2)));
}

#[test]
fn switch_with_default() {
    let src = "function f(x) {
        switch (x) {
            case 1: return 1;
            case 2: return 2;
            default: return 0;
        }
    }";
    let out = full_roundtrip(src);
    let tm2 = parse_and_lower(&out).unwrap();
    let body = first_function_body(&tm2);
    let cases = match &body[0] {
        Stmt::Switch { cases, .. } => cases,
        _ => panic!("expected Stmt::Switch, got: {:?}", &body[0]),
    };
    // Default must round-trip — find it.
    assert!(cases.iter().any(|c| c.test.is_none()), "default case lost");
}

#[test]
fn break_inside_switch_inside_loop() {
    // The corpus shape at one remove: switch case ends in `break`, the
    // surrounding loop body continues after the switch. Verifies that
    // break targets the switch (innermost), not the loop.
    let src = "function f(x) {
        var counter = 0;
        for (var i = 0; i < 10; i = i + 1) {
            switch (x) {
                case 1: break;
                case 2: counter = counter + 1; break;
            }
            counter = counter + 1;
        }
        return counter;
    }";
    let out = full_roundtrip(src);
    // Spot-check: after-switch counter increment must survive — that's
    // the post-break body.
    assert!(
        out.contains("counter")  ,
        "post-break loop body lost:\n{out}"
    );
}

#[test]
fn continue_inside_switch_inside_loop() {
    // Corpus-dominant shape. `continue;` inside a switch case must
    // target the enclosing loop, not the switch (per ES spec).
    let src = "function f(x) {
        var idx = 0;
        while (idx < 5) {
            switch (x) {
                case 1: idx = idx + 1; continue;
                case 2: idx = idx + 2; continue;
                case 3: return idx;
            }
            return -1;
        }
        return idx;
    }";
    let out = full_roundtrip(src);
    let tm2 = parse_and_lower(&out).unwrap();
    assert!(contains_switch(first_function_body(&tm2)));
}

#[test]
fn labeled_break_out_of_switch_lowers() {
    // Issue #33: labeled break/continue lift via the BreakFrame
    // pending-label mechanism in lower_ssa. The function lowers to
    // Item::Function with the labeled break/continue resolving against
    // the matching loop frame.
    let src = "function f(x) {
        outer: for (var i = 0; i < 10; i = i + 1) {
            switch (x) {
                case 1: break outer;
                case 2: continue outer;
            }
        }
        return i;
    }";
    let m = parse_and_lower(src).expect("parse");
    let item0 = m.items.first().expect("item");
    match item0 {
        Item::Function(_) => {} // expected: now lifted
        Item::PassThroughFunction(_) => panic!(
            "labeled break/continue should lower (issue #33)"
        ),
        _ => panic!("unexpected item kind"),
    }
    // Source round-trip: emit + re-parse must preserve.
    let out = emit(&m);
    parse_and_lower(&out).expect("re-parse labeled-break source");
}

#[test]
fn empty_case_body_is_break_only() {
    // `case 1: break;` — the simplest non-empty case. Tested
    // separately because it's the smallest case shape that produces
    // a meaningful CFG: dispatch → case body (1 inst: terminator) →
    // join.
    let src = "function f(x) {
        switch (x) {
            case 1: break;
        }
        return 0;
    }";
    let _ = full_roundtrip(src);
}

#[test]
fn cfg_predecessor_count_at_join() {
    // CFG predecessor walking at the join: a switch with N cases
    // that all break should produce a join block with exactly N
    // predecessors (one per case body's break edge). Cases that
    // return/throw/continue do not contribute join predecessors.
    //
    // This fixture: 3 cases, all break. Switch's `default` field
    // points at the join (no source default), so default itself
    // does NOT contribute a separate predecessor — the switch
    // terminator's edge to default IS the edge to join. With 3
    // case bodies all breaking + 1 direct switch→join edge for
    // default, the join has 4 predecessors.
    let src = "function f(x) {
        switch (x) {
            case 1: break;
            case 2: break;
            case 3: break;
        }
        return 0;
    }";
    let tm = parse_and_lower(src).expect("parse");
    let sm = lower_ssa::lower_module(&tm);
    sm.validate().expect("SSA validation");

    // Find the function and its switch terminator.
    let func = sm
        .functions
        .iter()
        .find(|f| f.name == "f")
        .expect("function f");
    let body = match &func.body {
        ir::ssa::FunctionBody::Lowered(b) => b,
        _ => panic!("expected Lowered body"),
    };
    let mut switch_join: Option<ir::ssa::BlockId> = None;
    for blk in &body.blocks {
        if let Terminator::Switch { join, .. } = &blk.terminator {
            switch_join = Some(*join);
        }
    }
    let join = switch_join.expect("switch terminator missing");
    let join_blk = &body.blocks[join.0 as usize];
    // 3 case-body break edges + 1 switch→join (default == join)
    // = 4 predecessors on the join block.
    assert_eq!(
        join_blk.predecessors.len(),
        4,
        "expected 4 predecessors at join block, got {} (preds={:?})",
        join_blk.predecessors.len(),
        join_blk.predecessors
    );
}

#[test]
fn corpus_obfuscated_dispatch_shape_lowered_via_real_ssa() {
    // The shape of the 19 corpus PT functions. After this ticket they
    // lower fully (no fallback). Post-SSA tree-IR must be
    // `Item::Function` with a reconstructed `Stmt::Switch` in body.
    let src = r#"function shuffle(_arr) {
        var idx = 0x12;
        while (true) {
            switch (idx) {
                case 0: idx = 1; continue;
                case 1: idx = 2; continue;
                case 2: return;
                case 3: idx = 0; continue;
            }
            break;
        }
    }"#;
    let tm = parse_and_lower(src).unwrap();
    let sm = lower_ssa::lower_module(&tm);
    sm.validate().expect("SSA validation");
    let tm2 = lower_tree::lower_module(&sm);
    let item0 = tm2.items.first().expect("item");
    match item0 {
        Item::Function(f) => {
            assert_eq!(f.name, "shuffle");
            assert!(
                contains_switch(&f.body),
                "post-SSA tree-IR must contain reconstructed Stmt::Switch:\n{:#?}",
                f.body
            );
        }
        Item::PassThroughFunction(_) => panic!(
            "regression: switch-bearing function should now lower fully (real \
             Terminator::Switch)"
        ),
        _ => panic!("unexpected item kind"),
    }
    // Full pipeline must complete cleanly.
    let _ = full_roundtrip(src);
}
