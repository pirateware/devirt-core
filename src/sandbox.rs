//! Sandboxed partial evaluation.
//!
//! Boa (a pure-Rust ECMAScript engine) exposes only ECMAScript built-ins — no
//! filesystem, network, or `require` — so an extracted decoder can compute
//! strings but cannot touch the host. Boa has no wall-clock interrupt or memory
//! cap; instead we impose a deterministic **loop-iteration limit** on the VM so a
//! malicious or non-terminating "decoder" can't spin forever. That limit is
//! uncatchable from JS (a `try/catch` cannot swallow it) and, being instruction-
//! rather than time-based, is reproducible across machines — strictly better for
//! the determinism the lifter relies on than the old per-process wall clock.

use std::cell::RefCell;
use std::sync::Once;
use std::time::Duration;

use boa_engine::object::builtins::JsArray;
use boa_engine::{Context, JsValue, Source};

/// Loop-iteration ceiling for any single evaluation. Boa increments a per-frame
/// counter on every loop back-edge and throws an *uncatchable* `RuntimeLimit`
/// error once this is exceeded — our stand-in for the old wall-clock deadline. It
/// is generous enough for legitimate decoders / table builds yet bounds an
/// infinite loop to a bounded amount of work (which also caps runaway allocation,
/// since Boa has no memory limit of its own).
const LOOP_ITERATION_LIMIT: u64 = 10_000_000;

/// Boa 0.21 has a soundness-irrelevant but process-fatal bug: its integer fast
/// path for `%` computes `x % y` directly, so a decoder that evaluates
/// `(-2147483648) % -1` panics the host (`i32::MIN % -1` overflows in Rust
/// regardless of `overflow-checks`). We can't stop a decoder from doing that, so
/// every sandbox evaluation runs under [`std::panic::catch_unwind`] (see
/// [`Sandbox::guarded`] / [`behavior_signature`]) and a panic is treated as a
/// failed eval. This hook keeps such *expected* engine panics from spamming
/// stderr while leaving genuine panics fully reported.
fn install_panic_filter() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let from_boa = info
                .location()
                .is_some_and(|l| l.file().contains("boa_engine"));
            if !from_boa {
                prev(info);
            }
        }));
    });
}

/// Run `f` under panic containment. A Boa internal panic (e.g. the `%` overflow
/// bug above) unwinds into `Err`, which callers map to a failed eval rather than
/// letting it tear down the pipeline. The caller is responsible for discarding
/// any context that was mid-evaluation, since its VM state may be inconsistent.
fn catch<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// One-time prelude evaluated when the context is created. Boa ships only
/// ECMAScript built-ins, but a common decoder string codec is base64 via the
/// WHATWG `atob`/`btoa` (e.g. DataDome's `function(i){ return atob(table[i]) }`).
/// Without these a base64 decoder throws `ReferenceError` and is left un-lifted.
/// We define spec-correct polyfills *only if absent* (so a future Boa that ships
/// them wins). `atob` uses a 6-bit accumulator so it decodes unpadded input
/// correctly; both return/consume Latin-1 "binary strings" like the host.
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
    // Pin the timezone to UTC. `getTimezoneOffset()` reads the *host* timezone
    // (sample_7 fingerprints with it — it lifted to -60/+300/-330 per machine),
    // and the local-time getters depend on it too. Force them to UTC so any
    // Date-derived lift is portable across machines, matching the frozen clock.
    var DP = _RealDate.prototype;
    DP.getTimezoneOffset = function(){ return 0; };
    DP.getFullYear = DP.getUTCFullYear; DP.getMonth = DP.getUTCMonth;
    DP.getDate = DP.getUTCDate; DP.getDay = DP.getUTCDay;
    DP.getHours = DP.getUTCHours; DP.getMinutes = DP.getUTCMinutes;
    DP.getSeconds = DP.getUTCSeconds; DP.getMilliseconds = DP.getUTCMilliseconds;
    // String formatters embed the host TZ *name* and locale (a fingerprint hash
    // of `new Date().toString()` jittered across machines). Pin them all to the
    // TZ/locale-independent UTC string so any Date-derived lift is portable.
    DP.toString = DP.toDateString = DP.toTimeString =
      DP.toLocaleString = DP.toLocaleDateString = DP.toLocaleTimeString = DP.toUTCString;
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

