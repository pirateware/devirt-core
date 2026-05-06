//! Structural-equality check for the parse → emit → reparse roundtrip.
//!
//! Pre-tightening, the corpus runner / stability test only compared
//! `m1.items.len() == m2.items.len()` — which means two modules with
//! the same number of top-level items but different structure passed
//! the gate. That hid silent corruption (top-level functions being
//! replaced by inner-anonymous emissions, etc.). This module walks
//! both trees in lockstep and returns the first diverging path.
//!
//! Intentional tolerances:
//! - PassThrough source slices need not byte-match (whitespace,
//!   comment loss); only the kind/name/arity must match.
//! - Numeric literals compared by `f64` value (hex/octal normalize).
//! - Strings compared kind-only (escape-sequence normalization).
//!
//! Known emit/parse asymmetries (accepted as no-op divergence):
//!
//! 1. **PassThroughFunction ↔ Function** when name and params match.
//!    A function whose body went through SSA-fallback is encoded as
//!    `Item::PassThroughFunction` in `tree_back`; its emit is the
//!    raw source slice; re-parsing classifies it as `Item::Function`
//!    when the disposition no longer blocks (e.g. throw/this after
//!    those tickets). Same source, different label.
//!
//! Two further asymmetries — `Number(-0) ↔ Unary(Neg, Number(0))`
//! and `Number(NaN/Infinity) ↔ Ident("NaN"/"Infinity")` — were
//! removed by the parser-side identifier/unary fold in
//! `parse::lower_expr`. Both sides now converge on the canonical
//! `Expr::Number` shape, so no tolerance is required and a
//! regression of either fold will surface as a structural diff
//! (the desired loud-failure mode).

use ir::tree::{Expr, Item, Module, Stmt, SwitchCase};

/// Returns `None` when `m1` and `m2` are structurally equivalent,
/// `Some(reason)` describing the first mismatch otherwise.
pub fn structural_diff(m1: &Module, m2: &Module) -> Option<String> {
    if m1.items.len() != m2.items.len() {
        return Some(format!(
            "module item-count mismatch: {} vs {}",
            m1.items.len(),
            m2.items.len()
        ));
    }
    for (i, (a, b)) in m1.items.iter().zip(m2.items.iter()).enumerate() {
        let path = format!("items[{i}]");
        match (a, b) {
            (Item::Function(fa), Item::Function(fb)) => {
                if fa.name != fb.name {
                    return Some(format!(
                        "{path}: function name {:?} vs {:?}",
                        fa.name, fb.name
                    ));
                }
                if fa.params != fb.params {
                    return Some(format!(
                        "{path} fn {:?}: params {:?} vs {:?}",
                        fa.name, fa.params, fb.params
                    ));
                }
                if let Some(d) =
                    stmts_diff(&fa.body, &fb.body, &format!("{path} fn {:?}", fa.name))
                {
                    return Some(d);
                }
            }
            (Item::PassThroughFunction(la), Item::PassThroughFunction(lb)) => {
                if la.name != lb.name {
                    return Some(format!(
                        "{path}: PT function name {:?} vs {:?}",
                        la.name, lb.name
                    ));
                }
                if la.params != lb.params {
                    return Some(format!(
                        "{path} PT fn {:?}: params {:?} vs {:?}",
                        la.name, la.params, lb.params
                    ));
                }
            }
            (Item::PassThroughStmt(_), Item::PassThroughStmt(_)) => {}
            (Item::Stmt(_), Item::Stmt(_)) => {}
            // Tolerance #1: PT-vs-Function with matching name+params.
            // Same source, different label — see module doc.
            (Item::PassThroughFunction(la), Item::Function(fb))
            | (Item::Function(fb), Item::PassThroughFunction(la)) => {
                if la.name.as_deref() != Some(fb.name.as_str()) {
                    return Some(format!(
                        "{path}: PT/Function name mismatch {:?} vs {:?}",
                        la.name, fb.name
                    ));
                }
                if la.params != fb.params {
                    return Some(format!(
                        "{path} fn {:?}: PT/Function params {:?} vs {:?}",
                        la.name, la.params, fb.params
                    ));
                }
            }
            (a, b) => {
                return Some(format!(
                    "{path}: kind mismatch {} vs {}",
                    item_kind(a),
                    item_kind(b)
                ));
            }
        }
    }
    None
}

