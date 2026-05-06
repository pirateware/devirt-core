//! Switch + break lowering tests (per ticket §7).
//!
//! Switch-containing functions take the SSA-fallback path (matching the
//! Throw/Try/ForIn precedent), so the tests exercise:
//!   - tree-IR shape after `parse_and_lower` (Stmt::Switch / Stmt::Break)
//!   - emit → re-parse roundtrip preserves the switch shape
//!   - SSA pipeline routes the function to PassThrough verbatim
//!   - common control-flow shapes from the obfuscation corpus
//!     (continue-in-switch-in-loop, break-out-of-while, labeled break).

use emit::emit;
use ir::tree::{FuncBody, Item, Stmt, SwitchCase};
use parse::lower_ssa;
use parse::lower_tree;
use parse::parse_and_lower;

fn first_fn_body(src: &str) -> Vec<Stmt> {
    let m = parse_and_lower(src).expect("parse_and_lower");
    for item in m.items {
        if let Item::Function(f) = item {
            return f.body;
        }
    }
    panic!("no top-level function");
}

fn ssa_roundtrip_emit(src: &str) -> String {
    let tm = parse_and_lower(src).expect("parse");
    let sm = lower_ssa::lower_module(&tm);
    sm.validate().expect("SSA validation");
    let tm2 = lower_tree::lower_module(&sm);
    let out = emit(&tm2);
    parse_and_lower(&out).expect("re-parse");
    out
}

#[test]
fn switch_three_cases_plus_default_roundtrips() {
    let src = "function f(x) {
        switch (x) {
            case 1: return 1;
            case 2: return 2;
            case 3: return 3;
            default: return 0;
        }
    }";
    let body = first_fn_body(src);
    match &body[0] {
        Stmt::Switch { cases, .. } => {
            assert_eq!(cases.len(), 4);
            assert!(cases[0].test.is_some());
            assert!(cases[3].test.is_none(), "last case should be default");
        }
        other => panic!("expected Stmt::Switch, got {other:?}"),
    }

    // Round-trip via emit (no SSA): preserves Stmt::Switch.
    let out = emit(&parse_and_lower(src).unwrap());
    let body2 = first_fn_body(&out);
    assert!(matches!(body2[0], Stmt::Switch { .. }));

    // Full SSA pipeline: switch-bearing function routes through SSA-fallback,
    // so the post-SSA tree-IR sees it as PassThroughFunction. Re-parse must succeed.
    let _ = ssa_roundtrip_emit(src);
}

#[test]
fn fallthrough_case_parses_into_tree_ir() {
    // A switch whose first case has no body falls through to the next case.
    // The tree-IR captures this shape; SSA lowering of fallthrough is
    // deferred per the SSA-Switch ticket §6 (corpus has 0 fallthrough).
    // We verify only the parse → tree-IR shape here; the full SSA
    // round-trip will panic with "case body did not terminate" until
    // fallthrough lowering lands.
    let src = "function f(x) {
        switch (x) {
            case 1:
            case 2:
                foo();
                break;
            default:
                bar();
        }
    }";
    let body = first_fn_body(src);
    let cases = match &body[0] {
        Stmt::Switch { cases, .. } => cases,
        _ => panic!("expected Stmt::Switch"),
    };
    assert_eq!(cases.len(), 3);
    assert!(cases[0].body.is_empty(), "case 1 body must be empty (fallthrough)");
    assert!(matches!(cases[1].body.last(), Some(Stmt::Break { .. })));
    assert!(cases[2].test.is_none());
}

#[test]
fn break_inside_switch_parses() {
    let src = "function f(x) {
        switch (x) {
            case 1: break;
        }
    }";
    let body = first_fn_body(src);
    let cases = match &body[0] {
        Stmt::Switch { cases, .. } => cases,
        _ => panic!("expected Stmt::Switch"),
    };
    assert!(matches!(cases[0].body[0], Stmt::Break { label: None }));
    let _ = ssa_roundtrip_emit(src);
}

#[test]
fn break_inside_switch_inside_loop_parses() {
    // The break lexically targets the switch; the loop body continues
    // after the switch. The target-stack semantics live in the SSA
    // layer (which we punt to PassThrough); here we verify the
    // syntactic representation round-trips.
    let src = "function f(x) {
        for (let i = 0; i < 10; i++) {
            switch (x) {
                case 1: break;
            }
            body();
        }
    }";
    let _ = ssa_roundtrip_emit(src);
}

