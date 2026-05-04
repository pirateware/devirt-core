//! Tree-IR → SSA-IR conversion using the Braun et al. 2013 algorithm
//! ("Simple and Efficient Construction of Static Single Assignment Form").
//!
//! The converter walks the tree-IR top-down, threading a per-function
//! `Converter` that owns the Braun bookkeeping (`current_def`,
//! `incomplete_params`, `sealed`) and a lexical scope stack mapping JS
//! identifiers to `SymbolId`s. All Braun state is keyed by `SymbolId`, never
//! by name, so lexical shadowing falls out for free.
//!
//! Trivial-param removal is mandatory (per the paper). Without it, every
//! join block accumulates redundant parameters that downstream passes have
//! to see through.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use crate::ir::ssa::{
    self, BasicBlock, BlockId, Constant, FuncId, FunctionBuilder, InstKind, Operand, Purity, StrId,
    Terminator, Value, ValueDef, ValueDefKind,
};
use crate::ir::tree::{self, BinOp, Expr, Stmt, UnOp};

// ============================================================================
// Public entry points
// ============================================================================

pub fn lower_module(tm: &tree::Module) -> ssa::Module {
    let mut sm = ssa::Module::new();

    // Pass 1: assign FuncIds to all top-level function-shaped items
    // (Lowered and PassThrough alike) in source order. Both variants
    // result in a `Function` slot in the module, in the same order.
    let mut func_names: HashMap<String, FuncId> = HashMap::new();
    let mut idx: u32 = 0;
    for it in &tm.items {
        match it {
            tree::Item::Function(f) => {
                func_names.insert(f.name.clone(), FuncId(idx));
                idx += 1;
            }
            tree::Item::PassThroughFunction(lit) => {
                if let Some(name) = &lit.name {
                    func_names.insert(name.clone(), FuncId(idx));
                }
                idx += 1;
            }
            tree::Item::Stmt(_) => {}
        }
    }

    // Pass 2: lower each function. Top-level statements (non-functions)
    // are currently dropped; the tree-IR allows them but the SSA module
    // shape is function-only.
    for it in &tm.items {
        match it {
            tree::Item::Function(f) => {
                lower_function(&mut sm, f, &func_names);
            }
            tree::Item::PassThroughFunction(lit) => {
                push_passthrough_function(&mut sm, lit);
            }
            tree::Item::Stmt(_) => {}
        }
    }

    sm
}

/// Build an SSA `Function` with `FunctionBody::PassThrough` from a
/// tree-IR `FuncLiteral` whose body is `FuncBody::PassThrough`. No
/// `FunctionBuilder` involved — PassThrough functions have no blocks,
/// values, or instructions.
fn push_passthrough_function(sm: &mut ssa::Module, lit: &tree::FuncLiteral) {
    let source = match &lit.body {
        tree::FuncBody::PassThrough(s) => s.clone(),
        _ => panic!("Item::PassThroughFunction must carry a PassThrough body"),
    };
    let name = lit.name.clone().unwrap_or_default();
    let function_name_id = lit.name.as_ref().map(|n| sm.strings.get_or_intern(n));
    let param_names: Vec<ssa::StrId> = lit
        .params
        .iter()
        .map(|p| sm.strings.get_or_intern(p))
        .collect();
    sm.functions.push(ssa::Function {
        name,
        capture_count: 0,
        body: ssa::FunctionBody::PassThrough(ssa::PassThroughBody {
            source,
            param_names,
            function_name: function_name_id,
            is_async: lit.is_async,
            is_generator: lit.is_generator,
            is_arrow: lit.is_arrow,
            body_is_expression: false,
        }),
    });
}

fn lower_function(
    sm: &mut ssa::Module,
    f: &tree::IrFunction,
    func_names: &HashMap<String, FuncId>,
) {
    let mut builder = FunctionBuilder::new(sm, f.name.clone(), f.params.len() as u32);
    {
        let mut conv = Converter::new(&mut builder, func_names);
        conv.lower(f);
    }
    builder.finish();
}

// ============================================================================
// SymbolId — internal identifier for Braun keys
// ============================================================================

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
struct SymbolId(u32);

// ============================================================================
// Converter
// ============================================================================

struct Converter<'b, 'm> {
    builder: &'b mut FunctionBuilder<'m>,
    func_names: &'b HashMap<String, FuncId>,

    // Symbol allocation
    next_symbol: u32,

    // Lexical scope stack. The innermost (most recently pushed) scope is
    // searched first; an inner `let` may shadow an outer binding by
    // allocating a fresh SymbolId in the inner scope.
    scopes: Vec<HashMap<String, SymbolId>>,

    // Braun's `current_def`: per (variable, block), the operand to use when
    // the variable is read in that block.
    current_def: HashMap<(SymbolId, BlockId), Operand>,

    // Block params added speculatively while the block was unsealed; filled
    // (via `add_param_operands`) at seal time. Order matters — params are
    // appended to `block.params` in the same order, and edge args must match
    // by position.
    incomplete_params: HashMap<BlockId, Vec<(SymbolId, Value)>>,

    // Sealed blocks. A sealed block promises no further predecessors will be
    // added, which means reads can fully resolve through `add_param_operands`.
    sealed: HashSet<BlockId>,

    // Source name (interned) for each SymbolId that originated from a
    // named source binding. Used to annotate block params introduced by
    // Braun for that SymbolId so the out-of-SSA renamer can recover the
    // original name across loop carriers.
    symbol_names: HashMap<SymbolId, StrId>,
}

impl<'b, 'm> Converter<'b, 'm> {
    fn new(builder: &'b mut FunctionBuilder<'m>, func_names: &'b HashMap<String, FuncId>) -> Self {
        Self {
            builder,
            func_names,
            next_symbol: 0,
            scopes: Vec::new(),
            current_def: HashMap::new(),
            incomplete_params: HashMap::new(),
            sealed: HashSet::new(),
            symbol_names: HashMap::new(),
        }
    }

    fn lower(&mut self, f: &tree::IrFunction) {
        // Entry block. Sealed from the start because nothing branches into it.
        let entry = self.builder.new_block();
        self.builder.switch_to_block(entry);
        self.sealed.insert(entry);

        // Scope 0: module — top-level function names bound to Constant::Func.
        // This makes references to other top-level functions (and the
        // function's own name, for recursion) resolve via the normal Braun
        // path.
        self.push_scope();
        // Iterate in deterministic order so the SymbolId assignment is
        // reproducible across runs.
        let mut sorted_names: Vec<(&String, &FuncId)> = self.func_names.iter().collect();
        sorted_names.sort_by_key(|(n, _)| n.as_str());
        for (name, fid) in sorted_names {
            let sym = self.fresh_symbol();
            self.bind_in_top_scope(name.clone(), sym);
            self.write_variable(sym, entry, Operand::Const(Constant::Func(*fid)));
        }

        // Scope 1: function parameters and body locals.
        self.push_scope();
        for (i, pname) in f.params.iter().enumerate() {
            let sym = self.fresh_symbol();
            self.bind_in_top_scope(pname.clone(), sym);
            let arg_val = self.builder.body.params[i];
            // Annotate the parameter Value with its source name so the
            // out-of-SSA renamer emits e.g. `function f(a, b)` rather than
            // `function f(_a0, _a1)`.
            let name_id = self.builder.intern(pname);
            self.builder.set_source_name(arg_val, name_id);
            self.write_variable(sym, entry, Operand::Value(arg_val));
        }

        for stmt in &f.body {
            self.lower_stmt(stmt);
        }

        self.pop_scope();
        self.pop_scope();

        // Implicit return undefined: any block that fell off the end without
        // setting a terminator gets one now. JS function bodies that end
        // without `return` evaluate to undefined.
        let cb = self
            .builder
            .current_block()
            .expect("function lowering left no current block");
        if !self.is_terminated(cb) {
            self.builder
                .set_terminator(Terminator::Return { val: None });
        }
    }