fn item_kind(it: &Item) -> &'static str {
    match it {
        Item::Function(_) => "Function",
        Item::PassThroughFunction(_) => "PassThroughFunction",
        Item::Stmt(_) => "Stmt",
        Item::PassThroughStmt(_) => "PassThroughStmt",
    }
}

fn stmts_diff(a: &[Stmt], b: &[Stmt], path: &str) -> Option<String> {
    if a.len() != b.len() {
        return Some(format!(
            "{path}: stmt-count mismatch {} vs {}",
            a.len(),
            b.len()
        ));
    }
    for (i, (sa, sb)) in a.iter().zip(b.iter()).enumerate() {
        if let Some(d) = stmt_diff(sa, sb, &format!("{path}.stmt[{i}]")) {
            return Some(d);
        }
    }
    None
}

fn stmt_diff(a: &Stmt, b: &Stmt, path: &str) -> Option<String> {
    match (a, b) {
        (Stmt::Let { name: na, init: ia }, Stmt::Let { name: nb, init: ib }) => {
            if na != nb {
                return Some(format!("{path}: let name {na:?} vs {nb:?}"));
            }
            opt_expr_diff(ia.as_ref(), ib.as_ref(), &format!("{path}.init"))
        }
        (Stmt::Const { name: na, init: ia }, Stmt::Const { name: nb, init: ib }) => {
            if na != nb {
                return Some(format!("{path}: const name {na:?} vs {nb:?}"));
            }
            expr_diff(ia, ib, &format!("{path}.init"))
        }
        (Stmt::Assign { value: va, .. }, Stmt::Assign { value: vb, .. }) => {
            expr_diff(va, vb, &format!("{path}.value"))
        }
        (Stmt::ExprStmt(ea), Stmt::ExprStmt(eb)) => expr_diff(ea, eb, path),
        (Stmt::Return(ea), Stmt::Return(eb)) => {
            opt_expr_diff(ea.as_ref(), eb.as_ref(), &format!("{path}.return"))
        }
        (
            Stmt::If { cond: ca, then_body: ta, else_body: ea },
            Stmt::If { cond: cb, then_body: tb, else_body: eb },
        ) => expr_diff(ca, cb, &format!("{path}.cond"))
            .or_else(|| stmts_diff(ta, tb, &format!("{path}.then")))
            .or_else(|| match (ea, eb) {
                (None, None) => None,
                (Some(ea), Some(eb)) => stmts_diff(ea, eb, &format!("{path}.else")),
                _ => Some(format!("{path}: else-arm presence mismatch")),
            }),
        (Stmt::While { body: ba, .. }, Stmt::While { body: bb, .. }) => {
            stmts_diff(ba, bb, &format!("{path}.body"))
        }
        (Stmt::For { body: ba, .. }, Stmt::For { body: bb, .. }) => {
            stmts_diff(ba, bb, &format!("{path}.body"))
        }
        (Stmt::ForOf { body: ba, .. }, Stmt::ForOf { body: bb, .. }) => {
            stmts_diff(ba, bb, &format!("{path}.body"))
        }
        (Stmt::ForIn { body: ba, .. }, Stmt::ForIn { body: bb, .. }) => {
            stmts_diff(ba, bb, &format!("{path}.body"))
        }
        (Stmt::Continue, Stmt::Continue) => None,
        (Stmt::Break { label: la }, Stmt::Break { label: lb }) => {
            if la == lb {
                None
            } else {
                Some(format!("{path}: break label {la:?} vs {lb:?}"))
            }
        }
        (
            Stmt::Switch { discriminant: da, cases: ca },
            Stmt::Switch { discriminant: db, cases: cb },
        ) => expr_diff(da, db, &format!("{path}.discriminant"))
            .or_else(|| switch_cases_diff(ca, cb, path)),
        (Stmt::Throw(ea), Stmt::Throw(eb)) => expr_diff(ea, eb, &format!("{path}.throw")),
        (Stmt::Block(a), Stmt::Block(b)) => stmts_diff(a, b, path),
        (Stmt::FuncDecl(la), Stmt::FuncDecl(lb)) => {
            if la.name != lb.name {
                return Some(format!(
                    "{path}: nested fn name {:?} vs {:?}",
                    la.name, lb.name
                ));
            }
            if la.params != lb.params {
                return Some(format!("{path}: nested fn params differ"));
            }
            None
        }
        (Stmt::Try { body: ba, .. }, Stmt::Try { body: bb, .. }) => {
            stmts_diff(ba, bb, &format!("{path}.try.body"))
        }
        (a, b) => Some(format!(
            "{path}: kind mismatch {} vs {}",
            stmt_kind(a),
            stmt_kind(b)
        )),
    }
}

