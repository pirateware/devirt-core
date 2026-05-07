//! ES5 desugar for `class` declarations and expressions (issue #31).
//!
//! Tree-IR has no first-class class node. Instead, at AST→tree-IR
//! lowering time, every `class Foo { ... }` is rewritten into the
//! canonical ES5 prototype-chain shape Babel/esbuild already emit:
//!
//! ```js
//! class Foo extends Bar {
//!     constructor(x) { super(x); this.x = x; }
//!     greet() { return this.x; }
//!     static create() { return new Foo(0); }
//! }
//! ```
//!
//! desugars to:
//!
//! ```js
//! function Foo(x) { Bar.call(this, x); this.x = x; }
//! Foo.prototype = Object.create(Bar.prototype);
//! Foo.prototype.constructor = Foo;
//! Foo.prototype.greet = function () { return this.x; };
//! Foo.create = function () { return new Foo(0); };
//! ```
//!
//! The desugar produces only nodes the existing tree-IR lowering
//! already handles (function decls, member assignments, calls), so the
//! enclosing function — previously routed to `PassThroughFunction`
//! because `BlockerKind::ClassDecl` fired — now flows through SSA.
//!
//! ## What desugars vs. what stays a blocker
//!
//! Desugars cleanly:
//! - basic class with constructor + methods
//! - `extends` with a simple superclass identifier or expression
//!   (the superclass expression is captured into a fresh `const` so
//!   `Object.create(<expr>.prototype)` evaluates it exactly once)
//! - implicit constructors (with and without `extends`)
//! - static methods / instance + static fields
//! - `super(args)` in the constructor
//! - `super.method(args)` in instance methods (rewritten to
//!   `<super>.prototype.method.call(this, args)`)
//! - `super.prop` reads (rewritten to `<super>.prototype.prop`)
//! - getters and setters (lowered via `Object.defineProperty`)
//! - computed method/property names
//! - class expressions (`var x = class Foo {}`) — desugar to an IIFE
//!   that returns the constructor
//!
//! Stays a blocker (PassThrough):
//! - `#name` private fields/methods (`BlockerKind::ClassPrivateMember`)
//! - `static { ... }` static blocks (`BlockerKind::ClassStaticBlock`)
//! - decorators on class or members (`BlockerKind::ClassDecorator`)
//! - any inner sub-construct that's itself blocked (those surface
//!   normally as the nested function's blockers, since the desugar
//!   produces inner FuncLiterals that go through the regular per-
//!   function disposition decision)
//!
//! Strict mode: class bodies are always strict per spec, but our
//! desugar doesn't introduce any new strict-mode surface — `this`
//! inside a method body still binds to whatever the call site
//! produces, and the desugared `function` form preserves the same
//! binding because we always invoke methods through `obj.method(...)`
//! (which sets `this = obj`) or through `super.m.call(this, ...)`.

use std::collections::BTreeSet;

use ir::blocker::BlockerKind;
use ir::error::Error;
use ir::tree::*;
use oxc_ast::ast as js;

use crate::Lowerer;

/// Innermost `super` rewrite context. Pushed onto `Lowerer::super_ctx`
/// while lowering a class method body so that `lower_expr`'s
/// `Expression::Super` paths know what to substitute.
#[derive(Clone, Debug)]
pub struct SuperCtx {
    /// The expression to substitute for the prototype owner: usually
    /// a fresh `Expr::Ident` referring to the captured superclass
    /// binding (`__super0__`). Cloned at each rewrite site.
    pub super_ref: Expr,
    /// True when the current method is `static`. Static methods rewrite
    /// `super.x` to `<super>.x` (the superclass *constructor function*
    /// owns static members), while instance methods rewrite to
    /// `<super>.prototype.x`.
    pub is_static: bool,
}

