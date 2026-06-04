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
  // Determinism freeze. Partial evaluation must be reproducible: a decoder we
  // run in the sandbox that happens to read the wall clock would otherwise fold
  // to a different literal on every run (the `report` jitter on s3/s11 was a
  // `Date.now()` seed/timestamp leaking into a lifted value). The in-process
  // double-eval determinism guard MISSES `Date.now()` because two back-to-back
  // millisecond reads agree, yet they differ across processes — so we pin the
  // clock itself to a constant. We freeze only `Date.now()` (the value-returning
  // reader), leaving the `Date` constructor and instance methods intact so
  // feature-detection and `new Date(x)` inside a real decoder keep working.
  //
  // `Math.random` is handled OUTSIDE this prelude: `new_seeded` installs a
  // deterministic, seeded PRNG (see `seeded_random_js`) — a real varied sequence,
  // not a frozen constant (a constant degenerates charset shuffles and
  // mass-declines decoders, ~+10% bloat on sample_11). The lifter runs two
  // sandboxes with different seeds and refuses any value that disagrees, so a
  // genuinely RNG-dependent decode is never baked.
  //
  // Freeze *every* wall-clock reader, not just `Date.now()`. The double-eval guard
  // misses `new Date()` / `+new Date()` / `.getTime()` because two reads in the
  // same millisecond agree (sample_7 leaked a `+new Date()` token timestamp into a
  // lifted value). Pin `Date.now()` and the no-arg `new Date()` (and `Date()` as a
  // function) to the epoch, while leaving `new Date(explicitArg)` exact so a real
  // decoder's date math still works.
  if (typeof g.Date === "function") {
    var _RealDate = g.Date;
    var FrozenDate = function(){
      if (!(this instanceof FrozenDate)) return new _RealDate(0).toString();
      return arguments.length === 0 ? new _RealDate(0) : new _RealDate(...arguments);
    };
    FrozenDate.prototype = _RealDate.prototype;
    FrozenDate.now = function(){ return 0; };
    FrozenDate.parse = _RealDate.parse;
    FrozenDate.UTC = _RealDate.UTC;
    g.Date = FrozenDate;
  }

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

/// Default `Math.random` seed for the partial-evaluation sandbox. QuickJS seeds
/// `Math.random` *non-deterministically per context*, so a decoder that shuffles
/// a charset/seed with it (sample_7) decodes to a different value every process —
/// jitter that leaks into a lifted literal. We install a *deterministic, seeded*
/// PRNG instead. Crucially it is a real (varied) sequence, **not** a frozen
/// constant: a constant `Math.random` degenerates Fisher-Yates shuffles and
/// mass-declines legitimate decoders (~+10% bloat on sample_11), whereas a seeded
/// PRNG shuffles correctly *and* reproducibly. Soundness against decoders whose
/// *result* truly depends on RNG is handled separately by the lifter's two-seed
/// differential (it refuses any value that changes between two seeds).
pub const DEFAULT_RANDOM_SEED: u32 = 0x9E37_79B9;

/// Install a deterministic mulberry32 PRNG as `Math.random`, seeded by `seed`.
/// Deterministic + well-distributed, so charset shuffles work and are reproducible.
fn seeded_random_js(seed: u32) -> String {
    format!(
        r#"(function(g){{
          var s = {seed} >>> 0;
          g.Math.random = function(){{
            s = (s + 0x6D2B79F5) | 0;
            var t = Math.imul(s ^ (s >>> 15), 1 | s);
            t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
            return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
          }};
        }})(typeof globalThis !== "undefined" ? globalThis : this);"#
    )
}

impl Sandbox {
    pub fn new() -> Option<Sandbox> {
        Self::new_seeded(DEFAULT_RANDOM_SEED)
    }