    // ------------------------------------------------------------------
    // Scope management
    // ------------------------------------------------------------------

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn bind_in_top_scope(&mut self, name: String, sym: SymbolId) {
        let id = self.builder.intern(&name);
        self.symbol_names.insert(sym, id);
        self.scopes
            .last_mut()
            .expect("no active scope")
            .insert(name, sym);
    }

    fn lookup(&self, name: &str) -> Option<SymbolId> {
        for scope in self.scopes.iter().rev() {
            if let Some(s) = scope.get(name) {
                return Some(*s);
            }
        }
        None
    }

    fn fresh_symbol(&mut self) -> SymbolId {
        let s = SymbolId(self.next_symbol);
        self.next_symbol += 1;
        s
    }

    /// Annotate an Operand-defined value with the source-level binding
    /// name. Only tags values that don't yet have a name — this matters
    /// for `let r = x;` where the RHS is an Ident already-tagged via its
    /// original binding ("x"); re-tagging it as "r" would lose the
    /// stronger name. The renamer's first-claim-wins handles the case
    /// where two distinct Insts coincide on the same name.
    ///
    /// No-op for Const operands and for retired Values.
    fn tag_binding_source_name(&mut self, op: Operand, name: &str) {
        if let Operand::Value(v) = op {
            let def = &self.builder.body.values[v.0 as usize];
            if def.is_removed() || def.source_name.is_some() {
                return;
            }
            let id = self.builder.intern(name);
            self.builder.set_source_name(v, id);
        }
    }

    // ------------------------------------------------------------------
    // Braun primitives
    // ------------------------------------------------------------------

    fn write_variable(&mut self, var: SymbolId, block: BlockId, op: Operand) {
        self.current_def.insert((var, block), op);
    }

    fn read_variable(&mut self, var: SymbolId, block: BlockId) -> Operand {
        if let Some(op) = self.current_def.get(&(var, block)) {
            return *op;
        }
        self.read_variable_recursive(var, block)
    }

    fn read_variable_recursive(&mut self, var: SymbolId, block: BlockId) -> Operand {
        if !self.sealed.contains(&block) {
            // Unsealed: speculatively add a param and queue it for filling.
            let param = self.builder.add_block_param(block);
            self.tag_block_param_with_symbol(param, var);
            self.incomplete_params
                .entry(block)
                .or_default()
                .push((var, param));
            self.write_variable(var, block, Operand::Value(param));
            return Operand::Value(param);
        }

        let preds = self.predecessors_of(block);
        if preds.len() == 1 {
            let val = self.read_variable(var, preds[0]);
            self.write_variable(var, block, val);
            return val;
        }

        // Sealed with multiple preds. Add a param, write the placeholder
        // BEFORE recursing so cycles bottom out. Then fill operands.
        let param = self.builder.add_block_param(block);
        self.tag_block_param_with_symbol(param, var);
        self.write_variable(var, block, Operand::Value(param));
        self.add_param_operands(var, block, param)
    }

    fn tag_block_param_with_symbol(&mut self, param: Value, var: SymbolId) {
        if let Some(&name) = self.symbol_names.get(&var) {
            self.builder.set_source_name(param, name);
        }
    }

    fn add_param_operands(&mut self, var: SymbolId, block: BlockId, param: Value) -> Operand {
        let preds = self.predecessors_of(block);
        for pred in preds {
            let op = self.read_variable(var, pred);
            self.append_edge_arg(pred, block, op);
        }
        self.try_remove_trivial_param(param)
    }

    fn try_remove_trivial_param(&mut self, param: Value) -> Operand {
        // Look up the block and current index of the param. If it's not a
        // block param (or already removed), bail out conservatively.
        let (block, idx) = match self.builder.body.values[param.0 as usize].kind {
            ValueDefKind::BlockParam { block, index } => (block, index as usize),
            ValueDefKind::Removed => return Operand::Const(Constant::Undefined),
            _ => return Operand::Value(param),
        };

        // Snapshot incomings at this index.
        let preds = self.predecessors_of(block);
        let mut incomings: Vec<Operand> = Vec::with_capacity(preds.len());
        for &p in &preds {
            let op = self.edge_arg_at(p, block, idx).unwrap_or_else(|| {
                panic!(
                    "predecessor bb{} of bb{} has no edge arg at index {}",
                    p.0, block.0, idx
                )
            });
            incomings.push(op);
        }

        // Determine `same`: the unique non-self incoming, or Undefined if
        // there are no non-self incomings.
        let mut same: Option<Operand> = None;
        for op in &incomings {
            if let Operand::Value(v) = op
                && *v == param
            {
                continue; // self-reference: harmless
            }
            match same {
                None => same = Some(*op),
                Some(s) if operand_eq(&s, op) => continue,
                Some(_) => return Operand::Value(param), // not trivial
            }
        }
        let same = same.unwrap_or(Operand::Const(Constant::Undefined));

        // Find users that are themselves block params, BEFORE replace_uses
        // walks the IR (which would erase the references we look at here).
        let block_param_users = self.find_block_param_users(param);

        // Substitute every use of `param` with `same`.
        self.replace_value_uses(param, same);

        // Drop the param from the block, then strip the corresponding edge
        // arg from each predecessor's terminator.
        self.remove_block_param(block, idx);
        for &p in &preds {
            self.remove_edge_arg_at(p, block, idx);
        }

        // Retire the Value slot.
        self.builder.body.values[param.0 as usize] = ValueDef::removed();

        // Recurse on users that may have become trivial.
        for u in block_param_users {
            if u == param {
                continue;
            }
            if self.builder.body.values[u.0 as usize].is_removed() {
                continue;
            }
            self.try_remove_trivial_param(u);
        }

        same
    }

    fn seal_block(&mut self, b: BlockId) {
        if self.sealed.contains(&b) {
            return;
        }
        // Take ownership of the entries; trivial-param removal during fills
        // can mutate `incomplete_params` indirectly, so iterating a borrowed
        // entry is unsafe.
        let entries = self.incomplete_params.remove(&b).unwrap_or_default();
        for (sym, param) in entries {
            // A param may have been retired by an earlier iteration's
            // recursive trivial-removal; skip in that case.
            if self.builder.body.values[param.0 as usize].is_removed() {
                continue;
            }
            self.add_param_operands(sym, b, param);
        }
        self.sealed.insert(b);
    }