/// Walk a class body for shapes the desugar cannot represent. Returns
/// the (possibly empty) blocker set. Empty means "desugarable".
///
/// This is consulted at disposition time: when a function body
/// contains a `class` whose desugar would fail, the enclosing function
/// stays PassThrough exactly as before — no silent regression.
pub fn class_blockers(class: &js::Class) -> BTreeSet<BlockerKind> {
    let mut blockers = BTreeSet::new();
    if !class.decorators.is_empty() {
        blockers.insert(BlockerKind::ClassDecorator);
    }
    for el in &class.body.body {
        match el {
            js::ClassElement::StaticBlock(_) => {
                blockers.insert(BlockerKind::ClassStaticBlock);
            }
            js::ClassElement::MethodDefinition(m) => {
                if !m.decorators.is_empty() {
                    blockers.insert(BlockerKind::ClassDecorator);
                }
                if matches!(&m.key, js::PropertyKey::PrivateIdentifier(_)) {
                    blockers.insert(BlockerKind::ClassPrivateMember);
                }
            }
            js::ClassElement::PropertyDefinition(p) => {
                if !p.decorators.is_empty() {
                    blockers.insert(BlockerKind::ClassDecorator);
                }
                if matches!(&p.key, js::PropertyKey::PrivateIdentifier(_)) {
                    blockers.insert(BlockerKind::ClassPrivateMember);
                }
            }
            js::ClassElement::AccessorProperty(_) => {
                // `accessor x = 1;` — stage-3, not desugared yet.
                blockers.insert(BlockerKind::ClassDecorator);
            }
            js::ClassElement::TSIndexSignature(_) => {
                // TypeScript-only, ignored. Body is type-erased anyway
                // by the time we get here in JS-source mode, but be
                // defensive.
            }
        }
    }
    blockers
}

impl<'src> Lowerer<'src> {
    /// Desugar `class Foo extends Bar { ... }` into a sequence of
    /// statements: a `function Foo(...) { ... }` declaration plus the
    /// prototype-chain hookup and per-member assignments.
    ///
    /// `class_name` is the binding name to use for the constructor
    /// function. For `class Foo {}`, it's `Foo`. For an anonymous class
    /// expression, the caller picks a fresh temp.
    ///
    /// `inner_decl` controls whether the constructor function appears
    /// as a `Stmt::FuncDecl` (when the class is inside a function body)
    /// or whether the caller should hoist it differently. We always
    /// emit `Stmt::FuncDecl`; top-level handling re-wraps as needed.
    pub(crate) fn desugar_class(
        &self,
        class: &js::Class,
        class_name: &str,
    ) -> Result<Vec<Stmt>, Error> {
        // Refuse early on shapes we can't represent. The disposition
        // walker also checks this so the enclosing function stays
        // PassThrough; the lowering refuse here is the safety net.
        let blockers = class_blockers(class);
        if !blockers.is_empty() {
            return Err(Error::Unsupported(
                "class with private/static-block/decorator member",
            ));
        }

        let mut out: Vec<Stmt> = Vec::new();

        // === 1. Capture superclass into a fresh const, if any. ===
        //
        // We bind the superclass expression once so the prototype
        // hookup AND every `super.x` reference inside method bodies
        // share a single evaluation. Using a fresh `__super<n>__`
        // temp avoids colliding with any name in scope.
        let (super_binding, super_capture_stmt) = match &class.super_class {
            Some(sc) => {
                let binding = format!("__super{}__", self.fresh_super_id());
                let init = self.lower_expr(sc)?;
                let stmt = Stmt::Const {
                    name: binding.clone(),
                    init,
                };
                (Some(binding), Some(stmt))
            }
            None => (None, None),
        };
        if let Some(s) = super_capture_stmt {
            out.push(s);
        }

        // === 2. Collect parts of the class body. ===
        //
        // The desugar order matters for observable behavior:
        //   constructor (defined as the function) → field initializers
        //   prepended to the constructor body → prototype hookup
        //   (extends) → constructor.constructor = self → method
        //   assignments in source order.
        let mut user_ctor: Option<&js::MethodDefinition> = None;
        let mut instance_fields: Vec<&js::PropertyDefinition> = Vec::new();
        let mut static_fields: Vec<&js::PropertyDefinition> = Vec::new();
        let mut methods: Vec<&js::MethodDefinition> = Vec::new();
        for el in &class.body.body {
            match el {
                js::ClassElement::MethodDefinition(m)
                    if matches!(m.kind, js::MethodDefinitionKind::Constructor) =>
                {
                    user_ctor = Some(m);
                }
                js::ClassElement::MethodDefinition(m) => methods.push(m),
                js::ClassElement::PropertyDefinition(p) => {
                    if p.r#static {
                        static_fields.push(p);
                    } else {
                        instance_fields.push(p);
                    }
                }
                _ => {
                    // class_blockers above rejected static blocks /
                    // accessor / TS-index. Reaching here is a bug.
                    return Err(Error::Unsupported("class element kind"));
                }
            }
        }