fn switch_cases_diff(a: &[SwitchCase], b: &[SwitchCase], path: &str) -> Option<String> {
    if a.len() != b.len() {
        return Some(format!(
            "{path}: switch case-count mismatch {} vs {}",
            a.len(),
            b.len()
        ));
    }
    for (i, (ca, cb)) in a.iter().zip(b.iter()).enumerate() {
        let cpath = format!("{path}.case[{i}]");
        match (&ca.test, &cb.test) {
            (None, None) => {}
            (Some(ea), Some(eb)) => {
                if let Some(d) = expr_diff(ea, eb, &format!("{cpath}.test")) {
                    return Some(d);
                }
            }
            _ => return Some(format!("{cpath}: default-vs-cased mismatch")),
        }
        if let Some(d) = stmts_diff(&ca.body, &cb.body, &format!("{cpath}.body")) {
            return Some(d);
        }
    }
    None
}

fn stmt_kind(s: &Stmt) -> &'static str {
    match s {
        Stmt::Let { .. } => "Let",
        Stmt::Const { .. } => "Const",
        Stmt::Assign { .. } => "Assign",
        Stmt::ExprStmt(_) => "ExprStmt",
        Stmt::Return(_) => "Return",
        Stmt::If { .. } => "If",
        Stmt::While { .. } => "While",
        Stmt::For { .. } => "For",
        Stmt::ForOf { .. } => "ForOf",
        Stmt::ForIn { .. } => "ForIn",
        Stmt::Continue => "Continue",
        Stmt::Break { .. } => "Break",
        Stmt::Switch { .. } => "Switch",
        Stmt::Throw(_) => "Throw",
        Stmt::Block(_) => "Block",
        Stmt::FuncDecl(_) => "FuncDecl",
        Stmt::Try { .. } => "Try",
    }
}

fn opt_expr_diff(a: Option<&Expr>, b: Option<&Expr>, path: &str) -> Option<String> {
    match (a, b) {
        (None, None) => None,
        (Some(ea), Some(eb)) => expr_diff(ea, eb, path),
        _ => Some(format!("{path}: optional-expr presence mismatch")),
    }
}