    // ------------------------------------------------------------------
    // Statement lowering
    // ------------------------------------------------------------------

    fn lower_stmt(&mut self, stmt: &Stmt) {
        // If the current block already terminates (e.g., previous stmt was
        // a Return), insert a fresh dead block here so we don't overwrite
        // the existing terminator. Lowering into this block is preserved as
        // dead code; downstream DCE can prune it.
        let cb = self.current_block();
        if self.is_terminated(cb) {
            let dead = self.builder.new_block();
            self.sealed.insert(dead);
            self.builder.switch_to_block(dead);
        }
        match stmt {
            Stmt::Let { name, init } => {
                let op = match init {
                    Some(e) => self.convert_expr(e),
                    None => Operand::Const(Constant::Undefined),
                };
                self.tag_binding_source_name(op, name);
                let sym = self.fresh_symbol();
                self.bind_in_top_scope(name.clone(), sym);
                let cb = self.current_block();
                self.write_variable(sym, cb, op);
            }
            Stmt::Const { name, init } => {
                let op = self.convert_expr(init);
                self.tag_binding_source_name(op, name);
                let sym = self.fresh_symbol();
                self.bind_in_top_scope(name.clone(), sym);
                let cb = self.current_block();
                self.write_variable(sym, cb, op);
            }
            Stmt::Assign { target, value } => {
                let val_op = self.convert_expr(value);
                match target {
                    Expr::Ident(name) => {
                        if let Some(sym) = self.lookup(name) {
                            // Tag the new value with the binding's source
                            // name so a same-name reassignment chain
                            // re-emerges in emit (suffix-disambiguated by
                            // the renamer when needed).
                            self.tag_binding_source_name(val_op, name);
                            let cb = self.current_block();
                            self.write_variable(sym, cb, val_op);
                        } else {
                            // Sloppy-mode-equivalent unbound write: lowers to
                            // a property store on the global object. Strict
                            // mode would throw on the unbound write; we
                            // accept the divergence (see read-side note).
                            // The special-case constants (globalThis,
                            // undefined, NaN, Infinity, …) are read-only at
                            // the language level; assigning to them is a
                            // no-op in sloppy mode and a TypeError in strict
                            // mode. We model the no-op by routing through
                            // the global store anyway — the value lands in
                            // a globalThis property of that name.
                            let key = self.builder.intern(name);
                            self.builder.add_inst(InstKind::StoreProp {
                                obj: Operand::Const(Constant::GlobalThis),
                                key,
                                val: val_op,
                                purity: Purity::Throws,
                            });
                        }
                    }
                    Expr::Member { obj, prop } => {
                        let obj_op = self.convert_expr(obj);
                        let key = self.builder.intern(prop);
                        let purity = property_access_purity(&obj_op);
                        self.builder.add_inst(InstKind::StoreProp {
                            obj: obj_op,
                            key,
                            val: val_op,
                            purity,
                        });
                    }
                    Expr::Index { obj, index } => {
                        let obj_op = self.convert_expr(obj);
                        let idx_op = self.convert_expr(index);
                        let purity = property_access_purity(&obj_op);
                        self.builder.add_inst(InstKind::StoreIndex {
                            obj: obj_op,
                            index: idx_op,
                            val: val_op,
                            purity,
                        });
                    }
                    _ => panic!("invalid assignment target: {target:?}"),
                }
            }
            Stmt::ExprStmt(e) => {
                let _ = self.convert_expr(e);
            }
            Stmt::Return(e) => {
                let val = e.as_ref().map(|x| self.convert_expr(x));
                self.builder.set_terminator(Terminator::Return { val });
                // We do NOT switch to a fresh block here. If a subsequent
                // statement appears, lower_stmt's prologue creates a dead
                // block on demand, avoiding a stray empty block when no
                // dead code follows.
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => self.lower_if(cond, then_body, else_body.as_deref()),
            Stmt::While { cond, body } => self.lower_while(cond, body),
            Stmt::Block(stmts) => {
                self.push_scope();
                for s in stmts {
                    self.lower_stmt(s);
                }
                self.pop_scope();
            }
            Stmt::FuncDecl(_) => {
                // Stage 4 will lower nested function declarations into an
                // AllocClosure bound at this point. Until then, the tree-IR
                // never carries this variant: lower.rs still rejects nested
                // function declarations at parse time, so this arm is
                // unreachable in production. Panic to make any premature
                // construction loud.
                panic!("Stmt::FuncDecl reached SSA lowering before stage 4 wired closures")
            }
        }
    }

    fn lower_if(&mut self, cond: &Expr, then_body: &[Stmt], else_body: Option<&[Stmt]>) {
        let cond_op = self.convert_expr(cond);
        let then_b = self.builder.new_block();
        let else_b = self.builder.new_block();
        let join_b = self.builder.new_block();

        self.builder.set_terminator(Terminator::CondBranch {
            cond: cond_op,
            then_b,
            then_args: vec![],
            else_b,
            else_args: vec![],
        });
        // then/else have exactly one pred each (the cbr we just set), so
        // they can be sealed immediately. join has up to two preds and stays
        // unsealed until both branches finish.
        self.sealed.insert(then_b);
        self.sealed.insert(else_b);

        // then-branch
        self.builder.switch_to_block(then_b);
        self.push_scope();
        for s in then_body {
            self.lower_stmt(s);
        }
        self.pop_scope();
        let then_end = self.current_block();
        if !self.is_terminated(then_end) {
            self.builder.set_terminator(Terminator::Branch {
                target: join_b,
                args: vec![],
            });
        }

        // else-branch (an absent else lowers to an empty body that just
        // joins, preserving the join's two-pred shape).
        self.builder.switch_to_block(else_b);
        self.push_scope();
        if let Some(eb) = else_body {
            for s in eb {
                self.lower_stmt(s);
            }
        }
        self.pop_scope();
        let else_end = self.current_block();
        if !self.is_terminated(else_end) {
            self.builder.set_terminator(Terminator::Branch {
                target: join_b,
                args: vec![],
            });
        }

        self.seal_block(join_b);
        self.builder.switch_to_block(join_b);
    }

    fn lower_while(&mut self, cond: &Expr, body: &[Stmt]) {
        let header_b = self.builder.new_block();
        let body_b = self.builder.new_block();
        let exit_b = self.builder.new_block();

        // Branch from current to header. Header has at least one pred (us);
        // a back-edge from body will be added later. We must NOT seal
        // header yet — the back-edge predecessor is missing.
        self.builder.set_terminator(Terminator::Branch {
            target: header_b,
            args: vec![],
        });

        // body and exit each have a single pred (the cbr we'll set in
        // header). Safe to seal.
        self.sealed.insert(body_b);
        self.sealed.insert(exit_b);

        // Header: condition + branch.
        self.builder.switch_to_block(header_b);
        let cond_op = self.convert_expr(cond);
        self.builder.set_terminator(Terminator::CondBranch {
            cond: cond_op,
            then_b: body_b,
            then_args: vec![],
            else_b: exit_b,
            else_args: vec![],
        });

        // Body.
        self.builder.switch_to_block(body_b);
        self.push_scope();
        for s in body {
            self.lower_stmt(s);
        }
        self.pop_scope();
        let body_end = self.current_block();
        if !self.is_terminated(body_end) {
            self.builder.set_terminator(Terminator::Branch {
                target: header_b,
                args: vec![],
            });
        }

        // NOW seal header — the back-edge predecessor exists. Sealing here
        // (rather than earlier) is what gives loop-varying variables their
        // phi-merged values. Sealing too early would silently dead-end
        // back-edge reads.
        self.seal_block(header_b);

        self.builder.switch_to_block(exit_b);
    }

    // ------------------------------------------------------------------
    // Expression lowering
    // ------------------------------------------------------------------

    fn convert_expr(&mut self, e: &Expr) -> Operand {
        match e {
            Expr::Number(n) => Operand::Const(Constant::Number(*n)),
            Expr::String(s) => {
                let id = self.builder.intern(s);
                Operand::Const(Constant::String(id))
            }
            Expr::Bool(b) => Operand::Const(Constant::Bool(*b)),
            Expr::Null => Operand::Const(Constant::Null),
            Expr::Undefined => Operand::Const(Constant::Undefined),
            Expr::Ident(name) => {
                if let Some(sym) = self.lookup(name) {
                    let cb = self.current_block();
                    return self.read_variable(sym, cb);
                }
                // Unbound identifier: lower as a property access on the
                // global object (or a special-case constant). Strict-mode JS
                // would throw ReferenceError on a bare unbound read while
                // `globalThis.foo` would return undefined; we treat both
                // identically (sloppy-mode-equivalent) — acceptable for
                // malware analysis, where strict-mode divergence is rare and
                // not load-bearing for understanding intent.
                if let Some(c) = special_global_constant(name) {
                    return Operand::Const(c);
                }
                let key = self.builder.intern(name);
                self.emit(InstKind::LoadProp {
                    obj: Operand::Const(Constant::GlobalThis),
                    key,
                    purity: Purity::Throws,
                })
            }
            Expr::Binary { op, lhs, rhs } => self.convert_binary(*op, lhs, rhs),
            Expr::Unary { op, operand } => self.convert_unary(*op, operand),
            Expr::Call { callee, args } => {
                let callee_op = self.convert_expr(callee);
                let arg_ops: Vec<Operand> = args.iter().map(|a| self.convert_expr(a)).collect();
                self.emit(InstKind::Call {
                    callee: callee_op,
                    args: arg_ops,
                })
            }
            Expr::Member { obj, prop } => {
                let obj_op = self.convert_expr(obj);
                let key = self.builder.intern(prop);
                let purity = property_access_purity(&obj_op);
                self.emit(InstKind::LoadProp {
                    obj: obj_op,
                    key,
                    purity,
                })
            }
            Expr::Index { obj, index } => {
                let obj_op = self.convert_expr(obj);
                let idx_op = self.convert_expr(index);
                let purity = property_access_purity(&obj_op);
                self.emit(InstKind::LoadIndex {
                    obj: obj_op,
                    index: idx_op,
                    purity,
                })
            }
            Expr::Func(_) => {
                // Stage 4 wires function expressions into AllocClosure.
                // See Stmt::FuncDecl arm above for the same reasoning —
                // lower.rs cannot yet emit Expr::Func from JS source, so
                // this arm is unreachable in production.
                panic!("Expr::Func reached SSA lowering before stage 4 wired closures")
            }
        }
    }

    /// Emit an instruction whose result is needed as an `Operand`. Panics if
    /// the InstKind is void (a store) — call sites use this only for
    /// value-producing kinds.
    fn emit(&mut self, kind: InstKind) -> Operand {
        let v = self
            .builder
            .add_inst(kind)
            .expect("emit() called on a void instruction");
        Operand::Value(v)
    }

    fn convert_binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> Operand {
        // Logical operators short-circuit, so they can't be lowered through
        // the value-producing LogAnd/LogOr instructions (which are pure,
        // both-sides-evaluated). Branch instead.
        if matches!(op, BinOp::LogAnd | BinOp::LogOr) {
            return self.convert_short_circuit(op, lhs, rhs);
        }

        let lhs_op = self.convert_expr(lhs);
        let rhs_op = self.convert_expr(rhs);
        let kind = match op {
            BinOp::Add => InstKind::AddGeneric {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::Sub => InstKind::SubGeneric {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::Mul => InstKind::MulGeneric {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::Div => InstKind::DivGeneric {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::Mod => InstKind::ModGeneric {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::Eq => InstKind::Eq {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::StrictEq => InstKind::StrictEq {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::Ne => {
                let inner = self.emit(InstKind::Eq {
                    lhs: lhs_op,
                    rhs: rhs_op,
                });
                return self.emit(InstKind::Not { operand: inner });
            }
            BinOp::StrictNe => {
                let inner = self.emit(InstKind::StrictEq {
                    lhs: lhs_op,
                    rhs: rhs_op,
                });
                return self.emit(InstKind::Not { operand: inner });
            }
            BinOp::Lt => InstKind::Lt {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::Le => InstKind::Le {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            // a > b  ≡  b < a; a >= b  ≡  b <= a. Reordering is sound only
            // because both operands are already evaluated above (left-to-right
            // observable order is preserved).
            BinOp::Gt => InstKind::Lt {
                lhs: rhs_op,
                rhs: lhs_op,
            },
            BinOp::Ge => InstKind::Le {
                lhs: rhs_op,
                rhs: lhs_op,
            },
            BinOp::LogAnd | BinOp::LogOr => unreachable!("handled above"),
            BinOp::BitAnd
            | BinOp::BitOr
            | BinOp::BitXor
            | BinOp::Shl
            | BinOp::Shr
            | BinOp::UShr => unimplemented!("bitwise operators not yet lowered"),
        };
        self.emit(kind)
    }

    fn convert_short_circuit(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> Operand {
        // && returns lhs if lhs is falsy, else rhs.
        // || returns lhs if lhs is truthy, else rhs.
        // Lowering: branch on lhs into either an "evaluate rhs" block or a
        // "skip" block that just forwards lhs. The join takes a single block
        // param holding the result.
        let lhs_op = self.convert_expr(lhs);

        let rhs_b = self.builder.new_block();
        let skip_b = self.builder.new_block();
        let join_b = self.builder.new_block();

        let (then_b, else_b) = match op {
            BinOp::LogAnd => (rhs_b, skip_b),
            BinOp::LogOr => (skip_b, rhs_b),
            _ => unreachable!(),
        };

        self.builder.set_terminator(Terminator::CondBranch {
            cond: lhs_op,
            then_b,
            then_args: vec![],
            else_b,
            else_args: vec![],
        });
        self.sealed.insert(rhs_b);
        self.sealed.insert(skip_b);

        // rhs evaluation block
        self.builder.switch_to_block(rhs_b);
        let rhs_op = self.convert_expr(rhs);
        let rhs_end = self.current_block();
        self.builder.body.blocks[rhs_end.0 as usize].terminator = Terminator::Branch {
            target: join_b,
            args: vec![rhs_op],
        };

        // skip block — forwards lhs unchanged
        self.builder.switch_to_block(skip_b);
        self.builder.set_terminator(Terminator::Branch {
            target: join_b,
            args: vec![lhs_op],
        });

        // join: result is the param. Manually add it (we know the arity)
        // so we don't go through Braun for a synthetic temporary.
        let result = self.builder.add_block_param(join_b);
        self.sealed.insert(join_b);
        self.builder.switch_to_block(join_b);
        Operand::Value(result)
    }

    fn convert_unary(&mut self, op: UnOp, operand: &Expr) -> Operand {
        let v = self.convert_expr(operand);
        let kind = match op {
            UnOp::Not => InstKind::Not { operand: v },
            UnOp::Pos => InstKind::ToNumber { val: v },
            UnOp::Typeof => InstKind::Typeof { val: v },
            UnOp::Void => {
                // `void e` evaluates e for effects, then yields undefined.
                // The convert_expr above already evaluated e.
                return Operand::Const(Constant::Undefined);
            }
            UnOp::Neg => unimplemented!("unary neg not yet lowered"),
            UnOp::BitNot => unimplemented!("bitwise not not yet lowered"),
        };
        self.emit(kind)
    }

    // ------------------------------------------------------------------
    // Helpers over the under-construction function
    // ------------------------------------------------------------------

    fn current_block(&self) -> BlockId {
        self.builder.current_block().expect("no current block")
    }

    fn is_terminated(&self, b: BlockId) -> bool {
        !matches!(
            self.builder.body.blocks[b.0 as usize].terminator,
            Terminator::Unreachable
        )
    }

    /// Predecessors of `target`, derived from terminator successors. Returns
    /// each source block exactly once even if multiple edges exist (shouldn't
    /// happen in our converter, but guards against degenerate IRs).
    fn predecessors_of(&self, target: BlockId) -> Vec<BlockId> {
        let mut out = Vec::new();
        for (i, blk) in self.builder.body.blocks.iter().enumerate() {
            for (succ, _) in blk.terminator.successors() {
                if succ == target {
                    let src = BlockId(i as u32);
                    if !out.contains(&src) {
                        out.push(src);
                    }
                    break;
                }
            }
        }
        out
    }

    /// Operand at `index` in `src`'s edge args targeting `dst`.
    fn edge_arg_at(&self, src: BlockId, dst: BlockId, index: usize) -> Option<Operand> {
        let blk = &self.builder.body.blocks[src.0 as usize];
        edge_args(&blk.terminator, dst).and_then(|args| args.get(index).copied())
    }

    /// Append `op` to `src`'s edge args targeting `dst`.
    fn append_edge_arg(&mut self, src: BlockId, dst: BlockId, op: Operand) {
        let blk = &mut self.builder.body.blocks[src.0 as usize];
        push_edge_arg(&mut blk.terminator, dst, op);
    }

    /// Strip the operand at `index` from `src`'s edge args targeting `dst`.
    fn remove_edge_arg_at(&mut self, src: BlockId, dst: BlockId, index: usize) {
        let blk = &mut self.builder.body.blocks[src.0 as usize];
        remove_edge_arg(&mut blk.terminator, dst, index);
    }

    /// Drop `block.params[idx]`, then shift the trailing params' indices
    /// down by one in their `ValueDef`s so block-param value lookups stay
    /// consistent.
    fn remove_block_param(&mut self, block: BlockId, idx: usize) {
        let blk = &mut self.builder.body.blocks[block.0 as usize];
        blk.params.remove(idx);
        // Snapshot indices to avoid borrowing both `blocks` and `values` at
        // once.
        let trailing: Vec<Value> = blk.params[idx..].to_vec();
        for (offset, v) in trailing.iter().enumerate() {
            if let ValueDefKind::BlockParam { index, .. } =
                &mut self.builder.body.values[v.0 as usize].kind
            {
                *index = (idx + offset) as u32;
            }
        }
    }

    /// Find every block-param Value whose incoming arg references `target`.
    /// Used to drive the recursive trivial-removal cascade after a removal:
    /// any block param that had `Operand::Value(target)` as one of its
    /// incomings may have just become trivial.
    fn find_block_param_users(&self, target: Value) -> Vec<Value> {
        let mut out = Vec::new();
        for blk in &self.builder.body.blocks {
            for (succ, args) in blk.terminator.successors() {
                let succ_blk = &self.builder.body.blocks[succ.0 as usize];
                for (i, arg) in args.iter().enumerate() {
                    if let Operand::Value(v) = arg
                        && *v == target
                        && i < succ_blk.params.len()
                    {
                        let user = succ_blk.params[i];
                        if !out.contains(&user) {
                            out.push(user);
                        }
                    }
                }
            }
        }
        out
    }

    /// Substitute every `Operand::Value(old)` with `new` across all
    /// instructions, all terminators, and the converter's own `current_def`.
    fn replace_value_uses(&mut self, old: Value, new: Operand) {
        for inst in &mut self.builder.body.instructions {
            walk_inst_operands_mut(&mut inst.kind, &mut |op| {
                if let Operand::Value(v) = op
                    && *v == old
                {
                    *op = new;
                }
            });
        }
        for blk in &mut self.builder.body.blocks {
            walk_term_operands_mut(&mut blk.terminator, &mut |op| {
                if let Operand::Value(v) = op
                    && *v == old
                {
                    *op = new;
                }
            });
        }
        for op in self.current_def.values_mut() {
            if let Operand::Value(v) = op
                && *v == old
            {
                *op = new;
            }
        }
    }
}

// ============================================================================
// Free helpers (don't need Converter state)
// ============================================================================

/// Recognize identifiers whose JS semantics is "always evaluate to a fixed
/// constant" — applied only when the name is not lexically bound (the caller
/// checks lookup() first, so a local `let undefined = 1` shadows correctly).
///
/// `NaN` and `Infinity` use the existing `Constant::Number` variants rather
/// than dedicated variants: a future constant-folding pass that produces NaN
/// from `0/0` (or Infinity from `1/0`) will yield the same canonical form,
/// avoiding two ways to spell the same value.
fn special_global_constant(name: &str) -> Option<Constant> {
    match name {
        "globalThis" | "window" | "self" | "global" => Some(Constant::GlobalThis),
        "undefined" => Some(Constant::Undefined),
        "NaN" => Some(Constant::Number(f64::NAN)),
        "Infinity" => Some(Constant::Number(f64::INFINITY)),
        _ => None,
    }
}

/// Pick the `Purity` tier for a property access whose object operand is
/// `obj`. Accesses rooted at `Const(GlobalThis)` get `Throws` (they read or
/// write the global record and may throw, but don't trigger user-defined
/// getters/setters under our semantic model); everything else stays
/// `Opaque`. Without this, `globalThis.foo` would lower to `Opaque` while
/// the bare unbound read of `foo` would lower to `Throws` — the same
/// observable JS denoted by two different SSA shapes, defeating CSE on
/// equivalent rewrites.
fn property_access_purity(obj: &Operand) -> Purity {
    match obj {
        Operand::Const(Constant::GlobalThis) => Purity::Throws,
        _ => Purity::Opaque,
    }
}

fn operand_eq(a: &Operand, b: &Operand) -> bool {
    match (a, b) {
        (Operand::Value(x), Operand::Value(y)) => x == y,
        (Operand::Const(x), Operand::Const(y)) => const_bit_eq(x, y),
        _ => false,
    }
}

fn const_bit_eq(a: &Constant, b: &Constant) -> bool {
    use Constant::*;
    match (a, b) {
        (Number(x), Number(y)) => x.to_bits() == y.to_bits(),
        (Int32(x), Int32(y)) => x == y,
        (Bool(x), Bool(y)) => x == y,
        (String(x), String(y)) => x == y,
        (Null, Null) => true,
        (Undefined, Undefined) => true,
        (Func(x), Func(y)) => x.0 == y.0,
        (GlobalThis, GlobalThis) => true,
        _ => false,
    }
}

/// Edge args from terminator `t` targeting `dst`. Returns the *first*
/// arg-list found that targets `dst`. Multiple-edge cases (switch with
/// duplicate targets, or a degenerate `cbr cond, L, L`) are not generated by
/// this converter.
fn edge_args(t: &Terminator, dst: BlockId) -> Option<&[Operand]> {
    match t {
        Terminator::Branch { target, args } if *target == dst => Some(args.as_slice()),
        Terminator::CondBranch {
            then_b, then_args, ..
        } if *then_b == dst => Some(then_args.as_slice()),
        Terminator::CondBranch {
            else_b, else_args, ..
        } if *else_b == dst => Some(else_args.as_slice()),
        Terminator::Switch {
            cases,
            default,
            default_args,
            ..
        } => {
            for c in cases {
                if c.target == dst {
                    return Some(c.args.as_slice());
                }
            }
            if *default == dst {
                Some(default_args.as_slice())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn push_edge_arg(t: &mut Terminator, dst: BlockId, op: Operand) {
    match t {
        Terminator::Branch { target, args } if *target == dst => args.push(op),
        Terminator::CondBranch {
            then_b,
            then_args,
            else_b,
            else_args,
            ..
        } => {
            if *then_b == dst {
                then_args.push(op);
            }
            if *else_b == dst {
                else_args.push(op);
            }
        }
        Terminator::Switch {
            cases,
            default,
            default_args,
            ..
        } => {
            for c in cases.iter_mut() {
                if c.target == dst {
                    c.args.push(op);
                }
            }
            if *default == dst {
                default_args.push(op);
            }
        }
        _ => panic!("push_edge_arg: terminator does not target bb{}", dst.0),
    }
}

fn remove_edge_arg(t: &mut Terminator, dst: BlockId, index: usize) {
    match t {
        Terminator::Branch { target, args } if *target == dst => {
            args.remove(index);
        }
        Terminator::CondBranch {
            then_b,
            then_args,
            else_b,
            else_args,
            ..
        } => {
            if *then_b == dst {
                then_args.remove(index);
            }
            if *else_b == dst {
                else_args.remove(index);
            }
        }
        Terminator::Switch {
            cases,
            default,
            default_args,
            ..
        } => {
            for c in cases.iter_mut() {
                if c.target == dst {
                    c.args.remove(index);
                }
            }
            if *default == dst {
                default_args.remove(index);
            }
        }
        _ => panic!("remove_edge_arg: terminator does not target bb{}", dst.0),
    }
}

// ----------------------------------------------------------------------
// Mutable operand walks. Mirrors the immutable walks in `ssa.rs` but
// threads `&mut Operand` so trivial-removal can substitute in place.
// ----------------------------------------------------------------------

fn walk_inst_operands_mut(kind: &mut InstKind, f: &mut impl FnMut(&mut Operand)) {
    use InstKind::*;
    match kind {
        AddGeneric { lhs, rhs }
        | SubGeneric { lhs, rhs }
        | MulGeneric { lhs, rhs }
        | DivGeneric { lhs, rhs }
        | ModGeneric { lhs, rhs }
        | AddNumber { lhs, rhs }
        | SubNumber { lhs, rhs }
        | MulNumber { lhs, rhs }
        | AddInt32 { lhs, rhs }
        | SubInt32 { lhs, rhs }
        | MulInt32 { lhs, rhs }
        | ConcatStr { lhs, rhs }
        | LogAnd { lhs, rhs }
        | LogOr { lhs, rhs }
        | Eq { lhs, rhs }
        | StrictEq { lhs, rhs }
        | Lt { lhs, rhs }
        | Le { lhs, rhs } => {
            f(lhs);
            f(rhs);
        }
        Not { operand } => f(operand),
        LoadProp { obj, .. } => f(obj),
        StoreProp { obj, val, .. } => {
            f(obj);
            f(val);
        }
        LoadIndex { obj, index, .. } => {
            f(obj);
            f(index);
        }
        StoreIndex {
            obj, index, val, ..
        } => {
            f(obj);
            f(index);
            f(val);
        }
        AllocObject | AllocArray { .. } => {}
        AllocClosure { captures, .. } => {
            for c in captures {
                f(c);
            }
        }
        Call { callee, args } => {
            f(callee);
            for a in args {
                f(a);
            }
        }
        CallMethod { obj, args, .. } => {
            f(obj);
            for a in args {
                f(a);
            }
        }
        Construct { ctor, args } => {
            f(ctor);
            for a in args {
                f(a);
            }
        }
        ToNumber { val }
        | ToString { val }
        | ToBool { val }
        | ToInt32 { val }
        | Typeof { val }
        | OpaqueEval { source: val } => f(val),
        Instanceof { obj, ctor } => {
            f(obj);
            f(ctor);
        }
    }
}

fn walk_term_operands_mut(t: &mut Terminator, f: &mut impl FnMut(&mut Operand)) {
    match t {
        Terminator::Branch { args, .. } => {
            for a in args {
                f(a);
            }
        }
        Terminator::CondBranch {
            cond,
            then_args,
            else_args,
            ..
        } => {
            f(cond);
            for a in then_args {
                f(a);
            }
            for a in else_args {
                f(a);
            }
        }
        Terminator::Switch {
            val,
            cases,
            default_args,
            ..
        } => {
            f(val);
            for c in cases {
                for a in &mut c.args {
                    f(a);
                }
            }
            for a in default_args {
                f(a);
            }
        }
        Terminator::Return { val: Some(v) } => f(v),
        Terminator::Return { val: None } => {}
        Terminator::Throw { val } => f(val),
        Terminator::Unreachable => {}
    }
}

// Silence dead-code lint on `BasicBlock` import — currently re-exported only
// because `walk_*_mut` and helpers may grow to need direct access in future
// changes. Kept here so the dependency is explicit.
#[allow(dead_code)]
fn _unused_basic_block(_: &BasicBlock) {}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::ssa::{Terminator, pretty_module};
    use crate::lower::parse_and_lower;

    fn lower_js(src: &str) -> ssa::Module {
        let tm = parse_and_lower(src).expect("tree-IR lowering must succeed");
        let sm = lower_module(&tm);
        sm.validate().unwrap_or_else(|errs| {
            panic!(
                "SSA module failed validation:\n{:#?}\n--- dump ---\n{}",
                errs,
                pretty_module(&sm)
            );
        });
        sm
    }

    /// Count live block params (skipping any `Removed` slots, which would
    /// indicate a bug — block.params is shifted to compact on removal).
    fn live_param_count(func: &ssa::Function, b: BlockId) -> usize {
        func.lowered().blocks[b.0 as usize].params.len()
    }

    #[test]
    fn fibonacci_recursion() {
        let src = r#"
            function fibonacci(n) {
                if (n <= 1) return n;
                return fibonacci(n - 1) + fibonacci(n - 2);
            }
        "#;
        let m = lower_js(src);
        insta::assert_snapshot!(pretty_module(&m));
    }

    #[test]
    fn sum_to_loop() {
        let src = r#"
            function sum_to(n) {
                let i = 0;
                let s = 0;
                while (i < n) {
                    s = s + i;
                    i = i + 1;
                }
                return s;
            }
        "#;
        let m = lower_js(src);

        // The header must carry block params for both i and s (both vary
        // across the back-edge). Header is bb1 (entry=bb0, then header=bb1
        // by construction order).
        let header = BlockId(1);
        assert_eq!(
            live_param_count(&m.functions[0], header),
            2,
            "header should carry params for i and s"
        );

        insta::assert_snapshot!(pretty_module(&m));
    }

    #[test]
    fn if_else_shared_mutation() {
        let src = r#"
            function f(c) {
                let x = 1;
                if (c) { x = 2; } else { x = 3; }
                return x;
            }
        "#;
        let m = lower_js(src);

        // The lowering is: entry=bb0, then_b=bb1, else_b=bb2, join=bb3.
        // join must carry exactly one param (for x).
        let join = BlockId(3);
        assert_eq!(
            live_param_count(&m.functions[0], join),
            1,
            "join block should carry exactly one param (for x)"
        );

        // The branches' edge args should be Const(2) and Const(3)
        // respectively — no instructions emitted in the branch bodies.
        let then_b = &m.functions[0].lowered().blocks[1];
        let else_b = &m.functions[0].lowered().blocks[2];
        match &then_b.terminator {
            Terminator::Branch { target, args } => {
                assert_eq!(*target, join);
                assert_eq!(args.len(), 1);
                assert!(matches!(args[0], Operand::Const(Constant::Number(n)) if n == 2.0));
            }
            t => panic!("then branch should jump to join; got {t:?}"),
        }
        match &else_b.terminator {
            Terminator::Branch { target, args } => {
                assert_eq!(*target, join);
                assert_eq!(args.len(), 1);
                assert!(matches!(args[0], Operand::Const(Constant::Number(n)) if n == 3.0));
            }
            t => panic!("else branch should jump to join; got {t:?}"),
        }

        insta::assert_snapshot!(pretty_module(&m));
    }

    #[test]
    fn lexical_shadowing() {
        let src = r#"
            function f() {
                let x = 1;
                if (true) { let x = 2; }
                return x;
            }
        "#;
        let m = lower_js(src);

        // The return reads the OUTER x, which was assigned 1 and is
        // unchanged (the inner x = 2 binds a different SymbolId). After
        // trivial-param removal, the return should literally be `ret 1`.
        let last_block = m.functions[0]
            .lowered()
            .blocks
            .iter()
            .rev()
            .find(|b| matches!(b.terminator, Terminator::Return { val: Some(_) }));
        let ret_block = last_block.expect("function should have a Return terminator");
        match &ret_block.terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => {
                assert_eq!(*n, 1.0, "return should yield outer x = 1");
            }
            t => panic!("return should be a literal Number(1); got {t:?}"),
        }

        insta::assert_snapshot!(pretty_module(&m));
    }

    #[test]
    fn trivial_param_elimination() {
        // The while loop modifies `i` (loop-varying) but not `x`
        // (loop-invariant). Braun would speculatively add a header param
        // for `x` on demand, but trivial-removal must collapse it because
        // both incomings (entry=Const(1), back-edge=same param) reduce to
        // Const(1).
        let src = r#"
            function f(n) {
                let x = 1;
                let i = 0;
                while (i < n) {
                    i = i + 1;
                }
                return x;
            }
        "#;
        let m = lower_js(src);

        // header is bb1 (entry=bb0, then header). It should carry exactly
        // one param — for `i`. If `x` survived the trivial-removal pass,
        // the count would be ≥2.
        let header = BlockId(1);
        assert_eq!(
            live_param_count(&m.functions[0], header),
            1,
            "header should carry exactly one param (for i); x must be removed"
        );

        // The exit block's return should read the constant 1, since x is
        // never modified in the loop.
        let exit_block = m.functions[0]
            .lowered()
            .blocks
            .iter()
            .rev()
            .find(|b| matches!(b.terminator, Terminator::Return { val: Some(_) }));
        let exit = exit_block.expect("function should have a Return terminator");
        match &exit.terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => {
                assert_eq!(*n, 1.0, "x is loop-invariant; return should fold to 1");
            }
            t => panic!("return should be a literal Number(1); got {t:?}"),
        }

        // No `Removed` ValueDef should appear under any *referenced* slot;
        // the validator already enforces this, but assert it explicitly to
        // catch regressions in trivial-removal bookkeeping.
        let removed_count = m.functions[0]
            .lowered()
            .values
            .iter()
            .filter(|v| v.is_removed())
            .count();
        // We expect at least one Removed entry (the eliminated param).
        assert!(
            removed_count >= 1,
            "expected at least one Removed value-def from trivial-param elimination"
        );

        insta::assert_snapshot!(pretty_module(&m));
    }

    // ====================================================================
    // Globals model
    // ====================================================================

    /// `console.log("hi")` survives the SSA pass without panicking and
    /// produces a `LoadProp` rooted at `Const(GlobalThis)` for the global
    /// read of `console`. The `Throws` purity is the key signal: it's the
    /// proof that we're using the new precision tier rather than collapsing
    /// to Opaque.
    #[test]
    fn unbound_console_call_lowers_to_global_load() {
        let src = r#"function f() { console.log("hi"); }"#;
        let m = lower_js(src);

        let mut saw_global_load = false;
        for inst in &m.functions[0].lowered().instructions {
            if let InstKind::LoadProp {
                obj: Operand::Const(Constant::GlobalThis),
                key,
                purity,
            } = inst.kind
            {
                assert_eq!(purity, Purity::Throws, "global read should use Throws");
                assert_eq!(m.strings.resolve(&key), "console");
                saw_global_load = true;
            }
        }
        assert!(
            saw_global_load,
            "expected a LoadProp(GlobalThis, \"console\") instruction"
        );

        insta::assert_snapshot!(pretty_module(&m));
    }

    /// `Math.floor(x)` exercises the same path as `console.log` plus a
    /// member-call rather than a method-call (the front end currently
    /// produces `Call(LoadProp(...), args)`, not `CallMethod`).
    #[test]
    fn unbound_math_floor_lowers_to_global_load() {
        let src = r#"function f(x) { return Math.floor(x); }"#;
        let m = lower_js(src);

        let saw_math = m.functions[0].lowered().instructions.iter().any(|i| {
            matches!(
                i.kind,
                InstKind::LoadProp {
                    obj: Operand::Const(Constant::GlobalThis),
                    purity: Purity::Throws,
                    ..
                }
            )
        });
        assert!(saw_math, "expected a LoadProp(GlobalThis, \"Math\")");

        insta::assert_snapshot!(pretty_module(&m));
    }

    /// `x = 1` with `x` unbound becomes a global-store rooted at
    /// `Const(GlobalThis)`. Mirror of the read-side test.
    #[test]
    fn unbound_write_lowers_to_global_store() {
        let src = r#"function f() { x = 1; }"#;
        let m = lower_js(src);

        let mut saw_global_store = false;
        for inst in &m.functions[0].lowered().instructions {
            if let InstKind::StoreProp {
                obj: Operand::Const(Constant::GlobalThis),
                key,
                purity,
                ..
            } = inst.kind
            {
                assert_eq!(purity, Purity::Throws, "global write should use Throws");
                assert_eq!(m.strings.resolve(&key), "x");
                saw_global_store = true;
            }
        }
        assert!(
            saw_global_store,
            "expected a StoreProp(GlobalThis, \"x\", _) instruction"
        );

        insta::assert_snapshot!(pretty_module(&m));
    }

    /// `let y = undefined; return y;` — `undefined` is a special-case
    /// constant lookup, not a global read. Validates that `Constant::
    /// Undefined` (not a global LoadProp) reaches the return after
    /// trivial-param removal collapses the chain.
    #[test]
    fn undefined_lowers_to_constant_undefined() {
        let src = r#"function f() { let y = undefined; return y; }"#;
        let m = lower_js(src);

        let ret_block = m.functions[0]
            .lowered()
            .blocks
            .iter()
            .find(|b| matches!(b.terminator, Terminator::Return { val: Some(_) }))
            .expect("function should have a Return");
        match &ret_block.terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Undefined)),
            } => {}
            t => panic!("expected `ret undefined`; got {t:?}"),
        }
    }

    /// `NaN` and `Infinity` use `Constant::Number(f64::NAN)` and
    /// `Constant::Number(f64::INFINITY)` — the existing canonical form
    /// rather than dedicated variants. This is the Q1 design decision: a
    /// future constant-folding pass that produces NaN from `0/0` agrees
    /// with the unbound-ident reads on the same canonical form, avoiding
    /// two ways to spell the same value.
    #[test]
    fn nan_and_infinity_use_number_constants() {
        for (src, expected_bits) in [
            ("function f() { return NaN; }", f64::NAN.to_bits()),
            ("function f() { return Infinity; }", f64::INFINITY.to_bits()),
        ] {
            let m = lower_js(src);
            let ret_block = m.functions[0]
                .lowered()
                .blocks
                .iter()
                .find(|b| matches!(b.terminator, Terminator::Return { val: Some(_) }))
                .expect("function should have a Return");
            match &ret_block.terminator {
                Terminator::Return {
                    val: Some(Operand::Const(Constant::Number(n))),
                } => {
                    assert_eq!(
                        n.to_bits(),
                        expected_bits,
                        "expected canonical Number constant for src `{src}`"
                    );
                }
                t => panic!("expected `ret <Number>`; got {t:?} for src `{src}`"),
            }
        }
    }

    /// SSA pretty-printer annotates Inst-defined values with their source
    /// name, e.g. `%2["x"] = add ...`. Visible only when source_name was
    /// populated by lowering.
    #[test]
    fn ssa_dump_includes_source_name_annotation() {
        let src = r#"function f() { let x = 1 + 2; return x; }"#;
        let m = lower_js(src);
        let dump = pretty_module(&m);
        assert!(
            dump.contains("[\"x\"]"),
            "expected source-name annotation `[\"x\"]` in dump:\n{dump}"
        );
    }

    /// `globalThis.foo` (member access on the explicit `globalThis` ident)
    /// produces SSA byte-identical to the bare `foo`. Both share the same
    /// `LoadProp(GlobalThis, "foo", Throws)` shape — `globalThis` is
    /// recognized as a special-case constant on the read side, and the
    /// Member arm picks `Throws` purity for GlobalThis-rooted access.
    /// This is a *lossy* normalization: the user can no longer tell which
    /// spelling the source used, which is the correct trade-off for a
    /// deobfuscator (the obfuscator's choice of spelling is noise).
    #[test]
    fn globalthis_dot_foo_normalizes_to_bare_foo() {
        let with_explicit = lower_js("function f() { return globalThis.foo; }");
        let bare = lower_js("function f() { return foo; }");
        assert_eq!(
            pretty_module(&with_explicit),
            pretty_module(&bare),
            "globalThis.foo and bare foo must produce identical SSA"
        );
    }

    /// A local `let console = 1;` shadows the special-case lookup —
    /// reads of `console` resolve via the normal Braun path to the local
    /// binding, never to `LoadProp(GlobalThis, "console")`. The whole
    /// shadowing-supports-globals contract relies on lookup() running
    /// before special-case resolution.
    #[test]
    fn lexical_binding_shadows_global() {
        let src = r#"function f() { let console = 1; return console; }"#;
        let m = lower_js(src);

        // No global LoadProp anywhere in the function.
        let leaked = m.functions[0].lowered().instructions.iter().any(|i| {
            matches!(
                i.kind,
                InstKind::LoadProp {
                    obj: Operand::Const(Constant::GlobalThis),
                    ..
                }
            )
        });
        assert!(!leaked, "local `console` must shadow the global lookup");

        // After trivial-param removal the return collapses to `ret 1`.
        let ret_block = m.functions[0]
            .lowered()
            .blocks
            .iter()
            .find(|b| matches!(b.terminator, Terminator::Return { val: Some(_) }))
            .expect("function should have a Return");
        match &ret_block.terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => {
                assert_eq!(*n, 1.0, "shadowed `console` should return 1");
            }
            t => panic!("expected `ret 1`; got {t:?}"),
        }
    }
}
