//! Escape / capture analysis: decide which of a function's symbols may be put
//! into SSA form and folded.
//!
//! This is the soundness gatekeeper. A symbol survives as a *candidate* only if
//! every one of its occurrences is a shape the CFG lowerer models exactly:
//!   - a definition: a simple `s = rhs` assignment, or a `var/let/const`
//!     declarator with a plain binding identifier;
//!   - a use: a member read `s[k]` / `s.name`, or — for non-aggregate symbols —
//!     a bare read `s`.
//! Anything else disqualifies the symbol:
//!   - captured by a nested function/class (closure could observe a stale value);
//!   - any write that is not a simple `=` def (compound `+=`, `++`, destructuring,
//!     for-in/of target, member write — i.e. `is_write` reaching a bare ident);
//!   - an aggregate symbol (one ever assigned an array/object literal) that is
//!     ever read bare (could be aliased and mutated elsewhere).
//! If the function uses `eval` or `arguments`, the whole function is abandoned.
//!
//! The guarantee this buys: for every surviving candidate, the CFG provably sees
//! *all* of its definitions, and its aggregate value can never be mutated through
//! an alias. That is exactly what the SSA reaching-def fold relies on.

use std::collections::HashSet;

use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::Scoping;
use oxc_syntax::scope::{ScopeFlags, ScopeId};
use oxc_syntax::symbol::SymbolId;

/// Compute the versionable candidate symbols for one function unit.
///
/// `func_scope` is the unit's own scope; candidates are the symbols bound
/// directly in it (params + function-top-level vars). Returns an empty set if the
/// function must be abandoned (`eval`/`arguments`).
pub fn collect_candidates(
    stmts: &[Statement<'_>],
    func_scope: ScopeId,
    scoping: &Scoping,
) -> HashSet<SymbolId> {
    let base: HashSet<SymbolId> = scoping.iter_bindings_in(func_scope).collect();
    if base.is_empty() {
        return base;
    }

    let mut cls = Classifier {
        scoping,
        base: &base,
        depth: 0,
        excluded: HashSet::new(),
        has_aggregate_def: HashSet::new(),
        has_bare_read: HashSet::new(),
        bail: false,
    };
    for s in stmts {
        cls.visit_statement(s);
    }
    // Move the verdict out so the borrow of `base` (held by `cls`) ends here.
    let Classifier { excluded, has_aggregate_def, has_bare_read, bail, .. } = cls;
    if bail {
        return HashSet::new();
    }

    base.into_iter()
        .filter(|s| !excluded.contains(s))
        // An aggregate symbol that is ever read bare could be aliased and mutated
        // through the alias — never fold it.
        .filter(|s| !(has_aggregate_def.contains(s) && has_bare_read.contains(s)))
        .collect()
}

/// Names of local **aggregate** (array/object) bindings that are *not* provably
/// immutable — i.e. unsafe to bake into the decoder preamble (plan §5.3). An
/// aggregate is unsafe if it is assigned more than once, reassigned, member-written
/// (`t[k] = …`, `t[k]++`, `delete t[k]`), compound-assigned, mutated by a method
/// (`t.push(…)`), or read *bare* anywhere (a bare read could alias it into a
/// binding that is later mutated, or pass it to a callee that mutates it).
///
/// `decoder-lift` removes exactly these from its baked table set. The query is
/// deliberately scoped to *aggregates*: a scalar value is immutable, so a stale
/// bake is impossible, and scalars are the only things the grounded base-N
/// arithmetic chain depends on — excluding a scalar would break that chain's
/// dependency closure and is never necessary for soundness. Read-capture by a
/// nested closure is fine: a closure that only reads cannot mutate.
///
/// This is the symbol-precise complement to the name-based "assigned once"
/// heuristic in `decoder-lift`, which counts only bare assignments and is
/// therefore blind to `t[i] = x` / `t.push(x)` / aliasing — exactly the hazard
/// scoped lifting widens. Returns names (not `SymbolId`s) because `decoder-lift`
/// is name-keyed; the binding name's uniqueness is separately enforced there.
pub fn mutable_aggregate_tables(program: &Program<'_>, scoping: &Scoping) -> HashSet<String> {
    let mut scan = TableScan {
        scoping,
        defs: std::collections::HashMap::new(),
        aggregate_def: HashSet::new(),
        has_bare_read: HashSet::new(),
        disqualified: HashSet::new(),
    };
    scan.visit_program(program);

    scan.aggregate_def
        .iter()
        .filter(|sym| {
            scan.defs.get(*sym).copied() != Some(1)
                || scan.disqualified.contains(*sym)
                || scan.has_bare_read.contains(*sym)
        })
        .map(|sym| scoping.symbol_name(*sym).to_string())
        .collect()
}

/// Method names that mutate their receiver. A call to one on a candidate table
/// disqualifies it. Mirrors `decoder-lift`'s list; kept local so this module owns
/// its own soundness reasoning.
const MUTATOR_METHODS: &[&str] = &[
    "push", "pop", "shift", "unshift", "splice", "sort", "reverse", "fill", "copyWithin", "set",
    "add", "delete", "clear",
];

struct TableScan<'s> {
    scoping: &'s Scoping,
    /// Value-establishing writes per symbol (declarator-with-init or `t = rhs`).
    defs: std::collections::HashMap<SymbolId, usize>,
    /// Symbols whose definition is an array/object literal.
    aggregate_def: HashSet<SymbolId>,
    /// Symbols read *bare* (not as a member-access object) somewhere — an alias /
    /// escape hazard for aggregates only.
    has_bare_read: HashSet<SymbolId>,
    /// Symbols seen in a shape that mutates them (write / member-write / mutator).
    disqualified: HashSet<SymbolId>,
}