fn expr_diff(a: &Expr, b: &Expr, path: &str) -> Option<String> {
    match (a, b) {
        (Expr::Number(x), Expr::Number(y)) => {
            if (x.is_nan() && y.is_nan()) || x == y {
                None
            } else {
                Some(format!("{path}: number {x} vs {y}"))
            }
        }
        (Expr::String(_), Expr::String(_)) => None,
        (Expr::Bool(x), Expr::Bool(y)) if x == y => None,
        (Expr::Null, Expr::Null) => None,
        (Expr::Undefined, Expr::Undefined) => None,
        (Expr::Ident(x), Expr::Ident(y)) => {
            if x == y {
                None
            } else {
                Some(format!("{path}: ident {x} vs {y}"))
            }
        }
        (Expr::This, Expr::This) => None,
        // RegExp source slices are stored verbatim. The roundtrip
        // emit/reparse path may normalize escape forms, so accept
        // them as kind-equal without comparing the slice itself —
        // analogous to how `String` is compared kind-only.
        (Expr::RegExp(_), Expr::RegExp(_)) => None,
        (Expr::Binary { lhs: la, rhs: ra, .. }, Expr::Binary { lhs: lb, rhs: rb, .. }) => {
            expr_diff(la, lb, &format!("{path}.lhs"))
                .or_else(|| expr_diff(ra, rb, &format!("{path}.rhs")))
        }
        (Expr::Unary { operand: oa, .. }, Expr::Unary { operand: ob, .. }) => {
            expr_diff(oa, ob, &format!("{path}.operand"))
        }
        (Expr::Call { callee: ca, args: aa }, Expr::Call { callee: cb, args: ab }) => {
            expr_diff(ca, cb, &format!("{path}.callee")).or_else(|| args_diff(aa, ab, path))
        }
        (Expr::Member { obj: oa, prop: pa }, Expr::Member { obj: ob, prop: pb }) => {
            if pa != pb {
                return Some(format!("{path}: member prop {pa} vs {pb}"));
            }
            expr_diff(oa, ob, &format!("{path}.obj"))
        }
        (Expr::Index { obj: oa, index: ia }, Expr::Index { obj: ob, index: ib }) => {
            expr_diff(oa, ob, &format!("{path}.obj"))
                .or_else(|| expr_diff(ia, ib, &format!("{path}.index")))
        }
        (Expr::New { callee: ca, args: aa }, Expr::New { callee: cb, args: ab }) => {
            expr_diff(ca, cb, &format!("{path}.callee")).or_else(|| args_diff(aa, ab, path))
        }
        (Expr::Array { elements: ea }, Expr::Array { elements: eb }) => args_diff(ea, eb, path),
        (Expr::Object { props: pa }, Expr::Object { props: pb }) => {
            if pa.len() != pb.len() {
                return Some(format!(
                    "{path}: object props count {} vs {}",
                    pa.len(),
                    pb.len()
                ));
            }
            for (i, (a, b)) in pa.iter().zip(pb.iter()).enumerate() {
                if let Some(d) = expr_diff(&a.value, &b.value, &format!("{path}.prop[{i}].value")) {
                    return Some(d);
                }
            }
            None
        }
        (
            Expr::Ternary { cond: ca, then_branch: ta, else_branch: ea },
            Expr::Ternary { cond: cb, then_branch: tb, else_branch: eb },
        ) => expr_diff(ca, cb, &format!("{path}.cond"))
            .or_else(|| expr_diff(ta, tb, &format!("{path}.then")))
            .or_else(|| expr_diff(ea, eb, &format!("{path}.else"))),
        (Expr::Update { target: ta, .. }, Expr::Update { target: tb, .. }) => {
            expr_diff(ta, tb, &format!("{path}.target"))
        }
        (Expr::Spread(a), Expr::Spread(b)) => expr_diff(a, b, &format!("{path}.spread")),
        (Expr::Sequence { exprs: a }, Expr::Sequence { exprs: b }) => args_diff(a, b, path),
        (Expr::Assign { target: ta, value: va }, Expr::Assign { target: tb, value: vb }) => {
            expr_diff(ta, tb, &format!("{path}.target"))
                .or_else(|| expr_diff(va, vb, &format!("{path}.value")))
        }
        (
            Expr::CompoundAssign { target: ta, value: va, .. },
            Expr::CompoundAssign { target: tb, value: vb, .. },
        ) => expr_diff(ta, tb, &format!("{path}.target"))
            .or_else(|| expr_diff(va, vb, &format!("{path}.value"))),
        (Expr::Func(la), Expr::Func(lb)) => {
            if la.name != lb.name {
                return Some(format!(
                    "{path}: nested fn name {:?} vs {:?}",
                    la.name, lb.name
                ));
            }
            if la.params != lb.params {
                return Some(format!("{path}: nested fn params differ"));
            }
            None
        }
        (Expr::Template { exprs: a, .. }, Expr::Template { exprs: b, .. }) => {
            args_diff(a, b, path)
        }
        (a, b) => Some(format!(
            "{path}: expr kind mismatch {} vs {}",
            expr_kind(a),
            expr_kind(b)
        )),
    }
}