/// A Boa [`Context`] is single-threaded and its `eval` takes `&mut self`, but the
/// deobfuscation passes drive the sandbox through shared (`&self`) references. A
/// `RefCell` gives us that interior mutability without changing the public API —
/// the borrows never overlap (every eval runs to completion before returning).
///
/// `seed` and `setup_src` retain everything needed to rebuild the context from
/// scratch: if an evaluation panics inside Boa (see [`install_panic_filter`]),
/// the half-evaluated context is discarded and replaced by a freshly built one
/// that has replayed the prelude, the seeded PRNG, and every prior [`setup`]
/// call — so a single poisonous decode never silently corrupts later lifts.
///
/// [`setup`]: Sandbox::setup
pub struct Sandbox {
    ctx: RefCell<Context>,
    seed: u32,
    setup_src: RefCell<String>,
}

/// A decoder result, tagged by JS type so the lifter can splice the *correct*
/// literal kind. Coercing a numeric result to a string literal (the old
/// `eval_to_string`-only path) silently changed semantics: `dec(x) + 1` would go
/// from numeric `262` to string `"2611"`.
///
/// `Arr` carries a flat array of primitives (no nesting, no holes) so an
/// array-returning decoder can be materialized as an array literal. It is
/// deliberately shallow: the splice path caps the element count so a huge bytecode
/// table is never expanded (net-negative — see `MAX_ARRAY_ELEMS`).
#[derive(Debug, Clone, PartialEq)]
pub enum EvalValue {
    Str(String),
    Num(f64),
    Bool(bool),
    Arr(Vec<EvalValue>),
    /// A plain object's own enumerable string-keyed entries, in key order. Produced
    /// only for ordinary objects (used by the `JSON.parse` fold, whose result is by
    /// spec a plain data object); the splice path materializes it as an object
    /// literal, capped by the same element budget as arrays.
    Obj(Vec<(String, EvalValue)>),
}

/// Default `Math.random` seed for the partial-evaluation sandbox. Boa seeds
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
        // Experiment hatch: `DEOB_NO_SANDBOX=1` disables all sandbox-backed passes
        // (decoder_lift / math_fingerprint / pure_eval bail to no-op on None).
        if std::env::var_os("DEOB_NO_SANDBOX").is_some() {
            return None;
        }
        install_panic_filter();
        let ctx = build_context(seed, "")?;
        Some(Sandbox {
            ctx: RefCell::new(ctx),
            seed,
            setup_src: RefCell::new(String::new()),
        })
    }

    /// Run `code` once for its side effects (define the decoder library /
    /// rotate the array) in the persistent context. Returns whether it ran
    /// without throwing. Subsequent `eval_to_string` calls see the definitions.
    /// The code is also retained so a post-panic context rebuild can replay it.
    pub fn setup(&self, code: &str) -> bool {
        let ok = self
            .guarded(|ctx| ctx.eval(Source::from_bytes(code)).is_ok())
            .unwrap_or(false);
        if ok {
            let mut acc = self.setup_src.borrow_mut();
            acc.push_str(code);
            acc.push('\n');
        }
        ok
    }

    /// Evaluate `code` (ending in an expression) and return it coerced to a
    /// string. Returns `None` on any error, timeout, or non-stringable result.
    pub fn eval_to_string(&self, code: &str) -> Option<String> {
        self.guarded(|ctx| {
            let v = ctx.eval(Source::from_bytes(code)).ok()?;
            if let Some(s) = v.as_string() {
                Some(s.to_std_string_lossy())
            } else {
                v.as_number().map(|n| {
                    if n.fract() == 0.0 {
                        format!("{}", n as i64)
                    } else {
                        format!("{n}")
                    }
                })
            }
        })
        .flatten()
    }

    /// Evaluate `code` (ending in an expression) and return its primitive value
    /// tagged by type. Returns `None` on any error, timeout, or non-primitive
    /// (object/array/function/undefined/null) result — those are not safe to
    /// splice as a single literal.
    pub fn eval_to_value(&self, code: &str) -> Option<EvalValue> {
        self.guarded(|ctx| {
            let v = ctx.eval(Source::from_bytes(code)).ok()?;
            let mut budget = MAX_ARRAY_ELEMS;
            value_to_eval(&v, &mut budget, ctx)
        })
        .flatten()
    }

    /// Run `f` against the persistent context under panic containment. If Boa
    /// panics mid-evaluation, the (possibly inconsistent) context is rebuilt from
    /// the seed + retained setup source and `None` is returned, so a single
    /// poisonous decode neither crashes the host nor corrupts later lifts.
    fn guarded<T>(&self, f: impl FnOnce(&mut Context) -> T) -> Option<T> {
        let mut ctx = self.ctx.borrow_mut();
        match catch(|| f(&mut ctx)) {
            Some(v) => Some(v),
            None => {
                drop(ctx);
                if let Some(fresh) = build_context(self.seed, &self.setup_src.borrow()) {
                    *self.ctx.borrow_mut() = fresh;
                }
                None
            }
        }
    }
}