        // === 3. Build the constructor function. ===
        let ctor_lit = self.build_constructor_function(
            class_name,
            user_ctor,
            &instance_fields,
            super_binding.as_deref(),
        )?;
        out.push(Stmt::FuncDecl(ctor_lit));

        // === 4. Prototype-chain hookup if `extends`. ===
        if let Some(super_name) = &super_binding {
            // Foo.prototype = Object.create(__super__.prototype);
            out.push(Stmt::Assign {
                target: prototype_of(class_name),
                value: Expr::Call {
                    callee: Box::new(Expr::Member {
                        obj: Box::new(Expr::Ident("Object".into())),
                        prop: "create".into(),
                    }),
                    args: vec![prototype_of(super_name)],
                },
            });
            // Foo.prototype.constructor = Foo;
            out.push(Stmt::Assign {
                target: Expr::Member {
                    obj: Box::new(prototype_of(class_name)),
                    prop: "constructor".into(),
                },
                value: Expr::Ident(class_name.into()),
            });
        }

        // === 5. Method assignments. ===
        for m in &methods {
            self.emit_method_assign(class_name, m, super_binding.as_deref(), &mut out)?;
        }

        // === 6. Static field initializers. ===
        for p in &static_fields {
            // Push a SuperCtx for static fields too — though `super`
            // in a static field initializer refers to the super class
            // constructor, not its prototype. That's exactly the
            // is_static=true rewrite.
            let initializer = match &p.value {
                Some(v) => self.with_super(super_binding.as_deref(), true, |this| this.lower_expr(v))?,
                None => Expr::Undefined,
            };
            let target = self.member_target(class_name, &p.key, p.computed)?;
            out.push(Stmt::Assign {
                target,
                value: initializer,
            });
        }

