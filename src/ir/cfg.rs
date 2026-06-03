//! CFG construction: lower a function body to basic blocks that record the
//! control-flow edges plus, in execution order, the def/use *events* of
//! candidate (versionable) symbols.
//!
//! Scope (deliberately narrow, plan §5): the CFG does not model arbitrary
//! expression values. It models exactly what intraprocedural SSA over the
//! candidate symbols needs — branch structure (so dominance is right, including
//! `&&`/`||`/`??`/`?:` short-circuits) and the ordered defs/uses of candidates.
//! Anything whose control flow or evaluation order we don't model exactly ⇒
//! [`Lowerer::bailed`] is set and the caller discards the result, leaving the
//! function's AST untouched. Partial coverage is always sound.
//!
//! Every *write* to a surviving candidate is one of the two forms the lowerer
//! emits a `Def` for (simple `s = rhs`, or a `var/let/const` declarator); the
//! escape analysis (see `analysis.rs`) excludes any symbol written any other way,
//! so the CFG provably sees every definition of every candidate it tracks. That
//! is the property the SSA reaching-def soundness rests on.

use std::collections::HashMap;

use oxc_allocator::{Address, Allocator, CloneIn, GetAddress};
use oxc_ast::ast::*;
use oxc_semantic::Scoping;
use oxc_syntax::symbol::SymbolId;

use crate::util::is_inlineable_literal;

pub type BlockId = usize;

/// Stable identity of an AST expression node (arena address), used to map an IR
/// use back to the exact node to rewrite. Stable for the arena's lifetime and
/// unchanged by the read-only analysis, so it matches in the later mutable walk.
pub type NodeId = Address;

/// A resolved static member key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Index(usize),
    Name(String),
}

/// The classified value of a definition. Aggregate/scalar values are stored as
/// *owned clones* of the immutable literal parts (no borrow of the AST), so the
/// IR carries everything the transform needs without holding the tree.
pub enum DefVal<'a> {
    /// `s = <inlineable literal>` — the cloned literal.
    Scalar(Expression<'a>),
    /// `s = [ ... ]` — per-index clone of each *inlineable-literal* element
    /// (`None` for holes / non-literal elements, which never fold).
    Array(Vec<Option<Expression<'a>>>),
    /// `s = { ... }` — name→clone for each inlineable-literal-valued property,
    /// last-writer-wins. Only built when the object has no spread/computed key.
    Object(HashMap<String, Expression<'a>>),
    /// Anything else: an opaque value that never folds.
    Opaque,
}

/// How a candidate symbol is read at a use site.
#[derive(Clone)]
pub enum UseKind {
    /// A bare read `s` in value position (scalar const propagation target).
    Scalar,
    /// A member read `s[k]` / `s.name` (const-aggregate fold target).
    Member(Key),
}

pub enum Event<'a> {
    Def { sym: SymbolId, val: DefVal<'a> },
    Use { sym: SymbolId, kind: UseKind, node: NodeId },
}

pub struct Block<'a> {
    pub events: Vec<Event<'a>>,
}

pub struct Cfg<'a> {
    pub blocks: Vec<Block<'a>>,
    pub succs: Vec<Vec<BlockId>>,
    pub entry: BlockId,
}

struct LoopCtx {
    label: Option<String>,
    is_loop: bool,
    brk: BlockId,
    cont: BlockId,
}

/// Lowers a statement list to a [`Cfg`]. Borrows the AST immutably; clones only
/// the immutable literal parts it needs into the arena.
pub struct Lowerer<'a, 'c> {
    allocator: &'a Allocator,
    scoping: &'c Scoping,
    cands: &'c std::collections::HashSet<SymbolId>,
    blocks: Vec<Block<'a>>,
    succs: Vec<Vec<BlockId>>,
    cur: BlockId,
    loops: Vec<LoopCtx>,
    /// Set just before lowering a labeled loop so the loop lowerer can attach the
    /// label to the context it pushes (for `break label` / `continue label`).
    pending_label: Option<String>,
    pub bailed: bool,
}

impl<'a, 'c> Lowerer<'a, 'c> {
    pub fn new(
        allocator: &'a Allocator,
        scoping: &'c Scoping,
        cands: &'c std::collections::HashSet<SymbolId>,
    ) -> Self {
        let mut l = Lowerer {
            allocator,
            scoping,
            cands,
            blocks: Vec::new(),
            succs: Vec::new(),
            cur: 0,
            loops: Vec::new(),
            pending_label: None,
            bailed: false,
        };
        let entry = l.new_block();
        l.cur = entry;
        l
    }

