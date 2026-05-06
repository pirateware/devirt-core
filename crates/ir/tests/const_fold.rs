//! Unit tests for `ir::ssa::try_const_fold` — the typed-arithmetic /
//! pure-comparison constant-fold helper used by ConstFoldPass.
//!
//! `Constant` deliberately does not derive `PartialEq` (NaN/Number
//! semantics make it ambiguous), so tests use `matches!` patterns
//! with explicit guards rather than `assert_eq!`.

use ir::ssa::{Constant, InstKind, Operand, try_const_fold};

fn n(x: f64) -> Operand {
    Operand::Const(Constant::Number(x))
}
fn i(x: i32) -> Operand {
    Operand::Const(Constant::Int32(x))
}
fn b(x: bool) -> Operand {
    Operand::Const(Constant::Bool(x))
}

fn assert_bool(r: Option<Constant>, expected: bool) {
    assert!(
        matches!(r, Some(Constant::Bool(x)) if x == expected),
        "want Some(Bool({expected}))"
    );
}
fn assert_none(r: Option<Constant>) {
    assert!(matches!(r, None));
}

// ---- Number arithmetic --------------------------------------------------

#[test]
fn add_number_basic() {
    let r = try_const_fold(&InstKind::AddNumber { lhs: n(1.5), rhs: n(2.25) });
    assert!(matches!(r, Some(Constant::Number(x)) if x == 3.75));
}

#[test]
fn sub_number_basic() {
    let r = try_const_fold(&InstKind::SubNumber { lhs: n(5.0), rhs: n(2.0) });
    assert!(matches!(r, Some(Constant::Number(x)) if x == 3.0));
}

#[test]
fn mul_number_basic() {
    let r = try_const_fold(&InstKind::MulNumber { lhs: n(3.0), rhs: n(4.0) });
    assert!(matches!(r, Some(Constant::Number(x)) if x == 12.0));
}

#[test]
fn add_number_nan_propagates() {
    let r = try_const_fold(&InstKind::AddNumber { lhs: n(f64::NAN), rhs: n(1.0) });
    assert!(matches!(r, Some(Constant::Number(x)) if x.is_nan()));
}

#[test]
fn add_number_infinity_propagates() {
    let r = try_const_fold(&InstKind::AddNumber { lhs: n(f64::INFINITY), rhs: n(1.0) });
    assert!(matches!(r, Some(Constant::Number(x)) if x.is_infinite() && x > 0.0));
}

#[test]
fn add_number_neg_zero_preserved() {
    // -0 + 0 = +0 per IEEE-754.
    let r = try_const_fold(&InstKind::AddNumber { lhs: n(-0.0), rhs: n(0.0) });
    assert!(matches!(r, Some(Constant::Number(x)) if x == 0.0 && x.to_bits() == 0_u64));
}

// ---- Int32 arithmetic (wrapping) ----------------------------------------

#[test]
fn add_int32_basic() {
    let r = try_const_fold(&InstKind::AddInt32 { lhs: i(2), rhs: i(3) });
    assert!(matches!(r, Some(Constant::Int32(5))));
}

#[test]
fn add_int32_wraps_on_overflow() {
    // i32::MAX + 1 wraps to i32::MIN — matches `(a+b) | 0` semantics.
    let r = try_const_fold(&InstKind::AddInt32 { lhs: i(i32::MAX), rhs: i(1) });
    assert!(matches!(r, Some(Constant::Int32(x)) if x == i32::MIN));
}

#[test]
fn mul_int32_wraps() {
    let r = try_const_fold(&InstKind::MulInt32 { lhs: i(i32::MAX), rhs: i(2) });
    assert!(matches!(r, Some(Constant::Int32(-2))));
}

// ---- Comparisons --------------------------------------------------------

#[test]
fn lt_number() {
    assert_bool(try_const_fold(&InstKind::Lt { lhs: n(1.0), rhs: n(2.0) }), true);
    assert_bool(try_const_fold(&InstKind::Lt { lhs: n(2.0), rhs: n(1.0) }), false);
}