impl<'s> TableScan<'s> {
    fn ref_sym(&self, id: &IdentifierReference<'_>) -> Option<SymbolId> {
        let rid = id.reference_id.get()?;
        self.scoping.get_reference(rid).symbol_id()
    }
    fn obj_sym(&self, obj: &Expression<'_>) -> Option<SymbolId> {
        match obj {
            Expression::Identifier(id) => self.ref_sym(id),
            _ => None,
        }
    }
}

impl<'a, 's> Visit<'a> for TableScan<'s> {
    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &decl.id {
            if let Some(sym) = id.symbol_id.get() {
                if let Some(init) = &decl.init {
                    *self.defs.entry(sym).or_default() += 1;
                    if is_aggregate_literal(init) {
                        self.aggregate_def.insert(sym);
                    }
                    self.visit_expression(init);
                    return; // the binding id is not a reference
                }
            }
        }
        walk::walk_variable_declarator(self, decl);
    }

    fn visit_assignment_expression(&mut self, node: &AssignmentExpression<'a>) {
        use oxc_syntax::operator::AssignmentOperator;
        // Member-write target (`t[k] = …`, `t.x op= …`) mutates the object.
        if let Some(sym) = member_target_base(&node.left).and_then(|o| self.obj_sym(o)) {
            self.disqualified.insert(sym);
        }
        if let AssignmentTarget::AssignmentTargetIdentifier(id) = &node.left {
            if let Some(sym) = self.ref_sym(id) {
                if node.operator == AssignmentOperator::Assign {
                    *self.defs.entry(sym).or_default() += 1;
                    if is_aggregate_literal(&node.right) {
                        self.aggregate_def.insert(sym);
                    }
                    self.visit_expression(&node.right);
                    return;
                } else {
                    self.disqualified.insert(sym); // compound read-modify-write
                }
            }
        }
        walk::walk_assignment_expression(self, node);
    }

    fn visit_update_expression(&mut self, node: &UpdateExpression<'a>) {
        match &node.argument {
            // `t[k]++` / `t.x--`: a member mutation.
            SimpleAssignmentTarget::ComputedMemberExpression(m) => {
                if let Some(sym) = self.obj_sym(&m.object) {
                    self.disqualified.insert(sym);
                }
            }
            SimpleAssignmentTarget::StaticMemberExpression(m) => {
                if let Some(sym) = self.obj_sym(&m.object) {
                    self.disqualified.insert(sym);
                }
            }
            // `x++` on a bare candidate: a read-modify-write reassignment.
            SimpleAssignmentTarget::AssignmentTargetIdentifier(id) => {
                if let Some(sym) = self.ref_sym(id) {
                    self.disqualified.insert(sym);
                }
            }
            _ => {}
        }
        walk::walk_update_expression(self, node);
    }

    fn visit_unary_expression(&mut self, node: &UnaryExpression<'a>) {
        if node.operator == oxc_syntax::operator::UnaryOperator::Delete {
            let base = match &node.argument {
                Expression::ComputedMemberExpression(m) => Some(&m.object),
                Expression::StaticMemberExpression(m) => Some(&m.object),
                _ => None,
            };
            if let Some(sym) = base.and_then(|o| self.obj_sym(o)) {
                self.disqualified.insert(sym);
            }
        }
        walk::walk_unary_expression(self, node);
    }

    fn visit_call_expression(&mut self, node: &CallExpression<'a>) {
        // A mutator method on a candidate (`t.push(x)`) mutates the table.
        let recv = match &node.callee {
            Expression::StaticMemberExpression(m) => {
                Some((m.property.name.as_str(), &m.object))
            }
            Expression::ComputedMemberExpression(m) => match &m.expression {
                Expression::StringLiteral(s) => Some((s.value.as_str(), &m.object)),
                _ => None,
            },
            _ => None,
        };
        if let Some((name, obj)) = recv {
            if MUTATOR_METHODS.contains(&name) {
                if let Some(sym) = self.obj_sym(obj) {
                    self.disqualified.insert(sym);
                }
            }
        }
        walk::walk_call_expression(self, node);
    }

    fn visit_static_member_expression(&mut self, m: &StaticMemberExpression<'a>) {
        // `t.x` read: a clean member access — don't descend into the object id
        // (that would count as a bare read and disqualify the table).
        if self.obj_sym(&m.object).is_some() {
            return;
        }
        walk::walk_static_member_expression(self, m);
    }

    fn visit_computed_member_expression(&mut self, m: &ComputedMemberExpression<'a>) {
        if self.obj_sym(&m.object).is_some() {
            // Skip the object; the index may reference candidates, so visit it.
            self.visit_expression(&m.expression);
            return;
        }
        walk::walk_computed_member_expression(self, m);
    }

    fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
        // Any reference reaching here is a *bare* occurrence (member-access objects
        // are intercepted above; bare write/def targets are intercepted in the
        // write visitors and return early). Record it as a bare read — harmless for
        // a scalar (immutable by value), but for an aggregate it is an alias /
        // escape hazard that the final filter rejects.
        if let Some(sym) = self.ref_sym(id) {
            self.has_bare_read.insert(sym);
        }
    }
}