#[test]
fn continue_inside_switch_inside_loop_parses() {
    // `continue` inside a switch nested in a loop targets the loop —
    // not the switch — per ECMAScript spec. The corpus uses this shape
    // heavily (obfuscated dispatch tables: case A: idx = X; continue;).
    let src = "function f(x) {
        while (true) {
            switch (x) {
                case 1: continue;
                case 2: continue;
            }
            break;
        }
    }";
    let body = first_fn_body(src);
    let while_body = match &body[0] {
        Stmt::While { body, .. } => body,
        _ => panic!("expected Stmt::While"),
    };
    let cases = match &while_body[0] {
        Stmt::Switch { cases, .. } => cases,
        _ => panic!("expected Stmt::Switch as first while-body stmt"),
    };
    assert!(matches!(cases[0].body.last(), Some(Stmt::Continue)));
    assert!(matches!(while_body.last(), Some(Stmt::Break { label: None })));
    let _ = ssa_roundtrip_emit(src);
}

#[test]
fn labeled_break_parses_and_roundtrips() {
    let src = "function f(x) {
        outer: for (let i = 0; i < 10; i++) {
            switch (x) {
                case 1: break outer;
            }
        }
    }";
    // Labeled statements are still a blocker (LabeledStmt) — confirm the
    // function passes through but `break outer;` is parsed correctly when
    // we emit verbatim. Since LabeledStmt forces PassThroughFunction at
    // disposition, we only verify the source round-trips through emit.
    let m = parse_and_lower(src).expect("parse");
    let out = emit(&m);
    parse_and_lower(&out).expect("labeled-break round-trip");
}

#[test]
fn default_at_start_middle_and_end_round_trip() {
    let cases = [
        // default first
        "function f(x) { switch (x) { default: return 0; case 1: return 1; } }",
        // default in middle
        "function f(x) { switch (x) { case 1: return 1; default: return 0; case 2: return 2; } }",
        // default last
        "function f(x) { switch (x) { case 1: return 1; default: return 0; } }",
    ];
    let expected_default_idx = [0usize, 1, 1];
    for (src, &exp_idx) in cases.iter().zip(expected_default_idx.iter()) {
        let body = first_fn_body(src);
        let switch_cases: &Vec<SwitchCase> = match &body[0] {
            Stmt::Switch { cases, .. } => cases,
            _ => panic!("expected Stmt::Switch in {src}"),
        };
        let actual_idx = switch_cases
            .iter()
            .position(|c| c.test.is_none())
            .expect("must have default");
        assert_eq!(actual_idx, exp_idx, "default position for {src}");
        let _ = ssa_roundtrip_emit(src);
    }
}

#[test]
fn corpus_obfuscated_dispatch_shape_round_trips() {
    // Mirrors the actual shape of the 19 PT functions cleared by this
    // ticket (lodash_debounce.high.js / datefns_addMonths.high.js / …).
    let src = r#"function shuffle(_arr) {
        var idx = 0x12;
        while (true) {
            switch (idx) {
                case 0:
                    work();
                    idx = 1;
                    continue;
                case 1:
                    work();
                    idx = 2;
                    continue;
                case 2:
                    return;
                case 3:
                    work();
                    idx = 0;
                    continue;
            }
            break;
        }
    }"#;
    let _ = ssa_roundtrip_emit(src);

    // Confirm the function is no longer PT at AST disposition: the
    // initial parse should classify it as `Item::Function`.
    let m = parse_and_lower(src).expect("parse");
    let item0 = m.items.first().expect("item");
    match item0 {
        Item::Function(f) => assert_eq!(f.name, "shuffle"),
        Item::PassThroughFunction(lit) => panic!(
            "regression: switch-bearing function should now be Lowered at disposition, \
             got PassThroughFunction with blockers {:?}",
            lit.blockers
        ),
        _ => panic!("unexpected item kind"),
    }

    // After the SSA pipeline, switch-bearing functions now go through
    // real SSA (Terminator::Switch + break-target stack). Post-SSA
    // tree-IR is `Item::Function` with a reconstructed Stmt::Switch in
    // the body — not the source slice. Verify the structure.
    let tm = parse_and_lower(src).unwrap();
    let sm = lower_ssa::lower_module(&tm);
    let tm2 = lower_tree::lower_module(&sm);
    let item0 = tm2.items.first().expect("item");
    match item0 {
        Item::Function(f) => {
            assert_eq!(f.name, "shuffle");
            // Body should contain a reconstructed Stmt::Switch
            // somewhere (inside the while body).
            let any_switch = contains_switch(&f.body);
            assert!(
                any_switch,
                "post-SSA tree-IR should contain reconstructed Stmt::Switch:\n{:#?}",
                f.body
            );
        }
        Item::PassThroughFunction(_) => panic!(
            "post-SSA: switch-bearing function should now lower fully (real \
             Terminator::Switch); regressed to PassThrough?"
        ),
        _ => panic!("unexpected item kind"),
    }
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