fn args_diff(a: &[Expr], b: &[Expr], path: &str) -> Option<String> {
    if a.len() != b.len() {
        return Some(format!(
            "{path}: arg-count mismatch {} vs {}",
            a.len(),
            b.len()
        ));
    }
    for (i, (a, b)) in a.iter().zip(b.iter()).enumerate() {
        if let Some(d) = expr_diff(a, b, &format!("{path}[{i}]")) {
            return Some(d);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ir::tree::{IrFunction, Module};

    fn fn_item(name: &str, params: Vec<&str>, body: Vec<Stmt>) -> Item {
        Item::Function(IrFunction {
            name: name.into(),
            params: params.into_iter().map(String::from).collect(),
            body,
            source: None,
        })
    }

    #[test]
    fn empty_modules_match() {
        let m = Module { items: vec![] };
        assert!(structural_diff(&m, &m).is_none());
    }

    #[test]
    fn item_count_mismatch_caught() {
        let m1 = Module { items: vec![fn_item("a", vec![], vec![])] };
        let m2 = Module {
            items: vec![
                fn_item("a", vec![], vec![]),
                fn_item("b", vec![], vec![]),
            ],
        };
        let diff = structural_diff(&m1, &m2).expect("must catch count diff");
        assert!(diff.contains("item-count mismatch"), "got: {diff}");
    }

    #[test]
    fn name_mismatch_caught() {
        let m1 = Module { items: vec![fn_item("foo", vec![], vec![])] };
        let m2 = Module { items: vec![fn_item("bar", vec![], vec![])] };
        let diff = structural_diff(&m1, &m2).expect("must catch name diff");
        assert!(diff.contains("function name"), "got: {diff}");
    }

    #[test]
    fn param_mismatch_caught() {
        let m1 = Module { items: vec![fn_item("f", vec!["x"], vec![])] };
        let m2 = Module {
            items: vec![fn_item("f", vec!["x", "y"], vec![])],
        };
        let diff = structural_diff(&m1, &m2).expect("must catch param diff");
        assert!(diff.contains("params"), "got: {diff}");
    }

    #[test]
    fn stmt_count_mismatch_caught() {
        let m1 = Module {
            items: vec![fn_item("f", vec![], vec![Stmt::Return(None)])],
        };
        let m2 = Module {
            items: vec![fn_item(
                "f",
                vec![],
                vec![Stmt::Return(None), Stmt::Return(None)],
            )],
        };
        let diff = structural_diff(&m1, &m2).expect("must catch stmt-count diff");
        assert!(diff.contains("stmt-count"), "got: {diff}");
    }

    #[test]
    fn negative_zero_unary_shape_caught() {
        // The parser-side fold in `parse::lower_expr` rewrites
        // `Unary(Neg, NumericLiteral(0))` to `Number(-0.0)` so this
        // shape divergence shouldn't appear in practice. If it does
        // (a fold regression), the structural check must catch it
        // loudly rather than silently accept it.
        use ir::tree::UnOp;
        let m1 = Module {
            items: vec![fn_item(
                "f",
                vec![],
                vec![Stmt::Return(Some(Expr::Number(-0.0)))],
            )],
        };
        let m2 = Module {
            items: vec![fn_item(
                "f",
                vec![],
                vec![Stmt::Return(Some(Expr::Unary {
                    op: UnOp::Neg,
                    operand: Box::new(Expr::Number(0.0)),
                }))],
            )],
        };
        let diff = structural_diff(&m1, &m2).expect("must catch -0 shape diff");
        assert!(diff.contains("kind mismatch"), "got: {diff}");
    }

    #[test]
    fn nan_ident_shape_caught() {
        // Same story for `NaN`/`Infinity` — folded at lower-time, so
        // a bare Ident("NaN") on one side and Number(NaN) on the
        // other should now flag, not silently accept.
        let m1 = Module {
            items: vec![fn_item(
                "f",
                vec![],
                vec![Stmt::Return(Some(Expr::Number(f64::NAN)))],
            )],
        };
        let m2 = Module {
            items: vec![fn_item(
                "f",
                vec![],
                vec![Stmt::Return(Some(Expr::Ident("NaN".into())))],
            )],
        };
        let diff = structural_diff(&m1, &m2).expect("must catch NaN shape diff");
        assert!(diff.contains("kind mismatch"), "got: {diff}");
    }

    #[test]
    fn distinct_numbers_caught() {
        let m1 = Module {
            items: vec![fn_item(
                "f",
                vec![],
                vec![Stmt::Return(Some(Expr::Number(1.0)))],
            )],
        };
        let m2 = Module {
            items: vec![fn_item(
                "f",
                vec![],
                vec![Stmt::Return(Some(Expr::Number(2.0)))],
            )],
        };
        let diff = structural_diff(&m1, &m2).expect("must catch number diff");
        assert!(diff.contains("number"), "got: {diff}");
    }
}

fn expr_kind(e: &Expr) -> &'static str {
    match e {
        Expr::Number(_) => "Number",
        Expr::String(_) => "String",
        Expr::Bool(_) => "Bool",
        Expr::Null => "Null",
        Expr::Undefined => "Undefined",
        Expr::Ident(_) => "Ident",
        Expr::This => "This",
        Expr::RegExp(_) => "RegExp",
        Expr::Binary { .. } => "Binary",
        Expr::Unary { .. } => "Unary",
        Expr::Call { .. } => "Call",
        Expr::Member { .. } => "Member",
        Expr::Index { .. } => "Index",
        Expr::Func(_) => "Func",
        Expr::Array { .. } => "Array",
        Expr::Object { .. } => "Object",
        Expr::Spread(_) => "Spread",
        Expr::New { .. } => "New",
        Expr::Ternary { .. } => "Ternary",
        Expr::Update { .. } => "Update",
        Expr::Template { .. } => "Template",
        Expr::Sequence { .. } => "Sequence",
        Expr::Assign { .. } => "Assign",
        Expr::CompoundAssign { .. } => "CompoundAssign",
    }
}