struct Classifier<'s, 'b> {
    scoping: &'s Scoping,
    base: &'b HashSet<SymbolId>,
    /// Function/class nesting depth relative to the unit (0 = the unit itself).
    depth: u32,
    excluded: HashSet<SymbolId>,
    has_aggregate_def: HashSet<SymbolId>,
    has_bare_read: HashSet<SymbolId>,
    bail: bool,
}

impl<'s, 'b> Classifier<'s, 'b> {
    fn ref_sym(&self, id: &IdentifierReference<'_>) -> Option<SymbolId> {
        let rid = id.reference_id.get()?;
        self.scoping.get_reference(rid).symbol_id()
    }

    /// A `for (var <pat> of/in …)` head rebinds its pattern's symbols on every
    /// iteration — a write the CFG does not model (`lower_for_each` ignores the
    /// head) and that is not an `is_write` reference. Exclude every candidate it
    /// binds. (Bare/destructuring `for (s of …)` targets are `is_write` references
    /// already excluded by `visit_identifier_reference`; `let`/`const` heads are
    /// block-scoped, hence never candidates.)
    fn exclude_for_head<'a>(&mut self, left: &ForStatementLeft<'a>) {
        if let ForStatementLeft::VariableDeclaration(d) = left {
            let mut ex = BindingExcluder { excluded: &mut self.excluded, base: self.base };
            ex.visit_variable_declaration(d);
        }
    }

    fn cand_object<'x>(&self, obj: &'x Expression<'x>) -> Option<SymbolId> {
        let Expression::Identifier(id) = obj else { return None };
        let sym = self.ref_sym(id)?;
        if self.base.contains(&sym) {
            Some(sym)
        } else {
            None
        }
    }
}

