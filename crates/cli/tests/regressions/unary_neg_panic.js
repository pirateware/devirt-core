//! Regression test for unary negation panic.
//!
//! Issue: The SSA lowering panics with "not implemented: unary neg not yet lowered"
//! when encountering a unary negation operator.
//!
//! Source: lodash_cloneDeep.js from real-world corpus
//!
//! Minimal reproducer:

function testNegation() {
    var x = -1;
    var y = -x;
    return x + y;
}

// The panic occurs at src/ir/lower_ssa.rs:2388
// which is in the unimplemented!() macro for unary negation.

// Expected behavior: Should lower unary negation to SSA form
// and emit code correctly.

// Workaround: None currently - this is a hard panic.
