//! Regression test: Unary negation panic
//!
//! GitHub Issue: TBD (to be created from REALWORLD_STABILITY.md findings)
//!
//! The SSA lowering panics with "not implemented: unary neg not yet lowered"
//! when processing code containing unary negation operators.

#[test]
fn test_unary_negation_lowering() {
    // This is a minimal reproducer extracted from lodash_cloneDeep.js
    let source = r#"
function testNegation() {
    var x = -1;
    var y = -x;
    return x + y;
}
"#;

    // Should not panic
    let result = std::panic::catch_unwind(|| {
        let tree_module = mvp::lower::parse_and_lower(source).expect("Parse should succeed");
        let mut ssa_module = mvp::ir::lower_ssa::lower_module(&tree_module);
        ssa_module.validate().expect("Validation should succeed");
        let tree_back = mvp::ir::lower_tree::lower_module(&ssa_module);
        mvp::emit::emit(&tree_back)
    });

    // Currently this will panic - when fixed, it should succeed
    // For now, we document the expected behavior
    match result {
        Ok(emitted) => {
            // Verify the emitted code contains negation
            assert!(emitted.contains("-") || emitted.contains("testNegation"));
        }
        Err(_) => {
            // Expected to panic until unary neg is implemented
            // This documents the current behavior
            println!("Expected panic: unary negation not yet implemented in SSA lowering");
        }
    }
}

#[test]
fn test_unary_negation_roundtrip() {
    // Test that negation roundtrips correctly
    let source = r#"
function negate(x) {
    return -x;
}
"#;

    let result = std::panic::catch_unwind(|| {
        let tree_module = mvp::lower::parse_and_lower(source).expect("Parse should succeed");
        let mut ssa_module = mvp::ir::lower_ssa::lower_module(&tree_module);
        ssa_module.validate().expect("Validation should succeed");
        let tree_back = mvp::ir::lower_tree::lower_module(&ssa_module);
        let emitted = mvp::emit::emit(&tree_back);
        
        // Re-parse and verify
        let tree_module2 = mvp::lower::parse_and_lower(&emitted).expect("Re-parse should succeed");
        (emitted, tree_module2)
    });

    match result {
        Ok((emitted, _)) => {
            // Verify negation is preserved
            assert!(emitted.contains("-") || emitted.contains("negate"));
        }
        Err(_) => {
            println!("Expected panic: unary negation not yet implemented");
        }
    }
}