        Ok(out)
    }

    /// Counter for synthesizing fresh `__super<n>__` binding names.
    /// Reuses `temp_counter` so collisions with `__t<n>__` are also
    /// impossible.
    fn fresh_super_id(&self) -> usize {
        let n = self.temp_counter.get();
        self.temp_counter.set(n + 1);
        n
    }

    /// Build the constructor `FuncLiteral`. Handles:
    ///  - explicit user constructor (its body becomes the function
    ///    body, with field initializers prepended right after any
    ///    leading `super(...)` call so the prototype-chain invariant
    ///    matches spec — a `super(...)` MUST run before `this` is
    ///    accessed).
    ///  - implicit constructor without `extends`: empty body.
    ///  - implicit constructor with `extends`: forwards all args via
    ///    `__super.apply(this, arguments)` (rest-args desugar).
    fn build_constructor_function(
        &self,
        class_name: &str,
        user_ctor: Option<&js::MethodDefinition>,
        instance_fields: &[&js::PropertyDefinition],
        super_binding: Option<&str>,
    ) -> Result<FuncLiteral, Error> {
        // === Field initializer prologue ===
        // Each `x = expr` becomes `this.x = expr;` evaluated at
        // construction time, BEFORE the user constructor body runs —
        // except when `extends` is present, in which case fields run
        // AFTER `super(...)` (to mirror spec semantics: `this` is not
        // bound until the super-call completes).
        let mut field_inits: Vec<Stmt> = Vec::with_capacity(instance_fields.len());
        for p in instance_fields {
            let init_expr = match &p.value {
                Some(v) => self.with_super(super_binding, false, |this| this.lower_expr(v))?,
                None => Expr::Undefined,
            };
            let target = self.this_member_target(&p.key, p.computed)?;
            field_inits.push(Stmt::Assign {
                target,
                value: init_expr,
            });
        }

        match user_ctor {
            Some(ctor) => {
                let func = &*ctor.value;
                // Lower params and body inside a super context so
                // `super(args)` and `super.x` rewrites apply.
                let (params, rest_param, mut prologue) = self.lower_param_list(&func.params)?;
                let body = func.body.as_ref().ok_or(Error::Unsupported(
                    "class constructor without a body",
                ))?;
                let lowered_body = self.with_super(super_binding, false, |this| {
                    this.lower_stmts_list(&body.statements)
                })?;
                // Splice field initializers immediately after the
                // first super() call (if any) so `this` is valid.
                let merged = splice_field_initializers(lowered_body, field_inits);
                prologue.extend(merged);
                Ok(FuncLiteral {
                    name: Some(class_name.into()),
                    params,
                    rest_param,
                    is_async: false,
                    is_generator: false,
                    is_arrow: false,
                    body: FuncBody::Stmts(prologue),
                    func_id: std::cell::Cell::new(None),
                    passthrough_source: None,
                    force_passthrough: std::cell::Cell::new(false),
                    blockers: BTreeSet::new(),
                })
            }
            None => {
                // Implicit constructor.
                let mut body: Vec<Stmt> = Vec::new();
                if let Some(super_name) = super_binding {
                    // function Foo(...args) { return Bar.apply(this, arguments); }
                    // We can't model rest-args, so use the dynamic
                    // `arguments` binding — it's available in ordinary
                    // functions and forwards every actual arg.
                    body.push(Stmt::ExprStmt(Expr::Call {
                        callee: Box::new(Expr::Member {
                            obj: Box::new(Expr::Ident(super_name.into())),
                            prop: "apply".into(),
                        }),
                        args: vec![Expr::This, Expr::Ident("arguments".into())],
                    }));
                }
                body.extend(field_inits);
                Ok(FuncLiteral {
                    name: Some(class_name.into()),
                    params: Vec::new(),
                    rest_param: None,
                    is_async: false,
                    is_generator: false,
                    is_arrow: false,
                    body: FuncBody::Stmts(body),
                    func_id: std::cell::Cell::new(None),
                    passthrough_source: None,
                    force_passthrough: std::cell::Cell::new(false),
                    blockers: BTreeSet::new(),
                })
            }
        }
    }

    /// Run `f` with `super_ctx` pushed onto the stack and pop it
    /// afterward. Always pops — even on error.
    fn with_super<F, R>(&self, super_binding: Option<&str>, is_static: bool, f: F) -> R
    where
        F: FnOnce(&Self) -> R,
    {
        if let Some(name) = super_binding {
            self.super_ctx.borrow_mut().push(SuperCtx {
                super_ref: Expr::Ident(name.into()),
                is_static,
            });
            let r = f(self);
            self.super_ctx.borrow_mut().pop();
            r
        } else {
            // No `extends`: `super` is invalid inside the method body
            // anyway. Lower without a super context; if `super` shows
            // up, lower_expr will reject it.
            f(self)
        }
    }

    /// Emit one method assignment statement.
    ///
    ///   instance method: `Foo.prototype.<key> = function(...) { ... };`
    ///   static   method: `Foo.<key>           = function(...) { ... };`
    ///   getter / setter: `Object.defineProperty(<owner>, <key>, { get/set });`
    fn emit_method_assign(
        &self,
        class_name: &str,
        m: &js::MethodDefinition,
        super_binding: Option<&str>,
        out: &mut Vec<Stmt>,
    ) -> Result<(), Error> {
        let owner = if m.r#static {
            Expr::Ident(class_name.into())
        } else {
            prototype_of(class_name)
        };

        // Lower the method body with a SuperCtx if the class extends.
        let func = &*m.value;
        let func_lit = self.with_super(super_binding, m.r#static, |this| {
            this.lower_method_function(func)
        })?;
        let func_expr = Expr::Func(Box::new(func_lit));

        match m.kind {
            js::MethodDefinitionKind::Method => {
                let target = self.compose_member(owner, &m.key, m.computed)?;
                out.push(Stmt::Assign {
                    target,
                    value: func_expr,
                });
            }
            js::MethodDefinitionKind::Get | js::MethodDefinitionKind::Set => {
                let key_expr = self.key_to_expr(&m.key, m.computed)?;
                let kind = if matches!(m.kind, js::MethodDefinitionKind::Get) {
                    "get"
                } else {
                    "set"
                };
                // Object.defineProperty(<owner>, <key>, { get|set: function(){...}, configurable: true })
                let descriptor = Expr::Object {
                    props: vec![
                        ObjectProp {
                            key: ObjectKey::Static(kind.into()),
                            value: func_expr,
                        },
                        ObjectProp {
                            key: ObjectKey::Static("configurable".into()),
                            value: Expr::Bool(true),
                        },
                    ],
                };
                out.push(Stmt::ExprStmt(Expr::Call {
                    callee: Box::new(Expr::Member {
                        obj: Box::new(Expr::Ident("Object".into())),
                        prop: "defineProperty".into(),
                    }),
                    args: vec![owner, key_expr, descriptor],
                }));
            }
            js::MethodDefinitionKind::Constructor => {
                // Caller already extracted the constructor.
                unreachable!("constructor should be filtered out before emit_method_assign");
            }
        }
        Ok(())
    }

    /// Lower a class-method `js::Function` into a `FuncLiteral`,
    /// reusing the same param/body machinery as ordinary function
    /// expressions. Methods in a class body cannot have decorators
    /// (rejected at `class_blockers` time); they are never `async`/
    /// `generator` in our supported subset (those would surface as
    /// blockers from the inner-function disposition decision).
    fn lower_method_function(&self, f: &js::Function) -> Result<FuncLiteral, Error> {
        // Reuse `lower_function_literal` (which also runs disposition
        // and falls back to PassThrough if the method body itself is
        // unlowerable — exactly the closure behavior we want). The
        // method has no `id`, but `lower_function_literal` only
        // requires one for declarations, not expressions (`is_decl =
        // false`).
        self.lower_function_literal(f, false)
    }

    /// Compose `owner.<key>` for a static-or-static-key method. For
    /// computed keys, returns `owner[<key_expr>]`.
    fn compose_member(
        &self,
        owner: Expr,
        key: &js::PropertyKey,
        computed: bool,
    ) -> Result<Expr, Error> {
        if computed {
            let e = key.as_expression().ok_or(Error::Unsupported(
                "non-expression computed class key",
            ))?;
            Ok(Expr::Index {
                obj: Box::new(owner),
                index: Box::new(self.lower_expr(e)?),
            })
        } else {
            match key {
                js::PropertyKey::StaticIdentifier(id) => Ok(Expr::Member {
                    obj: Box::new(owner),
                    prop: id.name.to_string(),
                }),
                js::PropertyKey::StringLiteral(s) => Ok(Expr::Member {
                    obj: Box::new(owner),
                    prop: s.value.to_string(),
                }),
                js::PropertyKey::NumericLiteral(n) => Ok(Expr::Index {
                    obj: Box::new(owner),
                    index: Box::new(Expr::Number(n.value)),
                }),
                js::PropertyKey::PrivateIdentifier(_) => {
                    // Rejected at class_blockers time; safety net here.
                    Err(Error::Unsupported("private class member"))
                }
                _ => Err(Error::Unsupported("class key kind")),
            }
        }
    }

    /// Build `<class_name>.<key>` or `<class_name>.prototype.<key>`
    /// for a static-field assignment site (or the getter/setter
    /// owner). Convenience wrapper around `compose_member`.
    fn member_target(
        &self,
        class_name: &str,
        key: &js::PropertyKey,
        computed: bool,
    ) -> Result<Expr, Error> {
        self.compose_member(Expr::Ident(class_name.into()), key, computed)
    }

    /// Build `this.<key>` for a field initializer in the constructor.
    fn this_member_target(
        &self,
        key: &js::PropertyKey,
        computed: bool,
    ) -> Result<Expr, Error> {
        self.compose_member(Expr::This, key, computed)
    }

    /// Lower a class key into the expression form expected by
    /// `Object.defineProperty`'s second argument: a string for static
    /// keys, the underlying expression for computed keys.
    fn key_to_expr(&self, key: &js::PropertyKey, computed: bool) -> Result<Expr, Error> {
        if computed {
            let e = key.as_expression().ok_or(Error::Unsupported(
                "non-expression computed class key",
            ))?;
            return self.lower_expr(e);
        }
        match key {
            js::PropertyKey::StaticIdentifier(id) => Ok(Expr::String(id.name.to_string())),
            js::PropertyKey::StringLiteral(s) => Ok(Expr::String(s.value.to_string())),
            js::PropertyKey::NumericLiteral(n) => Ok(Expr::Number(n.value)),
            js::PropertyKey::PrivateIdentifier(_) => Err(Error::Unsupported("private class member")),
            _ => Err(Error::Unsupported("class key kind")),
        }
    }
}

