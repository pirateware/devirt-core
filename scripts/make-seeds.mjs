#!/usr/bin/env node
//
// Generate a bank of small, varied, self-contained JS programs to use as
// obfuscation seeds. Deterministic: program N is a pure function of N (an
// index-seeded LCG drives all variation), so regenerating yields identical
// files. Writes samples/seeds/.
//
//   node scripts/make-seeds.mjs                # default 200 seeds
//   node scripts/make-seeds.mjs --count 400
//
// The point is variety of *structure* (loops, recursion, closures, objects,
// string/array kernels) so the obfuscator has diverse code to transform and the
// deobfuscator is exercised broadly — not behavioral correctness, which the
// corpus does not assert.
import { writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const OUT = "samples/seeds";
const args = process.argv.slice(2);
const count = parseInt((args[args.indexOf("--count") + 1] ?? "200"), 10) || 200;

// Tiny deterministic PRNG (mulberry32) seeded per-program.
function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const pick = (r, arr) => arr[Math.floor(r() * arr.length)];
const int = (r, lo, hi) => lo + Math.floor(r() * (hi - lo + 1));

// Each template returns a complete program string. `r` varies constants/names.
const TEMPLATES = [
  (r) => {
    const n = int(r, 8, 40);
    return `function sumOfSquares(n){let t=0;for(let i=1;i<=n;i++)t+=i*i;return t;}console.log(sumOfSquares(${n}));`;
  },
  (r) => {
    const a = int(r, 2, 9), b = int(r, 10, 30);
    return `function gcd(a,b){while(b){[a,b]=[b,a%b];}return a;}console.log(gcd(${a*b},${a}));`;
  },
  (r) => {
    const n = int(r, 6, 15);
    return `function fib(n){let a=0,b=1;while(n-->0){const t=a+b;a=b;b=t;}return a;}console.log([...Array(${n}).keys()].map(fib).join(","));`;
  },
  (r) => {
    const arr = Array.from({ length: int(r, 5, 10) }, () => int(r, 0, 99));
    return `function bubble(a){a=a.slice();for(let i=0;i<a.length;i++)for(let j=0;j<a.length-1-i;j++)if(a[j]>a[j+1])[a[j],a[j+1]]=[a[j+1],a[j]];return a;}console.log(bubble(${JSON.stringify(arr)}).join(","));`;
  },
  (r) => {
    const w = pick(r, ["racecar", "hello", "level", "world", "rotor", "stack"]);
    return `function isPalindrome(s){s=s.toLowerCase().replace(/[^a-z0-9]/g,"");return s===s.split("").reverse().join("");}console.log(isPalindrome(${JSON.stringify(w)}));`;
  },
  (r) => {
    const n = int(r, 3, 8);
    return `function fact(n){return n<2?1:n*fact(n-1);}console.log(fact(${n}));`;
  },
  (r) => {
    const k = int(r, 2, 5);
    return `const make=(k)=>{let c=0;return ()=>{c+=k;return c;};};const step=make(${k});console.log(step()+step()+step());`;
  },
  (r) => {
    const keys = ["x", "y", "z", "w"].slice(0, int(r, 2, 4));
    const obj = Object.fromEntries(keys.map((k) => [k, int(r, 1, 9)]));
    return `const o=${JSON.stringify(obj)};const sum=Object.keys(o).reduce((a,k)=>a+o[k],0);console.log(JSON.stringify(o),sum);`;
  },
  (r) => {
    const s = pick(r, ["the quick brown fox", "lorem ipsum dolor", "a b c d e f g"]);
    return `function wordCount(s){return s.trim().split(/\\s+/).reduce((m,w)=>{m[w]=(m[w]||0)+1;return m;},{});}console.log(JSON.stringify(wordCount(${JSON.stringify(s)})));`;
  },
  (r) => {
    const n = int(r, 10, 30);
    return `function primesUpTo(n){const s=[];for(let i=2;i<=n;i++){if(s.every(p=>i%p))s.push(i);}return s;}console.log(primesUpTo(${n}).join(","));`;
  },
  (r) => {
    const target = int(r, 5, 20);
    const arr = Array.from({ length: 6 }, () => int(r, 1, 15)).sort((a, b) => a - b);
    return `function binarySearch(a,t){let lo=0,hi=a.length-1;while(lo<=hi){const m=(lo+hi)>>1;if(a[m]===t)return m;if(a[m]<t)lo=m+1;else hi=m-1;}return -1;}console.log(binarySearch(${JSON.stringify(arr)},${target}));`;
  },
  (r) => {
    const n = int(r, 3, 6);
    return `function* range(n){for(let i=0;i<n;i++)yield i*i;}console.log([...range(${n})].join(","));`;
  },
];

mkdirSync(OUT, { recursive: true });
let written = 0;
const tags = ["sq", "gcd", "fib", "sort", "pal", "fact", "closure", "obj", "wc", "prime", "bsearch", "gen"];
for (let i = 0; i < count; i++) {
  const r = rng(i + 1);
  const tIdx = i % TEMPLATES.length;
  const code = TEMPLATES[tIdx](r);
  const name = `s${String(i).padStart(4, "0")}_${tags[tIdx]}.js`;
  writeFileSync(join(OUT, name), code + "\n");
  written++;
}
console.log(`Wrote ${written} seeds into ${OUT}/`);