    /// A sandbox whose `Math.random` is a deterministic PRNG seeded by `seed`.
    /// The lifter builds two with *different* seeds and lifts a decode only when
    /// both agree — so a value that genuinely depends on RNG (non-reproducible at
    /// runtime) is refused, while one independent of RNG lifts reproducibly.
    pub fn new_seeded(seed: u32) -> Option<Sandbox> {
        let rt = Runtime::new().ok()?;
        rt.set_memory_limit(128 * 1024 * 1024);
        rt.set_max_stack_size(2 * 1024 * 1024);

        let deadline: Deadline = std::sync::Arc::new(Mutex::new(None));
        let handler_deadline = deadline.clone();
        rt.set_interrupt_handler(Some(Box::new(
            move || matches!(*handler_deadline.lock().unwrap(), Some(d) if Instant::now() >= d),
        )));

        let ctx = Context::full(&rt).ok()?;
        // Install the base64 polyfills + Date freeze, then the seeded PRNG.
        let ok = ctx.with(|ctx| {
            ctx.eval::<rquickjs::Value, _>(SANDBOX_PRELUDE).is_ok()
                && ctx
                    .eval::<rquickjs::Value, _>(seeded_random_js(seed))
                    .is_ok()
        });
        if !ok {
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

// ---------------------------------------------------------------------------
// Whole-program behavior signatures (regression / equivalence harness).
// ---------------------------------------------------------------------------

/// The terminal outcome of running a *whole program* under the equivalence
/// harness. This is the half of a [`BehaviorSignature`] that captures *how* a
/// program ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    /// Ran to completion without throwing.
    Completed,
    /// Threw. Carries the error's constructor name (`"TypeError"`,
    /// `"ReferenceError"`, …). The *message* is deliberately excluded: it embeds
    /// identifiers the deobfuscator renames, so comparing messages would flag a
    /// sound rename as a behavior change. Built-in error names are rename-stable.
    Threw(String),
    /// Hit the wall-clock budget without terminating — e.g. a polling /
    /// animation loop the permissive host shim never lets the program break out
    /// of. Not a comparable outcome; the equivalence harness skips these.
    TimedOut,
}

/// A deterministic, observable behavior signature of a whole program: its
/// terminal [`RunOutcome`] plus the exact sequence of `console` output it
/// produced. Two programs with equal signatures are observably equivalent under
/// the harness — this is the soundness contract a deobfuscation pass must keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BehaviorSignature {
    pub outcome: RunOutcome,
    pub output: Vec<String>,
}

/// Run `source` as a whole program under a deterministic, permissive
/// browser-host shim and capture its [`BehaviorSignature`].
///
/// The shim makes adversarial browser code *runnable* off a real browser so we
/// can observe it end-to-end:
///
/// * **Permissive host.** Every browser global in [`HOST_GLOBALS`] (`window`,
///   `document`, `navigator`, …) is a single deep [`Proxy`] that is callable,
///   constructable, and indexable, and returns itself for any access. So
///   `window.foo.bar()` and `new navigator.x()` resolve instead of throwing —
///   the program runs far enough to exercise the deobfuscated logic.
/// * **Determinism.** `Date.now` and `Math.random` are frozen to constants and
///   timers (`setTimeout`/`requestAnimationFrame`) are inert, so the same
///   program yields the same signature every run — a precondition for comparing
///   original against deobfuscated.
/// * **Isolation + bounds.** A fresh runtime per call (no cross-run state), a
///   128 MiB memory cap, and a wall-clock `budget` enforced from the interrupt
///   handler (a non-terminating program becomes [`RunOutcome::TimedOut`], never
///   a hang).
///
/// This is the engine behind the full-sample equivalence regression test: run a
/// sample and its deobfuscated form, require identical signatures.
pub fn behavior_signature(source: &str, budget: Duration) -> BehaviorSignature {
    behavior_signature_impl(source, budget).unwrap_or(BehaviorSignature {
        // A harness that fails to even construct its runtime is reported as a
        // distinct, comparable outcome rather than panicking the test.
        outcome: RunOutcome::Threw("HarnessError".to_string()),
        output: Vec::new(),
    })
}

/// Browser globals the shim installs as the permissive host proxy. Covers what
/// the sample corpus reaches for (DOM, navigator, storage, timers, workers,
/// messaging); unknown accesses *through* the proxy already resolve to itself,
/// so this list only needs the bare *global* entry points.
const HOST_GLOBALS: &[&str] = &[
    "window",
    "document",
    "self",
    "navigator",
    "location",
    "Image",
    "screen",
    "history",
    "localStorage",
    "sessionStorage",
    "XMLHttpRequest",
    "fetch",
    "top",
    "parent",
    "frames",
    "WebSocket",
    "performance",
    "crypto",
    "postMessage",
    "addEventListener",
    "removeEventListener",
    "HTMLScriptElement",
    "HTMLElement",
    "Node",
    "Element",
    "Worker",
    "requestAnimationFrame",
    "cancelAnimationFrame",
    "setTimeout",
    "setInterval",
    "clearTimeout",
    "clearInterval",
    "getComputedStyle",
    "MutationObserver",
    "Notification",
];

/// One-time harness prelude: determinism freeze, a `console` that records into a
/// global `__lines` array, and a deep permissive host proxy bound to every name
/// in [`HOST_GLOBALS`]. Kept self-contained (no host calls of its own) so it
/// installs cleanly in a bare QuickJS context.
const HARNESS_PRELUDE: &str = r#"
(function(g){
  // Determinism: pin the two ambient nondeterminism sources so a program's
  // signature is reproducible (and so original vs. deobfuscated see identical
  // values). Unlike the partial-evaluation sandbox, here we DO freeze
  // Math.random — we are comparing two whole-program runs, not lifting a value,
  // so a stable RNG is exactly what we want.
  if (typeof g.Date === "function") {
    var _RealDate = g.Date;
    var FrozenDate = function(){
      if (!(this instanceof FrozenDate)) return new _RealDate(0).toString();
      return arguments.length === 0 ? new _RealDate(0) : new _RealDate(...arguments);
    };
    FrozenDate.prototype = _RealDate.prototype;
    FrozenDate.now = function(){ return 0; };
    FrozenDate.parse = _RealDate.parse;
    FrozenDate.UTC = _RealDate.UTC;
    g.Date = FrozenDate;
  }
  g.Math.random = function(){ return 0.5; };

  // Console capture. Every variant funnels into one recorder that stringifies
  // each argument defensively (a throwing toString must not abort the program).
  g.__lines = [];
  var rec = function(){
    var parts = [];
    for (var i = 0; i < arguments.length; i++) {
      try { parts.push(String(arguments[i])); } catch (e) { parts.push("?"); }
    }
    g.__lines.push(parts.join(" "));
  };
  g.console = { log: rec, info: rec, warn: rec, error: rec, debug: rec, trace: rec };

  // Deep permissive host proxy: callable, constructable, indexable; returns
  // itself for any access so chained host expressions resolve instead of
  // throwing. A handful of well-known keys get sensible primitives so coercions
  // (`+host`, `"" + host`, `for…of host`) don't throw either.
  function makeProxy(){
    var f = function(){ return P; };
    var P = new Proxy(f, {
      get: function(t, k){
        if (k === "toString" || k === "valueOf" || k === Symbol.toPrimitive) return function(){ return 0; };
        if (k === "length") return 0;
        if (k === Symbol.iterator) return function(){ return { next: function(){ return { done: true, value: undefined }; } }; };
        if (k === "then") return undefined;            // not a thenable
        return P;
      },
      set: function(){ return true; },
      apply: function(){ return P; },
      construct: function(){ return P; },
      has: function(){ return true; }
    });
    return P;
  }
  g.__hostProxy = makeProxy();
})(globalThis);
"#;

fn behavior_signature_impl(source: &str, budget: Duration) -> Option<BehaviorSignature> {
    let rt = Runtime::new().ok()?;
    rt.set_memory_limit(128 * 1024 * 1024);
    rt.set_max_stack_size(2 * 1024 * 1024);

    let deadline: Deadline = std::sync::Arc::new(Mutex::new(None));
    let handler_deadline = deadline.clone();
    rt.set_interrupt_handler(Some(Box::new(
        move || matches!(*handler_deadline.lock().unwrap(), Some(d) if Instant::now() >= d),
    )));

    let ctx = Context::full(&rt).ok()?;

    // Install the harness, then bind every host global to the shared proxy.
    let bind_hosts = HOST_GLOBALS
        .iter()
        .map(|n| format!("try {{ globalThis[{n:?}] = globalThis.__hostProxy; }} catch (e) {{}}"))
        .collect::<Vec<_>>()
        .join("\n");
    let installed = ctx.with(|ctx| {
        ctx.eval::<rquickjs::Value, _>(HARNESS_PRELUDE).is_ok()
            && ctx
                .eval::<rquickjs::Value, _>(bind_hosts.as_bytes())
                .is_ok()
    });
    if !installed {
        return None;
    }

    let deadline_at = Instant::now() + budget;
    *deadline.lock().unwrap() = Some(deadline_at);
    let outcome = ctx.with(|ctx| {
        let result = ctx.eval::<rquickjs::Value, _>(source);
        // A budget-exhausted run surfaces as an exception (QuickJS raises an
        // InternalError when the interrupt handler trips). Distinguish it by the
        // clock, not the error kind — robust regardless of how the engine maps
        // the interrupt.
        if Instant::now() >= deadline_at {
            return RunOutcome::TimedOut;
        }
        match result {
            Ok(_) => RunOutcome::Completed,
            Err(_) => {
                let exc = ctx.catch();
                let name = exc
                    .as_object()
                    .and_then(|o| o.get::<_, String>("name").ok())
                    .unwrap_or_else(|| "Error".to_string());
                RunOutcome::Threw(name)
            }
        }
    });
    *deadline.lock().unwrap() = None;

    // Read back the captured console output. A missing / clobbered `__lines`
    // (a program could overwrite the global) degrades to "no output".
    let output = ctx.with(|ctx| {
        let mut out = Vec::new();
        if let Ok(arr) = ctx.globals().get::<_, rquickjs::Array>("__lines") {
            for i in 0..arr.len() {
                if let Ok(s) = arr.get::<String>(i) {
                    out.push(s);
                }
            }
        }
        out
    });

    Some(BehaviorSignature { outcome, output })
}

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