/// Build a fresh sandbox context: bound runaway loops (our stand-in for the
/// wall-clock deadline; other limits keep Boa's stack-safe defaults), install the
/// base64 polyfills + Date freeze, the seeded PRNG, and replay `setup_src`.
fn build_context(seed: u32, setup_src: &str) -> Option<Context> {
    let mut ctx = Context::default();
    ctx.runtime_limits_mut()
        .set_loop_iteration_limit(LOOP_ITERATION_LIMIT);
    let built = catch(|| {
        ctx.eval(Source::from_bytes(SANDBOX_PRELUDE)).is_ok()
            && ctx
                .eval(Source::from_bytes(&seeded_random_js(seed)))
                .is_ok()
            && (setup_src.is_empty() || ctx.eval(Source::from_bytes(setup_src)).is_ok())
    })?;
    built.then_some(ctx)
}

/// Hard cap on the number of array elements (across all nesting) any single
/// `eval_to_value` result may materialize. This keeps the array-splice from ever
/// expanding a large packed table (e.g. a VM bytecode array) into a giant literal,
/// which would bloat output for no readability win.
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
/// * **Isolation + bounds.** A fresh context per call (no cross-run state) and a
///   loop-iteration limit (a non-terminating program trips it and becomes
///   [`RunOutcome::TimedOut`], never a hang). Boa has no wall clock, so the
///   `budget` argument is retained for API compatibility but the bound is the
///   deterministic iteration ceiling, not elapsed time.
///
/// This is the engine behind the full-sample equivalence regression test: run a
/// sample and its deobfuscated form, require identical signatures.
pub fn behavior_signature(source: &str, budget: Duration) -> BehaviorSignature {
    install_panic_filter();
    let fallback = || BehaviorSignature {
        // A harness that fails to even construct its runtime is reported as a
        // distinct, comparable outcome rather than panicking the test.
        outcome: RunOutcome::Threw("HarnessError".to_string()),
        output: Vec::new(),
    };
    // Boa's recursive-descent parser, bytecompiler, and tree-walking VM lean on
    // the native Rust stack far more than QuickJS's bytecode VM did, so a large
    // obfuscated program can overflow a default (~2-8 MiB) test/server stack.
    // `run_isolated` gives it a big dedicated stack (a stack overflow aborts and
    // is *not* catchable, so it must be prevented, not contained). The
    // whole-program run is also panic-contained: a Boa internal panic (e.g. its
    // `%` overflow bug) collapses to the same comparable `HarnessError`, never a
    // host crash.
    const HARNESS_STACK: usize = 256 * 1024 * 1024;
    crate::util::run_isolated("behavior-signature", HARNESS_STACK, || {
        catch(|| behavior_signature_impl(source, budget))
            .flatten()
            .unwrap_or_else(fallback)
    })
    .unwrap_or_else(|_| fallback())
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
/// installs cleanly in a bare Boa context.
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
    // Pin the timezone to UTC. `getTimezoneOffset()` reads the *host* timezone
    // (sample_7 fingerprints with it — it lifted to -60/+300/-330 per machine),
    // and the local-time getters depend on it too. Force them to UTC so any
    // Date-derived lift is portable across machines, matching the frozen clock.
    var DP = _RealDate.prototype;
    DP.getTimezoneOffset = function(){ return 0; };
    DP.getFullYear = DP.getUTCFullYear; DP.getMonth = DP.getUTCMonth;
    DP.getDate = DP.getUTCDate; DP.getDay = DP.getUTCDay;
    DP.getHours = DP.getUTCHours; DP.getMinutes = DP.getUTCMinutes;
    DP.getSeconds = DP.getUTCSeconds; DP.getMilliseconds = DP.getUTCMilliseconds;
    // String formatters embed the host TZ *name* and locale (a fingerprint hash
    // of `new Date().toString()` jittered across machines). Pin them all to the
    // TZ/locale-independent UTC string so any Date-derived lift is portable.
    DP.toString = DP.toDateString = DP.toTimeString =
      DP.toLocaleString = DP.toLocaleDateString = DP.toLocaleTimeString = DP.toUTCString;
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

fn behavior_signature_impl(source: &str, _budget: Duration) -> Option<BehaviorSignature> {
    let mut ctx = Context::default();
    // A non-terminating program trips this and surfaces as an uncatchable
    // `RuntimeLimit` error, which we map to `TimedOut` below. (Boa has no wall
    // clock; this is the deterministic equivalent of the old per-call budget.)
    ctx.runtime_limits_mut()
        .set_loop_iteration_limit(LOOP_ITERATION_LIMIT);

    // Install the harness, then bind every host global to the shared proxy.
    let bind_hosts = HOST_GLOBALS
        .iter()
        .map(|n| format!("try {{ globalThis[{n:?}] = globalThis.__hostProxy; }} catch (e) {{}}"))
        .collect::<Vec<_>>()
        .join("\n");
    if ctx.eval(Source::from_bytes(HARNESS_PRELUDE)).is_err()
        || ctx.eval(Source::from_bytes(&bind_hosts)).is_err()
    {
        return None;
    }

    let outcome = match ctx.eval(Source::from_bytes(source)) {
        Ok(_) => RunOutcome::Completed,
        Err(err) => {
            // The loop-iteration ceiling raises an uncatchable `RuntimeLimit`
            // error; treat that (and only that) as non-termination. Every other
            // throw carries its constructor name, which is part of the signature.
            if err.try_native(&mut ctx).is_ok_and(|n| n.is_runtime_limit()) {
                RunOutcome::TimedOut
            } else {
                let name = err
                    .to_opaque(&mut ctx)
                    .as_object()
                    .and_then(|o| o.get(boa_engine::js_string!("name"), &mut ctx).ok())
                    .and_then(|n| n.as_string().map(|s| s.to_std_string_lossy()))
                    .unwrap_or_else(|| "Error".to_string());
                RunOutcome::Threw(name)
            }
        }
    };

    // Read back the captured console output. A missing / clobbered `__lines`
    // (a program could overwrite the global) degrades to "no output".
    let output = read_lines(&mut ctx).unwrap_or_default();

    Some(BehaviorSignature { outcome, output })
}

/// Read the harness's `__lines` console-capture array back into a `Vec<String>`.
/// Returns `None` if the global is missing or not an array (degrades to no output).
fn read_lines(ctx: &mut Context) -> Option<Vec<String>> {
    let lines = ctx
        .global_object()
        .get(boa_engine::js_string!("__lines"), ctx)
        .ok()?;
    let obj = lines.as_object()?;
    if !obj.is_array() {
        return None;
    }
    let arr = JsArray::from_object(obj).ok()?;
    let len = usize::try_from(arr.length(ctx).ok()?).ok()?;
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        if let Some(s) = arr.at(i as i64, ctx).ok().and_then(|v| v.as_string()) {
            out.push(s.to_std_string_lossy());
        }
    }
    Some(out)
}

