//! Interprocedural happens-before ("ordering") proof — the soundness crux of
//! cross-function write-once constant resolution.
//!
//! Extracted from `global_const_inline` so it can back a reusable
//! [`crate::ir::ConstResolver`]. The question it answers is: *does a write that
//! runs (unconditionally, once) inside an init function `W` provably complete
//! before a given read executes?* If yes, a constant established by that write
//! can be substituted at the read.
//!
//! The model is **sound by sufficient condition**: it only ever answers "yes"
//! when it can prove the ordering, and declines (no answer) otherwise — a decline
//! is always safe (the caller leaves the read symbolic).
//!
//! ## v1 sufficient conditions (the narrow core)
//!
//! - `W` is invoked **exactly once**, via a bare `W();` statement that is a direct
//!   child of a *region* `S` (the program body, or a top-level IIFE body), at
//!   statement index `c`.
//! - A read in the same region `S` is after-the-write iff its statement index is
//!   `> c`.
//! - A read inside a nested function `G` is after-the-write iff `G` is **not
//!   reachable** from the setup prefix — the transitive closure of functions
//!   invoked by `S`'s statements at index `≤ c`. A function unreachable from setup
//!   cannot run until that prefix completes, i.e. after the write.
//!
//! `OrderingOracle::build` constructs the regions + the coarse call graph once;
//! the per-symbol query methods consult them. Generalizations (nested setup
//! regions, IIFE-assignment, idempotency guards) widen the *sufficient
//! conditions* here without weakening the decline-when-unsure contract.

use std::collections::{HashMap, HashSet};

use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::Scoping;
use oxc_span::{GetSpan, Span};
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;

/// A region whose top-level statements may host the unconditional init calls.
pub struct Region<'b, 'a> {
    pub scope: ScopeId,
    pub stmts: &'b [Statement<'a>],
}

/// The interprocedural ordering proof. Borrows the program (regions point into
/// its statement lists) and a snapshot of the call census; ephemeral — built per
/// resolver construction.
pub struct OrderingOracle<'b, 'a> {
    regions: Vec<Region<'b, 'a>>,
    /// Caller function scope → callee function scopes (resolved calls only).
    edges: HashMap<ScopeId, HashSet<ScopeId>>,
    /// Function symbol → its function scope.
    sym_to_scope: HashMap<SymbolId, ScopeId>,
}

impl<'b, 'a> OrderingOracle<'b, 'a> {
    /// Build the regions and the coarse call graph.
    ///
    /// `fn_scope_symbol` maps each function scope to the symbol it is bound to;
    /// `raw_calls` is `(caller scope, callee symbol)` for every resolved call.
    pub fn build(
        program: &'b Program<'a>,
        scoping: &Scoping,
        fn_scope_symbol: &HashMap<ScopeId, SymbolId>,
        raw_calls: &[(ScopeId, SymbolId)],
    ) -> Self {
        let sym_to_scope: HashMap<SymbolId, ScopeId> = fn_scope_symbol
            .iter()
            .map(|(scope, sym)| (*sym, *scope))
            .collect();

        let mut edges: HashMap<ScopeId, HashSet<ScopeId>> = HashMap::new();
        for (caller, callee_sym) in raw_calls {
            if let Some(callee_scope) = sym_to_scope.get(callee_sym) {
                edges.entry(*caller).or_default().insert(*callee_scope);
            }
        }

        // Regions: the program body and every top-level IIFE body. A top-level
        // IIFE runs exactly once, in program order, when its statement executes —
        // so its body's statement indices are a valid happens-before axis, exactly
        // like the program body. Three IIFE-statement shapes are recognized:
        //   * `(function(){…})();`              (bare expression-statement IIFE)
        //   * `var X = (function(){…})();`       (declarator-init IIFE)
        //   * `X = (function(){…})();`           (assignment-expression IIFE)
        let mut regions: Vec<Region> = vec![Region {
            scope: scoping.root_scope_id(),
            stmts: &program.body,
        }];
        for stmt in &program.body {
            if let Some((scope, body)) = iife_region(stmt) {
                regions.push(Region { scope, stmts: body });
            }
        }

        OrderingOracle {
            regions,
            edges,
            sym_to_scope,
        }
    }

    fn region(&self, idx: usize) -> &Region<'b, 'a> {
        &self.regions[idx]
    }

    /// Find the single init call `wf_sym();` — a bare, no-arg call that is a direct
    /// child of some region. Returns `(region index, statement index c)`.
    fn find_init_call(&self, scoping: &Scoping, wf_sym: SymbolId) -> Option<(usize, usize)> {
        for (ridx, region) in self.regions.iter().enumerate() {
            for (i, stmt) in region.stmts.iter().enumerate() {
                if let Some(callee) = bare_call_callee(stmt) {
                    if let Some(rid) = callee.reference_id.get() {
                        if scoping.get_reference(rid).symbol_id() == Some(wf_sym) {
                            return Some((ridx, i));
                        }
                    }
                }
            }
        }
        None
    }