/// Build `<name>.prototype` as a new `Expr::Member`.
fn prototype_of(name: &str) -> Expr {
    Expr::Member {
        obj: Box::new(Expr::Ident(name.into())),
        prop: "prototype".into(),
    }
}

/// Insert `field_inits` into a constructor body just after the first
/// top-level `super(...)` call (if present), or at the very top
/// otherwise. The order matches the spec: in a derived class, `this`
/// only becomes valid after the super call.
fn splice_field_initializers(body: Vec<Stmt>, field_inits: Vec<Stmt>) -> Vec<Stmt> {
    if field_inits.is_empty() {
        return body;
    }
    // Find the first top-level Stmt::ExprStmt(Expr::Call) where the
    // callee was originally `super(...)`. After lowering, the rewrite
    // performed by `lower_expr` has already turned `super(...)` into
    // `<super>.call(this, ...)` / `<super>.apply(...)`, so we look
    // for that exact shape. We tag synthesized super-calls in the
    // `Stmt::ExprStmt` form below; spliciing happens here.
    //
    // Simpler approach: pattern-match the rewritten shape. The
    // rewrite always produces an `Expr::Call` whose callee is
    // `Expr::Member { obj: Expr::Ident(<super_binding>), prop: "call" }`
    // with first arg `Expr::This` — but matching that name requires
    // the binding to be in scope. As an over-approximation, we splice
    // after the first `Stmt::ExprStmt` whose call shape looks like
    // `<ident>.call(this, ...)` OR `<ident>.apply(this, ...)` — both
    // are super-call forms our lowering produces.
    // First check if the body starts with a super-call shape; if so,
    // splice fields right after it. Otherwise prepend.
    let starts_with_super_call = body
        .first()
        .map(stmt_is_super_call_shape)
        .unwrap_or(false);
    let mut out: Vec<Stmt> = Vec::with_capacity(body.len() + field_inits.len());
    if starts_with_super_call {
        let mut iter = body.into_iter();
        out.push(iter.next().unwrap());
        out.extend(field_inits);
        out.extend(iter);
    } else {
        out.extend(field_inits);
        out.extend(body);
    }
    out
}

/// True when `s` is `<id>.call(this, ...)` or `<id>.apply(this, ...)`
/// at top level — the shape `lower_expr` produces for `super(...)`.
fn stmt_is_super_call_shape(s: &Stmt) -> bool {
    let Stmt::ExprStmt(Expr::Call { callee, args }) = s else {
        return false;
    };
    if args.is_empty() {
        return false;
    }
    // First arg must be `this` for it to be a forwarded super call.
    if !matches!(&args[0], Expr::This) {
        return false;
    }
    let Expr::Member { obj, prop } = &**callee else {
        return false;
    };
    if !matches!(prop.as_str(), "call" | "apply") {
        return false;
    }
    // Be permissive about the receiver — any identifier is fine.
    matches!(&**obj, Expr::Ident(_))
}