/// Convert a Boa value to an [`EvalValue`], counting array elements against
/// `budget`. Returns `None` for any non-primitive/non-flat-array value, a hole, or
/// when the element budget is exhausted — those are not safe/desirable to splice.
fn value_to_eval(v: &JsValue, budget: &mut usize, ctx: &mut Context) -> Option<EvalValue> {
    if let Some(s) = v.as_string() {
        return Some(EvalValue::Str(s.to_std_string_lossy()));
    }
    if let Some(n) = v.as_number() {
        return Some(EvalValue::Num(n));
    }
    if let Some(b) = v.as_boolean() {
        return Some(EvalValue::Bool(b));
    }
    if let Some(obj) = v.as_object() {
        if obj.is_array() {
            let arr = JsArray::from_object(obj).ok()?;
            let len = usize::try_from(arr.length(ctx).ok()?).ok()?;
            if len > *budget {
                return None; // would exceed the materialization cap
            }
            *budget -= len;
            let mut out = Vec::with_capacity(len);
            for i in 0..len {
                // A hole reads back as `undefined` here, and a non-convertible
                // element returns `None` — either disqualifies the whole array,
                // since splicing it would change semantics or require a non-literal
                // form.
                let elem = arr.at(i as i64, ctx).ok()?;
                out.push(value_to_eval(&elem, budget, ctx)?);
            }
            return Some(EvalValue::Arr(out));
        }
        // A plain ordinary object: materialize its own enumerable string-keyed
        // entries. Restricted to ordinary objects (no exotic/proxy behavior); the
        // caller (the `JSON.parse` fold) further guarantees the result is a plain
        // data object with no accessors, so reading each value is faithful.
        if !obj.is_ordinary() {
            return None;
        }
        let keys = obj.own_property_keys(ctx).ok()?;
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            // Only string keys (no symbols / array-index exotics). The caller
            // guarantees the object is plain (a `JSON.parse` result), so every own
            // string key is an enumerable data property — read its value directly.
            let boa_engine::property::PropertyKey::String(s) = &key else {
                return None;
            };
            if *budget == 0 {
                return None;
            }
            *budget -= 1;
            let name = s.to_std_string_lossy();
            let val = obj.get(key.clone(), ctx).ok()?;
            out.push((name, value_to_eval(&val, budget, ctx)?));
        }
        return Some(EvalValue::Obj(out));
    }
    None
}