    /// Lower the given statements; returns the CFG, or `None` if lowering bailed.
    pub fn lower(mut self, stmts: &[Statement<'a>]) -> Option<Cfg<'a>> {
        self.lower_stmts(stmts);
        if self.bailed {
            return None;
        }
        Some(Cfg { blocks: self.blocks, succs: self.succs, entry: 0 })
    }

    fn new_block(&mut self) -> BlockId {
        let id = self.blocks.len();
        self.blocks.push(Block { events: Vec::new() });
        self.succs.push(Vec::new());
        id
    }

    fn add_edge(&mut self, from: BlockId, to: BlockId) {
        self.succs[from].push(to);
    }

    fn emit(&mut self, ev: Event<'a>) {
        self.blocks[self.cur].events.push(ev);
    }

    fn ref_sym(&self, idref: &IdentifierReference<'a>) -> Option<SymbolId> {
        let rid = idref.reference_id.get()?;
        self.scoping.get_reference(rid).symbol_id()
    }

    fn is_cand_ident(&self, e: &Expression<'a>) -> Option<SymbolId> {
        let Expression::Identifier(idref) = e else { return None };
        let sym = self.ref_sym(idref)?;
        if self.cands.contains(&sym) {
            Some(sym)
        } else {
            None
        }
    }

    // --- statements -------------------------------------------------------

    fn lower_stmts(&mut self, stmts: &[Statement<'a>]) {
        for s in stmts {
            if self.bailed {
                return;
            }
            self.lower_stmt(s);
        }
    }

    fn lower_stmt(&mut self, s: &Statement<'a>) {
        match s {
            Statement::ExpressionStatement(e) => self.lower_expr(&e.expression),
            Statement::EmptyStatement(_) | Statement::DebuggerStatement(_) => {}
            Statement::VariableDeclaration(d) => self.lower_var_decl(d),
            Statement::BlockStatement(b) => self.lower_stmts(&b.body),
            Statement::IfStatement(i) => self.lower_if(i),
            Statement::WhileStatement(w) => self.lower_while(w),
            Statement::DoWhileStatement(w) => self.lower_do_while(w),
            Statement::ForStatement(f) => self.lower_for(f),
            Statement::ForOfStatement(f) => self.lower_for_each(&f.right, &f.body),
            Statement::ForInStatement(f) => self.lower_for_each(&f.right, &f.body),
            Statement::SwitchStatement(sw) => self.lower_switch(sw),
            Statement::BreakStatement(b) => self.lower_jump(b.label.as_ref().map(|l| l.name.as_str()), false),
            Statement::ContinueStatement(c) => self.lower_jump(c.label.as_ref().map(|l| l.name.as_str()), true),
            Statement::ReturnStatement(r) => {
                if let Some(arg) = &r.argument {
                    self.lower_expr(arg);
                }
                self.cur = self.new_block(); // unreachable tail
            }
            Statement::ThrowStatement(t) => {
                self.lower_expr(&t.argument);
                self.cur = self.new_block();
            }
            Statement::LabeledStatement(l) => self.lower_labeled(l),
            // Function/class declarations introduce a binding but no control flow
            // here; their bodies are separate units (captured candidates are
            // already excluded by escape analysis), so we do not recurse.
            Statement::FunctionDeclaration(_) | Statement::ClassDeclaration(_) => {}
            // try/with and anything we don't recognise: bail (sound).
            _ => self.bailed = true,
        }
    }

    fn lower_var_decl(&mut self, d: &VariableDeclaration<'a>) {
        for decl in &d.declarations {
            if let Some(init) = &decl.init {
                self.lower_expr(init);
            }
            if let BindingPattern::BindingIdentifier(id) = &decl.id {
                if let Some(sym) = id.symbol_id.get() {
                    if self.cands.contains(&sym) {
                        let val = match &decl.init {
                            Some(init) => self.classify_def(init),
                            None => DefVal::Opaque, // `var x;` → undefined, not folded
                        };
                        self.emit(Event::Def { sym, val });
                    }
                }
            }
            // Non-identifier binding patterns declare only non-candidate symbols
            // (escape analysis admits BindingIdentifier defs only); their init
            // was already lowered, so uses inside are captured.
        }
    }

    fn lower_if(&mut self, i: &IfStatement<'a>) {
        self.lower_expr(&i.test);
        let after = self.cur;
        let then_b = self.new_block();
        let cont = self.new_block();
        self.add_edge(after, then_b);
        self.cur = then_b;
        self.lower_stmt(&i.consequent);
        let then_end = self.cur;
        self.add_edge(then_end, cont);
        if let Some(alt) = &i.alternate {
            let else_b = self.new_block();
            self.add_edge(after, else_b);
            self.cur = else_b;
            self.lower_stmt(alt);
            let else_end = self.cur;
            self.add_edge(else_end, cont);
        } else {
            self.add_edge(after, cont);
        }
        self.cur = cont;
    }

    fn lower_while(&mut self, w: &WhileStatement<'a>) {
        let label = self.pending_label.take();
        let after = self.cur;
        let header = self.new_block();
        self.add_edge(after, header);
        self.cur = header;
        self.lower_expr(&w.test);
        let test_end = self.cur;
        let body = self.new_block();
        let cont = self.new_block();
        self.add_edge(test_end, body);
        self.add_edge(test_end, cont);
        self.loops.push(LoopCtx { label, is_loop: true, brk: cont, cont: header });
        self.cur = body;
        self.lower_stmt(&w.body);
        let body_end = self.cur;
        self.add_edge(body_end, header);
        self.loops.pop();
        self.cur = cont;
    }

    fn lower_do_while(&mut self, w: &DoWhileStatement<'a>) {
        let label = self.pending_label.take();
        let after = self.cur;
        let body = self.new_block();
        let latch = self.new_block();
        let cont = self.new_block();
        self.add_edge(after, body);
        self.loops.push(LoopCtx { label, is_loop: true, brk: cont, cont: latch });
        self.cur = body;
        self.lower_stmt(&w.body);
        let body_end = self.cur;
        self.add_edge(body_end, latch);
        self.loops.pop();
        self.cur = latch;
        self.lower_expr(&w.test);
        let latch_end = self.cur;
        self.add_edge(latch_end, body);
        self.add_edge(latch_end, cont);
        self.cur = cont;
    }

    fn lower_for(&mut self, f: &ForStatement<'a>) {
        let for_label = self.pending_label.take();
        if let Some(init) = &f.init {
            match init {
                ForStatementInit::VariableDeclaration(d) => self.lower_var_decl(d),
                expr => {
                    if let Some(e) = expr.as_expression() {
                        self.lower_expr(e);
                    }
                }
            }
        }
        let after = self.cur;
        let header = self.new_block();
        self.add_edge(after, header);
        self.cur = header;
        if let Some(test) = &f.test {
            self.lower_expr(test);
        }
        let test_end = self.cur;
        let body = self.new_block();
        let update_b = self.new_block();
        let cont = self.new_block();
        self.add_edge(test_end, body);
        self.add_edge(test_end, cont);
        self.loops.push(LoopCtx { label: for_label, is_loop: true, brk: cont, cont: update_b });
        self.cur = body;
        self.lower_stmt(&f.body);
        let body_end = self.cur;
        self.add_edge(body_end, update_b);
        self.loops.pop();
        self.cur = update_b;
        if let Some(update) = &f.update {
            self.lower_expr(update);
        }
        let update_end = self.cur;
        self.add_edge(update_end, header);
        self.cur = cont;
    }

    /// for-in / for-of, modelled conservatively: the loop binding gets no const
    /// knowledge (its symbol, if a candidate, was excluded by escape analysis as
    /// a non-simple write), and the body may run 0+ times.
    fn lower_for_each(&mut self, right: &Expression<'a>, body: &Statement<'a>) {
        let label = self.pending_label.take();
        self.lower_expr(right);
        let after = self.cur;
        let header = self.new_block();
        let body_b = self.new_block();
        let cont = self.new_block();
        self.add_edge(after, header);
        self.add_edge(header, body_b);
        self.add_edge(header, cont);
        self.loops.push(LoopCtx { label, is_loop: true, brk: cont, cont: header });
        self.cur = body_b;
        self.lower_stmt(body);
        let body_end = self.cur;
        self.add_edge(body_end, header);
        self.loops.pop();
        self.cur = cont;
    }

    fn lower_switch(&mut self, sw: &SwitchStatement<'a>) {
        self.lower_expr(&sw.discriminant);
        let cont = self.new_block();
        let bodies: Vec<BlockId> = sw.cases.iter().map(|_| self.new_block()).collect();

        // Test chain: each case's test evaluates in its own block, branching to
        // the case body on match or the next test on miss. `default` has no test.
        let mut test_cur = self.cur;
        let mut default_idx = None;
        for (i, case) in sw.cases.iter().enumerate() {
            if let Some(test) = &case.test {
                self.cur = test_cur;
                self.lower_expr(test);
                let te = self.cur;
                self.add_edge(te, bodies[i]);
                let next_test = self.new_block();
                self.add_edge(te, next_test);
                test_cur = next_test;
            } else {
                default_idx = Some(i);
            }
        }
        self.add_edge(test_cur, default_idx.map(|i| bodies[i]).unwrap_or(cont));

        self.loops.push(LoopCtx { label: None, is_loop: false, brk: cont, cont });
        for (i, case) in sw.cases.iter().enumerate() {
            self.cur = bodies[i];
            self.lower_stmts(&case.consequent);
            let end = self.cur;
            let fall = if i + 1 < bodies.len() { bodies[i + 1] } else { cont };
            self.add_edge(end, fall);
        }
        self.loops.pop();
        self.cur = cont;
    }

    fn lower_jump(&mut self, label: Option<&str>, is_continue: bool) {
        let mut target = None;
        for ctx in self.loops.iter().rev() {
            let matches = match label {
                None => ctx.is_loop || !is_continue, // continue needs a loop; break takes loop or switch
                Some(l) => ctx.label.as_deref() == Some(l),
            };
            if matches {
                target = Some(if is_continue { ctx.cont } else { ctx.brk });
                break;
            }
        }
        match target {
            Some(t) => {
                self.add_edge(self.cur, t);
                self.cur = self.new_block(); // unreachable tail
            }
            None => self.bailed = true,
        }
    }

    fn lower_labeled(&mut self, l: &LabeledStatement<'a>) {
        let label = l.label.name.to_string();
        match &l.body {
            Statement::WhileStatement(_)
            | Statement::DoWhileStatement(_)
            | Statement::ForStatement(_)
            | Statement::ForOfStatement(_)
            | Statement::ForInStatement(_) => {
                // Re-lower the loop, then tag the just-pushed loop context's label.
                // Simplest: temporarily lower with a labeled break/continue target
                // by wrapping. We lower the loop and patch the label onto the
                // LoopCtx it pushes — but the ctx is popped inside. Instead, set a
                // pending label the loop lowerers consult.
                self.pending_label = Some(label);
                self.lower_stmt(&l.body);
                self.pending_label = None;
            }
            other => {
                // Labeled non-loop block: only `break label` applies.
                let cont = self.new_block();
                self.loops.push(LoopCtx { label: Some(label), is_loop: false, brk: cont, cont });
                self.lower_stmt(other);
                let end = self.cur;
                self.add_edge(end, cont);
                self.loops.pop();
                self.cur = cont;
            }
        }
    }

    // --- expressions ------------------------------------------------------

    fn lower_expr(&mut self, e: &Expression<'a>) {
        if self.bailed {
            return;
        }
        match e {
            Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::BigIntLiteral(_)
            | Expression::RegExpLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::ThisExpression(_)
            | Expression::Super(_)
            | Expression::MetaProperty(_) => {}

            Expression::Identifier(idref) => {
                if let Some(sym) = self.ref_sym(idref) {
                    if self.cands.contains(&sym) {
                        // A bare read in value position (writes never reach here:
                        // they are handled at the assignment/declarator level, and
                        // escape analysis excludes any candidate with a non-simple
                        // write). Scalar const-prop target.
                        self.emit(Event::Use { sym, kind: UseKind::Scalar, node: e.address() });
                    }
                }
            }

            Expression::ArrayExpression(a) => {
                for el in &a.elements {
                    match el {
                        ArrayExpressionElement::SpreadElement(s) => self.lower_expr(&s.argument),
                        ArrayExpressionElement::Elision(_) => {}
                        other => {
                            if let Some(ex) = other.as_expression() {
                                self.lower_expr(ex);
                            }
                        }
                    }
                }
            }

            Expression::ObjectExpression(o) => {
                for prop in &o.properties {
                    match prop {
                        ObjectPropertyKind::ObjectProperty(p) => {
                            if p.computed {
                                self.lower_property_key(&p.key);
                            }
                            // Function-valued properties are nested scopes; skip.
                            if !matches!(
                                p.value,
                                Expression::FunctionExpression(_)
                                    | Expression::ArrowFunctionExpression(_)
                            ) {
                                self.lower_expr(&p.value);
                            }
                        }
                        ObjectPropertyKind::SpreadProperty(s) => self.lower_expr(&s.argument),
                    }
                }
            }

            Expression::StaticMemberExpression(m) => {
                if let Some(sym) = self.is_cand_ident(&m.object) {
                    self.emit(Event::Use {
                        sym,
                        kind: UseKind::Member(Key::Name(m.property.name.to_string())),
                        node: e.address(),
                    });
                } else {
                    self.lower_expr(&m.object);
                }
            }

            Expression::ComputedMemberExpression(m) => {
                if let Some(sym) = self.is_cand_ident(&m.object) {
                    if let Some(key) = static_key(&m.expression) {
                        self.emit(Event::Use {
                            sym,
                            kind: UseKind::Member(key),
                            node: e.address(),
                        });
                        // index is a literal here — no candidate uses inside.
                        return;
                    }
                    // Non-const index: object is still read, but not foldable.
                    // Lower the index for its own uses/defs.
                    self.lower_expr(&m.expression);
                } else {
                    self.lower_expr(&m.object);
                    self.lower_expr(&m.expression);
                }
            }

            Expression::CallExpression(c) => {
                self.lower_expr(&c.callee);
                for arg in &c.arguments {
                    self.lower_argument(arg);
                }
            }
            Expression::NewExpression(c) => {
                self.lower_expr(&c.callee);
                for arg in &c.arguments {
                    self.lower_argument(arg);
                }
            }

            Expression::BinaryExpression(b) => {
                self.lower_expr(&b.left);
                self.lower_expr(&b.right);
            }
            Expression::PrivateInExpression(b) => {
                self.lower_expr(&b.right);
            }

            Expression::LogicalExpression(b) => {
                self.lower_expr(&b.left);
                let after = self.cur;
                let rhs = self.new_block();
                let cont = self.new_block();
                self.add_edge(after, rhs);
                self.add_edge(after, cont);
                self.cur = rhs;
                self.lower_expr(&b.right);
                let rhs_end = self.cur;
                self.add_edge(rhs_end, cont);
                self.cur = cont;
            }

            Expression::ConditionalExpression(c) => {
                self.lower_expr(&c.test);
                let after = self.cur;
                let cons_b = self.new_block();
                let alt_b = self.new_block();
                let cont = self.new_block();
                self.add_edge(after, cons_b);
                self.add_edge(after, alt_b);
                self.cur = cons_b;
                self.lower_expr(&c.consequent);
                let ce = self.cur;
                self.add_edge(ce, cont);
                self.cur = alt_b;
                self.lower_expr(&c.alternate);
                let ae = self.cur;
                self.add_edge(ae, cont);
                self.cur = cont;
            }

            Expression::AssignmentExpression(a) => self.lower_assignment(a),

            Expression::SequenceExpression(s) => {
                for ex in &s.expressions {
                    self.lower_expr(ex);
                }
            }

            Expression::UnaryExpression(u) => self.lower_expr(&u.argument),
            Expression::UpdateExpression(u) => {
                // `++x` / `x--`: a read+write of its target. If the target is a
                // candidate it was excluded (non-simple write); we only need to
                // recurse a member target's parts for nested uses.
                if let SimpleAssignmentTarget::ComputedMemberExpression(m) = &u.argument {
                    self.lower_expr(&m.object);
                    self.lower_expr(&m.expression);
                } else if let SimpleAssignmentTarget::StaticMemberExpression(m) = &u.argument {
                    self.lower_expr(&m.object);
                }
            }

            Expression::TemplateLiteral(t) => {
                for ex in &t.expressions {
                    self.lower_expr(ex);
                }
            }

            Expression::ParenthesizedExpression(p) => self.lower_expr(&p.expression),

            // Nested scopes: no control-flow effect here, captured candidates
            // already excluded — do not recurse.
            Expression::FunctionExpression(_)
            | Expression::ArrowFunctionExpression(_)
            | Expression::ClassExpression(_) => {}

            // Subtle eval order / control flow we don't model exactly: bail.
            _ => self.bailed = true,
        }
    }

    fn lower_argument(&mut self, arg: &Argument<'a>) {
        match arg {
            Argument::SpreadElement(s) => self.lower_expr(&s.argument),
            other => {
                if let Some(e) = other.as_expression() {
                    self.lower_expr(e);
                }
            }
        }
    }

    fn lower_property_key(&mut self, key: &PropertyKey<'a>) {
        if let Some(e) = key.as_expression() {
            self.lower_expr(e);
        }
    }

    fn lower_assignment(&mut self, a: &AssignmentExpression<'a>) {
        use oxc_syntax::operator::AssignmentOperator;
        match &a.left {
            AssignmentTarget::AssignmentTargetIdentifier(id) => {
                let sym = self.ref_sym(id);
                let is_cand = sym.is_some_and(|s| self.cands.contains(&s));
                if is_cand && a.operator == AssignmentOperator::Assign {
                    // `s = rhs`: rhs evaluates, then the def takes effect.
                    self.lower_expr(&a.right);
                    let val = self.classify_def(&a.right);
                    self.emit(Event::Def { sym: sym.unwrap(), val });
                } else {
                    // Non-candidate target, or compound assignment to a candidate
                    // (the latter is excluded by escape analysis, so it can't be a
                    // surviving candidate). Just evaluate the rhs for nested uses.
                    self.lower_expr(&a.right);
                }
            }
            AssignmentTarget::ComputedMemberExpression(m) => {
                self.lower_expr(&m.object);
                self.lower_expr(&m.expression);
                self.lower_expr(&a.right);
            }
            AssignmentTarget::StaticMemberExpression(m) => {
                self.lower_expr(&m.object);
                self.lower_expr(&a.right);
            }
            // Destructuring assignment targets: bail (sound).
            _ => self.bailed = true,
        }
    }

    // --- value classification --------------------------------------------

    fn classify_def(&self, rhs: &Expression<'a>) -> DefVal<'a> {
        if is_inlineable_literal(rhs) {
            return DefVal::Scalar(rhs.clone_in(self.allocator));
        }
        match rhs {
            Expression::ArrayExpression(arr) => {
                if arr.elements.iter().any(|e| matches!(e, ArrayExpressionElement::SpreadElement(_))) {
                    return DefVal::Opaque;
                }
                let mut v = Vec::with_capacity(arr.elements.len());
                for el in &arr.elements {
                    match el {
                        ArrayExpressionElement::Elision(_) => v.push(None),
                        other => {
                            let slot = other
                                .as_expression()
                                .filter(|e| is_inlineable_literal(e))
                                .map(|e| e.clone_in(self.allocator));
                            v.push(slot);
                        }
                    }
                }
                DefVal::Array(v)
            }
            Expression::ObjectExpression(obj) => {
                let mut map = HashMap::new();
                for prop in &obj.properties {
                    let ObjectPropertyKind::ObjectProperty(p) = prop else {
                        return DefVal::Opaque; // spread → unknown keys
                    };
                    if p.computed || p.kind != PropertyKind::Init {
                        return DefVal::Opaque; // computed key / accessor → unknown
                    }
                    let name = match &p.key {
                        PropertyKey::StaticIdentifier(id) => id.name.to_string(),
                        PropertyKey::StringLiteral(s) => s.value.to_string(),
                        PropertyKey::NumericLiteral(n) => n.value.to_string(),
                        _ => return DefVal::Opaque,
                    };
                    if is_inlineable_literal(&p.value) {
                        map.insert(name, p.value.clone_in(self.allocator));
                    } else {
                        // Non-literal value: this name is not foldable, but the
                        // object's key set is still known. Last-writer-wins, so a
                        // later non-literal write must un-record an earlier one.
                        map.remove(&name);
                    }
                }
                DefVal::Object(map)
            }
            _ => DefVal::Opaque,
        }
    }
}

/// Resolve a constant member key from an index expression (`s[<key>]`).
fn static_key(expr: &Expression<'_>) -> Option<Key> {
    match expr {
        Expression::NumericLiteral(n) => int_index(n.value).map(Key::Index),
        Expression::StringLiteral(s) => Some(Key::Name(s.value.to_string())),
        _ => None,
    }
}

fn int_index(n: f64) -> Option<usize> {
    if n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n <= usize::MAX as f64 {
        Some(n as usize)
    } else {
        None
    }
}
