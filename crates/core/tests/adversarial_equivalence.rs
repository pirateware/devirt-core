//! Adversarial whole-program soundness battery.
//!
//! A curated set of programs that combine obfuscation features (string-array
//! rotation, decoders, control-flow flattening, operator proxies, value
//! evaluation) with semantics that are easy to get wrong (exception flow, getters,
//! aliasing, number/Boolean coercion, host globals, async/generators). Each is run
//! through the host-shim harness before and after deobfuscation; the observable
//! behavior must match. These are the regression guards distilled from fuzzing —
//! several encode bugs that were found and fixed (rotation replay, unbound host
//! globals under `try`/`typeof`, async/generator inlining, the nested-dispatcher
//! panic).

use devirt_core::deobfuscate;
use devirt_core::sandbox::{behavior_signature, RunOutcome};
use std::time::Duration;

fn equiv(src: &str) {
    let b = Duration::from_secs(5);
    let o = behavior_signature(src, b);
    if o.outcome == RunOutcome::TimedOut {
        return; // not a comparable outcome
    }
    let d = deobfuscate(src, "t.js");
    assert!(d.parse_errors.is_empty(), "parse errors: {:?}", d.parse_errors);
    let a = behavior_signature(&d.code, b);
    assert_eq!(o, a, "behavior changed\n--src--\n{src}\n--deob--\n{}", d.code);
}

macro_rules! cases {
    ($($name:ident: $src:expr;)*) => {
        $(#[test] fn $name() { equiv($src); })*
    };
}

cases! {
    rotation_numeric_array: "var a=[10,20,30,40];(function(x,n){while(--n)x.push(x.shift());})(a,2);function g(i){return a[i];}console.log(g(0)+g(1));";
    rotation_double: "var a=['p','q','r'];(function(x,n){while(--n)x.push(x.shift());})(a,2);(function(x,n){while(--n)x.push(x.shift());})(a,2);console.log(a.join(''));";
    rotation_base64: "var a=['SGk=','Qnll'];(function(x,n){while(--n)x.push(x.shift());})(a,2);function d(i){return atob(a[i]);}console.log(d(0)+d(1));";
    decoder_in_cff: "var a=['foo','bar'];function d(i){return a[i];}var s=1,out='';while(s){switch(s){case 1:out+=d(0);s=2;break;case 2:out+=d(1);s=0;break;}}console.log(out);";
    memoized_decoder: "var c={},t=['A','B'];function d(i){return c[i]!==undefined?c[i]:(c[i]=t[i].toLowerCase());}console.log(d(0)+d(1)+d(0));";
    decoder_numeric_arith: "var a=[10,20,30];function d(i){return a[i];}console.log(d(0)+d(1)*2);";
    try_finally_return: "function f(){try{return 1;}finally{return 2;}}console.log(f());";
    nested_try_finally: "var r='';try{try{throw 1;}finally{r+='f1';}}catch(e){r+='c';}console.log(r);";
    getter_side_effect: "var log=[];var o={get x(){log.push('g');return 5;}};var a=o.x+o.x;console.log(a,log.length);";
    alias_mutation: "var a=[1,2],b=a;b.push(3);console.log(a.length);";
    host_global_try: "var out=(function(){try{return window.k;}catch(e){return 99;}})();console.log(typeof out);";
    typeof_host_global: "console.log((function(){return typeof window;})());";
    async_not_inlined: "async function a(){return 21;}a().then(function(v){console.log('r',v);});console.log('s');";
    generator_not_inlined: "function* g(){return 5;}console.log(typeof g().next);";
    nested_dispatchers: "function f(x){var s=1,r=0;while(s){switch(s){case 1:r=x;s=2;break;case 2:{var t=1,m=0;while(t){switch(t){case 1:m=r*2;t=0;break;}}r=m;}s=0;break;}}return r;}console.log(f(7));";
    number_edges: "console.log(0.1+0.2,(2**53),(-5)%3,7&-1,~5);";
    coercion_chain: "var a=[1,'2',3];function d(i){return a[i];}console.log(d(0)+d(1)+d(2));";
    string_reverse_idiom: "console.log('!dlroW'.split('').reverse().join(''));";
    operator_proxy_effects: "var log='';function f(){log+='f';return 1;}function g(){log+='g';return 2;}var P={s:function(a,b){return a-b;}};var v=P.s(f(),g());console.log(v,log);";
    effect_in_cff_transition: "var log='';var s=1;while(s){switch(s){case 1:s=(log+='a',2);break;case 2:log+='b';s=0;break;}}console.log(log);";
    template_with_effects: "var c=0;function f(){c++;return c;}console.log(`${f()}-${f()}`,c);";
    json_config: "var cfg=JSON.parse('{\"a\":1,\"b\":[2,3]}');console.log(cfg.a+cfg.b[1]);";
    pure_iife_vm: "console.log((function(){var p=[1,7,1,35,2],st=[];for(var i=0;i<p.length;i++){var op=p[i];if(op===1){st.push(p[++i]);}else{var x=st.pop(),y=st.pop();st.push(x+y);}}return st.pop();})());";
    closure_loop_var: "var fns=[];for(var i=0;i<3;i++){fns.push(function(){return i;});}console.log(fns[0]());";
    let_closure_loop: "var fns=[];for(let i=0;i<3;i++){fns.push(function(){return i;});}console.log(fns[0](),fns[2]());";
}