impl<'a, 's, 'b> Visit<'a> for Classifier<'s, 'b> {
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        self.depth += 1;
        walk::walk_function(self, it, flags);
        self.depth -= 1;
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        self.depth += 1;
        walk::walk_arrow_function_expression(self, it);
        self.depth -= 1;
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        self.depth += 1;
        walk::walk_class(self, it);
        self.depth -= 1;
    }

    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        if self.depth == 0 {
            if let BindingPattern::BindingIdentifier(id) = &decl.id {
                if let Some(sym) = id.symbol_id.get() {
                    if self.base.contains(&sym) {
                        if let Some(init) = &decl.init {
                            if is_aggregate_literal(init) {
                                self.has_aggregate_def.insert(sym);
                            }
                            self.visit_expression(init);
                        }
                        return; // binding id is not a reference: nothing to escape
                    }
                }
            }
        }
        walk::walk_variable_declarator(self, decl);
    }

    fn visit_assignment_expression(&mut self, node: &AssignmentExpression<'a>) {
        use oxc_syntax::operator::AssignmentOperator;
        if self.depth == 0 {
            // A member-write target (`s[k] = v`, `s.x op= v`) mutates the object:
            // exclude its base. Must run before the member-read interception in
            // `visit_*_member_expression`, which would otherwise misread the write
            // target as a clean read and leave `s` foldable.
            if let Some(sym) = member_target_base(&node.left).and_then(|o| self.cand_object(o)) {
                self.excluded.insert(sym);
            }
            if let AssignmentTarget::AssignmentTargetIdentifier(id) = &node.left {
                if let Some(sym) = self.ref_sym(id) {
                    if self.base.contains(&sym) {
                        if node.operator == AssignmentOperator::Assign {
                            // A recognised simple def. Record aggregate-ness and
                            // visit only the rhs (the target is not an escape).
                            if is_aggregate_literal(&node.right) {
                                self.has_aggregate_def.insert(sym);
                            }
                            self.visit_expression(&node.right);
                        } else {
                            // Compound `+=` etc.: an unmodelled read-modify-write.
                            self.excluded.insert(sym);
                            self.visit_expression(&node.right);
                        }
                        return;
                    }
                }
            }
        }
        walk::walk_assignment_expression(self, node);
    }

    fn visit_for_of_statement(&mut self, node: &ForOfStatement<'a>) {
        if self.depth == 0 {
            self.exclude_for_head(&node.left);
        }
        walk::walk_for_of_statement(self, node);
    }

    fn visit_for_in_statement(&mut self, node: &ForInStatement<'a>) {
        if self.depth == 0 {
            self.exclude_for_head(&node.left);
        }
        walk::walk_for_in_statement(self, node);
    }

    fn visit_update_expression(&mut self, node: &UpdateExpression<'a>) {
        // `s[k]++` / `--s.x`: a member mutation → exclude the base.
        if self.depth == 0 {
            let base = match &node.argument {
                SimpleAssignmentTarget::ComputedMemberExpression(m) => Some(&m.object),
                SimpleAssignmentTarget::StaticMemberExpression(m) => Some(&m.object),
                _ => None,
            };
            if let Some(sym) = base.and_then(|o| self.cand_object(o)) {
                self.excluded.insert(sym);
            }
        }
        walk::walk_update_expression(self, node);
    }

    fn visit_unary_expression(&mut self, node: &UnaryExpression<'a>) {
        // `delete s[k]` / `delete s.x`: mutates the object → exclude the base.
        if self.depth == 0 && node.operator == oxc_syntax::operator::UnaryOperator::Delete {
            let base = match &node.argument {
                Expression::ComputedMemberExpression(m) => Some(&m.object),
                Expression::StaticMemberExpression(m) => Some(&m.object),
                _ => None,
            };
            if let Some(sym) = base.and_then(|o| self.cand_object(o)) {
                self.excluded.insert(sym);
            }
        }
        walk::walk_unary_expression(self, node);
    }

    fn visit_static_member_expression(&mut self, m: &StaticMemberExpression<'a>) {
        if self.depth == 0 {
            if self.cand_object(&m.object).is_some() {
                // `s.name` read: a clean member access — do not descend into the
                // object identifier (that would mark it a bare read / escape).
                return;
            }
        }
        walk::walk_static_member_expression(self, m);
    }

    fn visit_computed_member_expression(&mut self, m: &ComputedMemberExpression<'a>) {
        if self.depth == 0 {
            if self.cand_object(&m.object).is_some() {
                // `s[expr]` read: skip the object; the index may itself reference
                // candidates, so visit it.
                self.visit_expression(&m.expression);
                return;
            }
        }
        walk::walk_computed_member_expression(self, m);
    }

    fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
        if id.name == "eval" || id.name == "arguments" {
            self.bail = true;
            return;
        }
        if let Some(sym) = self.ref_sym(id) {
            if self.base.contains(&sym) {
                let reference = id
                    .reference_id
                    .get()
                    .map(|rid| self.scoping.get_reference(rid));
                if self.depth > 0 {
                    // Captured by a nested function/class.
                    self.excluded.insert(sym);
                } else if reference.is_some_and(|r| r.is_write()) {
                    // A write that bypassed the recognised def shapes (for-of/in
                    // target, destructuring, etc.).
                    self.excluded.insert(sym);
                } else {
                    // A bare read in value position (or a member-write base, etc.).
                    self.has_bare_read.insert(sym);
                }
            }
        }
    }
}

