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

/// One-time prelude evaluated when the context is created. QuickJS ships only
/// ECMAScript built-ins, but a common decoder string codec is base64 via the
/// WHATWG `atob`/`btoa` (e.g. DataDome's `function(i){ return atob(table[i]) }`).
/// Without these a base64 decoder throws `ReferenceError` and is left un-lifted.
/// We define spec-correct polyfills *only if absent* (so a future QuickJS that
/// ships them wins). `atob` uses a 6-bit accumulator so it decodes unpadded
/// input correctly; both return/consume Latin-1 "binary strings" like the host.
const SANDBOX_PRELUDE: &str = r#"
(function(g){
  var B="ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  if (typeof g.atob === "undefined") {
    g.atob = function(input){
      var s=String(input).replace(/[ \t\n\f\r]/g,"");
      if (s.length % 4 === 0) s=s.replace(/==?$/,"");
      if (s.length % 4 === 1 || /[^+/0-9A-Za-z]/.test(s)) return null;
      var out="",buf=0,bits=0;
      for (var i=0;i<s.length;i++){
        buf=(buf<<6)|B.indexOf(s.charAt(i)); bits+=6;
        if (bits===24){
          out+=String.fromCharCode((buf>>16)&255,(buf>>8)&255,buf&255);
          buf=0; bits=0;
        }
      }
      if (bits===12){ out+=String.fromCharCode((buf>>4)&255); }
      else if (bits===18){ out+=String.fromCharCode((buf>>10)&255,(buf>>2)&255); }
      return out;
    };
  }
  if (typeof g.btoa === "undefined") {
    g.btoa = function(data){
      var s=String(data),out="";
      for (var i=0;i<s.length;i+=3){
        var c0=s.charCodeAt(i),c1=s.charCodeAt(i+1),c2=s.charCodeAt(i+2);
        if (c0>255||c1>255||c2>255) throw new Error("btoa: out of range");
        out+=B.charAt(c0>>2)+B.charAt(((c0&3)<<4)|((c1>>4)||0));
        out+=isNaN(c1)?"=":B.charAt(((c1&15)<<2)|((c2>>6)||0));
        out+=isNaN(c2)?"=":B.charAt(c2&63);
      }
      return out;
    };
  }
})(typeof globalThis!=="undefined"?globalThis:this);
"#;

/// Shared deadline the interrupt handler checks. `Mutex<Option<Instant>>`:
/// `None` means "no eval in flight" (never interrupt).
type Deadline = std::sync::Arc<Mutex<Option<Instant>>>;

pub struct Sandbox {
    ctx: Context,
    deadline: Deadline,
    _rt: Runtime,
}

/// A decoder result, tagged by JS type so the lifter can splice the *correct*
/// literal kind. Coercing a numeric result to a string literal (the old
/// `eval_to_string`-only path) silently changed semantics: `dec(x) + 1` would go
/// from numeric `262` to string `"2611"`.
///
/// `Arr` carries a flat array of primitives (no nesting, no holes) so an
/// array-returning decoder can be materialized as an array literal. It is
/// deliberately shallow: the splice path caps the element count so a huge bytecode
/// table is never expanded (net-negative — see `issues/09`).
#[derive(Debug, Clone, PartialEq)]
pub enum EvalValue {
    Str(String),
    Num(f64),
    Bool(bool),
    Arr(Vec<EvalValue>),
}

impl Sandbox {
    pub fn new() -> Option<Sandbox> {
        let rt = Runtime::new().ok()?;
        rt.set_memory_limit(128 * 1024 * 1024);
        rt.set_max_stack_size(2 * 1024 * 1024);

        let deadline: Deadline = std::sync::Arc::new(Mutex::new(None));
        let handler_deadline = deadline.clone();
        rt.set_interrupt_handler(Some(Box::new(
            move || matches!(*handler_deadline.lock().unwrap(), Some(d) if Instant::now() >= d),
        )));

        let ctx = Context::full(&rt).ok()?;
        // Install the base64 polyfills once in the persistent context.
        if !ctx.with(|ctx| ctx.eval::<rquickjs::Value, _>(SANDBOX_PRELUDE).is_ok()) {
            return None;
        }
        Some(Sandbox {
            ctx,
            deadline,
            _rt: rt,
        })
    }

    /// Run `code` once for its side effects (define the decoder library /
    /// rotate the array) in the persistent context. Returns whether it ran
    /// without throwing. Subsequent `eval_to_string` calls see the definitions.
    pub fn setup(&self, code: &str) -> bool {
        *self.deadline.lock().unwrap() = Some(Instant::now() + EVAL_BUDGET);
        let ok = self
            .ctx
            .with(|ctx| ctx.eval::<rquickjs::Value, _>(code).is_ok());
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
            let mut budget = MAX_ARRAY_ELEMS;
            value_to_eval(&v, &mut budget)
        });
        *self.deadline.lock().unwrap() = None;
        result
    }
}

/// Hard cap on the number of array elements (across all nesting) any single
/// `eval_to_value` result may materialize. This keeps the array-splice from ever
/// expanding a large packed table (e.g. a VM bytecode array) into a giant literal,
/// which would bloat output for no readability win (`issues/09`).
const MAX_ARRAY_ELEMS: usize = 64;

/// Convert a QuickJS value to an [`EvalValue`], counting array elements against
/// `budget`. Returns `None` for any non-primitive/non-flat-array value, a hole, or
/// when the element budget is exhausted — those are not safe/desirable to splice.
fn value_to_eval(v: &rquickjs::Value<'_>, budget: &mut usize) -> Option<EvalValue> {
    if v.is_string() {
        return v.get::<String>().ok().map(EvalValue::Str);
    }
    if v.is_number() {
        return v.as_number().map(EvalValue::Num);
    }
    if v.is_bool() {
        return v.as_bool().map(EvalValue::Bool);
    }
    if let Some(arr) = v.as_array() {
        let len = arr.len();
        if len > *budget {
            return None; // would exceed the materialization cap
        }
        *budget -= len;
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            // A hole or a non-convertible element disqualifies the whole array:
            // splicing it would change semantics or require a non-literal form.
            let elem: rquickjs::Value = arr.get(i).ok()?;
            out.push(value_to_eval(&elem, budget)?);
        }
        return Some(EvalValue::Arr(out));
    }
    None
}
