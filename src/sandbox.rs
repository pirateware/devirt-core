//! Sandboxed partial evaluation.
//!
//! QuickJS exposes only ECMAScript built-ins — no filesystem, network, or
//! `require` — so an extracted decoder can compute strings but cannot touch the
//! host. We additionally bound memory and impose a per-evaluation wall-clock
//! deadline (enforced from the VM interrupt handler) so a malicious or
//! non-terminating "decoder" can't hang or OOM the deobfuscator. The deadline
//! is reset before every `eval`, so the budget is per-call, never cumulative.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use rquickjs::{Context, Runtime};

/// Max wall-clock time any single decoder evaluation may run.
const EVAL_BUDGET: Duration = Duration::from_secs(2);

/// Shared deadline the interrupt handler checks. `Mutex<Option<Instant>>`:
/// `None` means "no eval in flight" (never interrupt).
type Deadline = std::sync::Arc<Mutex<Option<Instant>>>;

pub struct Sandbox {
    ctx: Context,
    deadline: Deadline,
    _rt: Runtime,
}

/// A primitive decoder result, tagged by JS type so the lifter can splice the
/// *correct* literal kind. Coercing a numeric result to a string literal (the
/// old `eval_to_string`-only path) silently changed semantics: `dec(x) + 1`
/// would go from numeric `262` to string `"2611"`.
#[derive(Debug, Clone, PartialEq)]
pub enum EvalValue {
    Str(String),
    Num(f64),
    Bool(bool),
}

impl Sandbox {
    pub fn new() -> Option<Sandbox> {
        let rt = Runtime::new().ok()?;
        rt.set_memory_limit(128 * 1024 * 1024);
        rt.set_max_stack_size(2 * 1024 * 1024);

        let deadline: Deadline = std::sync::Arc::new(Mutex::new(None));
        let handler_deadline = deadline.clone();
        rt.set_interrupt_handler(Some(Box::new(move || {
            matches!(*handler_deadline.lock().unwrap(), Some(d) if Instant::now() >= d)
        })));

        let ctx = Context::full(&rt).ok()?;
        Some(Sandbox { ctx, deadline, _rt: rt })
    }

    /// Run `code` once for its side effects (define the decoder library /
    /// rotate the array) in the persistent context. Returns whether it ran
    /// without throwing. Subsequent `eval_to_string` calls see the definitions.
    pub fn setup(&self, code: &str) -> bool {
        *self.deadline.lock().unwrap() = Some(Instant::now() + EVAL_BUDGET);
        let ok = self.ctx.with(|ctx| ctx.eval::<rquickjs::Value, _>(code).is_ok());
        *self.deadline.lock().unwrap() = None;
        ok
    }

    /// Evaluate `code` (ending in an expression) and return it coerced to a
    /// string. Returns `None` on any error, timeout, or non-stringable result.
    pub fn eval_to_string(&self, code: &str) -> Option<String> {
        *self.deadline.lock().unwrap() = Some(Instant::now() + EVAL_BUDGET);
        let result = self.ctx.with(|ctx| {
            let v: rquickjs::Value = ctx.eval(code).ok()?;
            if v.is_string() {
                v.get::<String>().ok()
            } else if v.is_number() {
                v.as_number().map(|n| {
                    if n.fract() == 0.0 {
                        format!("{}", n as i64)
                    } else {
                        format!("{n}")
                    }
                })
            } else {
                None
            }
        });
        *self.deadline.lock().unwrap() = None;
        result
    }

    /// Evaluate `code` (ending in an expression) and return its primitive value
    /// tagged by type. Returns `None` on any error, timeout, or non-primitive
    /// (object/array/function/undefined/null) result — those are not safe to
    /// splice as a single literal.
    pub fn eval_to_value(&self, code: &str) -> Option<EvalValue> {
        *self.deadline.lock().unwrap() = Some(Instant::now() + EVAL_BUDGET);
        let result = self.ctx.with(|ctx| {
            let v: rquickjs::Value = ctx.eval(code).ok()?;
            if v.is_string() {
                v.get::<String>().ok().map(EvalValue::Str)
            } else if v.is_number() {
                v.as_number().map(EvalValue::Num)
            } else if v.is_bool() {
                v.as_bool().map(EvalValue::Bool)
            } else {
                None
            }
        });
        *self.deadline.lock().unwrap() = None;
        result
    }
}