#[test]
fn lt_int32() {
    assert_bool(try_const_fold(&InstKind::Lt { lhs: i(1), rhs: i(2) }), true);
}

#[test]
fn lt_with_nan_is_false() {
    assert_bool(try_const_fold(&InstKind::Lt { lhs: n(f64::NAN), rhs: n(1.0) }), false);
    assert_bool(try_const_fold(&InstKind::Lt { lhs: n(1.0), rhs: n(f64::NAN) }), false);
}

#[test]
fn le_number() {
    assert_bool(try_const_fold(&InstKind::Le { lhs: n(1.0), rhs: n(1.0) }), true);
}

#[test]
fn strict_eq_number() {
    assert_bool(try_const_fold(&InstKind::StrictEq { lhs: n(3.0), rhs: n(3.0) }), true);
}

#[test]
fn strict_eq_nan_is_false() {
    // ECMA-262: NaN !== NaN.
    assert_bool(
        try_const_fold(&InstKind::StrictEq { lhs: n(f64::NAN), rhs: n(f64::NAN) }),
        false,
    );
}

#[test]
fn strict_eq_pos_neg_zero_is_true() {
    // ECMA-262: +0 === -0.
    assert_bool(
        try_const_fold(&InstKind::StrictEq { lhs: n(0.0), rhs: n(-0.0) }),
        true,
    );
}

#[test]
fn strict_eq_mixed_types_is_false() {
    assert_bool(try_const_fold(&InstKind::StrictEq { lhs: n(1.0), rhs: i(1) }), false);
    assert_bool(try_const_fold(&InstKind::StrictEq { lhs: b(true), rhs: n(1.0) }), false);
}

#[test]
fn strict_eq_null_undefined() {
    assert_bool(
        try_const_fold(&InstKind::StrictEq {
            lhs: Operand::Const(Constant::Null),
            rhs: Operand::Const(Constant::Null),
        }),
        true,
    );
    assert_bool(
        try_const_fold(&InstKind::StrictEq {
            lhs: Operand::Const(Constant::Null),
            rhs: Operand::Const(Constant::Undefined),
        }),
        false,
    );
}

#[test]
fn strict_eq_skips_globalthis_and_this() {
    assert_none(try_const_fold(&InstKind::StrictEq {
        lhs: Operand::Const(Constant::GlobalThis),
        rhs: Operand::Const(Constant::Null),
    }));
    assert_none(try_const_fold(&InstKind::StrictEq {
        lhs: Operand::Const(Constant::This),
        rhs: Operand::Const(Constant::This),
    }));
}

// ---- Not ----------------------------------------------------------------

#[test]
fn not_bool() {
    assert_bool(try_const_fold(&InstKind::Not { operand: b(true) }), false);
    assert_bool(try_const_fold(&InstKind::Not { operand: b(false) }), true);
}

// ---- Refusal cases ------------------------------------------------------

#[test]
fn no_fold_on_value_operand() {
    use ir::ssa::Value;
    let v = Operand::Value(Value(0));
    assert_none(try_const_fold(&InstKind::AddNumber { lhs: v, rhs: n(1.0) }));
    assert_none(try_const_fold(&InstKind::AddNumber { lhs: n(1.0), rhs: v }));
}

#[test]
fn no_fold_on_generic_arithmetic() {
    assert_none(try_const_fold(&InstKind::AddGeneric { lhs: n(1.0), rhs: n(2.0) }));
}

#[test]
fn no_fold_on_loose_eq() {
    assert_none(try_const_fold(&InstKind::Eq { lhs: n(1.0), rhs: n(1.0) }));
}

#[test]
fn no_fold_on_heap_or_call() {
    use ir::ssa::Purity;
    use lasso::Rodeo;
    let mut r = Rodeo::default();
    let key = r.get_or_intern("foo");
    assert_none(try_const_fold(&InstKind::LoadProp {
        obj: Operand::Const(Constant::GlobalThis),
        key,
        purity: Purity::Throws,
    }));
    assert_none(try_const_fold(&InstKind::Call {
        callee: n(1.0),
        args: vec![],
    }));
}