/// Excludes every candidate symbol bound by a binding pattern (handles nested
/// destructuring, defaults, and rest elements without enumerating each shape).
struct BindingExcluder<'x> {
    excluded: &'x mut HashSet<SymbolId>,
    base: &'x HashSet<SymbolId>,
}

impl<'a, 'x> Visit<'a> for BindingExcluder<'x> {
    fn visit_binding_identifier(&mut self, id: &BindingIdentifier<'a>) {
        if let Some(sym) = id.symbol_id.get() {
            if self.base.contains(&sym) {
                self.excluded.insert(sym);
            }
        }
    }
}

/// Whether an initializer/rhs is an array or object literal (an aggregate value).
fn is_aggregate_literal(e: &Expression<'_>) -> bool {
    matches!(e, Expression::ArrayExpression(_) | Expression::ObjectExpression(_))
}

/// If an assignment target is a member expression (`s[k]` / `s.x`), the base
/// object expression — whose symbol, if a candidate, is being mutated.
fn member_target_base<'x, 'a>(target: &'x AssignmentTarget<'a>) -> Option<&'x Expression<'a>> {
    match target {
        AssignmentTarget::ComputedMemberExpression(m) => Some(&m.object),
        AssignmentTarget::StaticMemberExpression(m) => Some(&m.object),
        _ => None,
    }
}