    /// Transitive set of function scopes invoked while executing region `ridx`'s
    /// statements at index `≤ c` (the setup prefix).
    fn setup_reachable(&self, ridx: usize, c: usize, scoping: &Scoping) -> HashSet<ScopeId> {
        let region = self.region(ridx);
        let mut roots: Vec<ScopeId> = Vec::new();
        let mut collector = CallCollector {
            scoping,
            sym_to_scope: &self.sym_to_scope,
            out: &mut roots,
        };
        for stmt in region.stmts.iter().take(c + 1) {
            collector.visit_statement(stmt);
        }

        let mut reachable: HashSet<ScopeId> = HashSet::new();
        let mut stack = roots;
        while let Some(scope) = stack.pop() {
            if reachable.insert(scope) {
                if let Some(callees) = self.edges.get(&scope) {
                    stack.extend(callees.iter().copied());
                }
            }
        }
        reachable
    }

    /// Decide whether every read in `reads` happens-after the single unconditional
    /// write of symbol `sym` that lives in function scope `wf` (bound to `wf_sym`,
    /// called exactly once).
    ///
    /// `reads` is `(reading function scope, read span)` for every read of `sym`.
    /// Returns `false` (decline) unless the init-call ordering can be proven for
    /// *all* reads.
    pub fn writes_before_all_reads(
        &self,
        scoping: &Scoping,
        wf_sym: SymbolId,
        reads: &[(ScopeId, Span)],
    ) -> bool {
        let Some((ridx, c)) = self.find_init_call(scoping, wf_sym) else {
            return false;
        };
        let region = self.region(ridx);
        let reachable = self.setup_reachable(ridx, c, scoping);

        reads.iter().all(|&(ef, span)| {
            if ef == region.scope {
                // Top-level read of S: must be in a statement after the init call.
                match child_index(region.stmts, span) {
                    Some(idx) => idx > c,
                    None => false,
                }
            } else {
                // Read inside a nested function: safe iff that function cannot run
                // during setup.
                !reachable.contains(&ef)
            }
        })
    }
}

/// Visitor collecting the function scopes of every resolved call within a subtree.
struct CallCollector<'o, 's> {
    scoping: &'s Scoping,
    sym_to_scope: &'s HashMap<SymbolId, ScopeId>,
    out: &'o mut Vec<ScopeId>,
}
impl<'a, 'o, 's> Visit<'a> for CallCollector<'o, 's> {
    fn visit_call_expression(&mut self, node: &CallExpression<'a>) {
        if let Expression::Identifier(callee) = &node.callee {
            if let Some(rid) = callee.reference_id.get() {
                if let Some(sym) = self.scoping.get_reference(rid).symbol_id() {
                    if let Some(scope) = self.sym_to_scope.get(&sym) {
                        self.out.push(*scope);
                    }
                }
            }
        }
        walk::walk_call_expression(self, node);
    }
}

/// If `stmt` is `Ident();` (bare, no arguments), return the callee identifier.
fn bare_call_callee<'b, 'a>(stmt: &'b Statement<'a>) -> Option<&'b IdentifierReference<'a>> {
    let Statement::ExpressionStatement(es) = stmt else {
        return None;
    };
    let Expression::CallExpression(call) = &es.expression else {
        return None;
    };
    if !call.arguments.is_empty() {
        return None;
    }
    match &call.callee {
        Expression::Identifier(id) => Some(id),
        _ => None,
    }
}

/// Index of the direct-child statement of `stmts` whose span contains `span`.
fn child_index(stmts: &[Statement<'_>], span: Span) -> Option<usize> {
    stmts
        .iter()
        .position(|s| s.span().start <= span.start && span.end <= s.span().end)
}

/// If `stmt` is a top-level IIFE statement — `(IIFE)();`, `var X = (IIFE)();`, or
/// `X = (IIFE)();` — return the invoked function's body scope and statements.
fn iife_region<'b, 'a>(stmt: &'b Statement<'a>) -> Option<(ScopeId, &'b [Statement<'a>])> {
    let expr = match stmt {
        Statement::ExpressionStatement(es) => match &es.expression {
            // `X = (IIFE)();` — the IIFE is the assignment RHS.
            Expression::AssignmentExpression(a)
                if matches!(a.operator, oxc_syntax::operator::AssignmentOperator::Assign) =>
            {
                &a.right
            }
            // `(IIFE)();` — the IIFE is the statement expression.
            other => other,
        },
        // `var X = (IIFE)();` — single declarator initialized by the IIFE.
        Statement::VariableDeclaration(vd) if vd.declarations.len() == 1 => {
            vd.declarations[0].init.as_ref()?
        }
        _ => return None,
    };
    iife_call_body(expr)
}

/// If `expr` is an immediately-invoked function expression (possibly behind unary
/// operators like `!`/`void`), return the function body scope and statements.
fn iife_call_body<'b, 'a>(expr: &'b Expression<'a>) -> Option<(ScopeId, &'b [Statement<'a>])> {
    let mut expr = expr;
    while let Expression::UnaryExpression(u) = expr {
        expr = &u.argument;
    }
    let Expression::CallExpression(call) = expr else {
        return None;
    };
    match &call.callee {
        Expression::FunctionExpression(f) => {
            let scope = f.scope_id.get()?;
            f.body.as_ref().map(|b| (scope, b.statements.as_slice()))
        }
        Expression::ArrowFunctionExpression(a) => {
            let scope = a.scope_id.get()?;
            Some((scope, a.body.statements.as_slice()))
        }
        _ => None,
    }
}
