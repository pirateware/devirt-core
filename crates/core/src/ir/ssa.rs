//! SSA-style reaching-definition resolution.
//!
//! We don't materialise a full SSA program; we use SSA *renaming* purely as the
//! analysis that answers, for every candidate use, "which single definition
//! reaches here?" — soundly across branches and loops. Phis are placed at the
//! iterated dominance frontier of each symbol's definition sites (Cytron et al.);
//! a use that renames to a phi or to the entry (param / `undefined`) value is
//! reported as *non-constant* and never folds. A use that renames to a concrete
//! constant definition carries that definition's value, ready to inline.
//!
//! This is what makes the fold sound where the structural passes were blind: the
//! reassignment trap
//! ```text
//!   s = [1,2]; if (c) s = [3,4]; use s[0];
//! ```
//! places a phi at the merge, so `use s[0]` resolves to the phi (non-const) and
//! is left alone — exactly right, since the value depends on `c`.

use std::collections::{HashMap, HashSet};

use oxc_syntax::symbol::SymbolId;

use super::cfg::{Cfg, DefVal, Event, NodeId, UseKind};
use super::dom::{BlockId, Dominators};

/// A use whose reaching definition is a single concrete (non-phi) definition,
/// resolved to the rewrite it enables.
pub enum Resolved<'c, 'a> {
    /// A use reached by a constant definition — fold to the constant.
    Const {
        node: NodeId,
        kind: &'c UseKind,
        val: &'c DefVal<'a>,
    },
    /// A scalar use reached by a copy `s = src` where `src` is provably unchanged
    /// since the copy — replace the use with `src`.
    Copy { node: NodeId, src: SymbolId },
}

impl<'c, 'a> Resolved<'c, 'a> {
    #[allow(dead_code)] // used by the unit tests
    pub fn node(&self) -> NodeId {
        match self {
            Resolved::Const { node, .. } | Resolved::Copy { node, .. } => *node,
        }
    }
}

/// What a live SSA version carries, for resolving uses. `Copy` is reflexively
/// `Copy` (it holds only refs / small values), so it can be lifted off the stack
/// cheaply during the rename walk.
#[derive(Clone, Copy)]
enum Reaching<'c, 'a> {
    /// Entry/param value or a phi result — never folds.
    NonConst,
    /// A concrete definition's classified value.
    Concrete(&'c DefVal<'a>),
    /// `s = src`, captured with the version id `src` held at the copy point.
    Copy { src: SymbolId, src_version: u32 },
}

/// Resolve every candidate use to its reaching definition; return only those
/// reached by a single concrete definition or a stable copy (the fold sites).
pub fn resolve<'c, 'a>(cfg: &'c Cfg<'a>, dom: &Dominators) -> Vec<Resolved<'c, 'a>> {
    let n = cfg.blocks.len();

    // Definition sites per symbol (blocks containing at least one Def).
    let mut defsites: HashMap<SymbolId, HashSet<BlockId>> = HashMap::new();
    let mut symbols: HashSet<SymbolId> = HashSet::new();
    for (b, block) in cfg.blocks.iter().enumerate() {
        if !dom.reachable(b) {
            continue;
        }
        for ev in &block.events {
            match ev {
                Event::Def { sym, .. } => {
                    defsites.entry(*sym).or_default().insert(b);
                    symbols.insert(*sym);
                }
                Event::Use { sym, .. } => {
                    symbols.insert(*sym);
                }
                Event::Barrier => {}
            }
        }
    }

    // Phi placement: iterated dominance frontier of each symbol's def sites.
    // We only need *where* phis are (a phi result is always non-constant), not
    // their operands.
    let mut has_phi: Vec<HashSet<SymbolId>> = vec![HashSet::new(); n];
    for (&sym, sites) in &defsites {
        let mut worklist: Vec<BlockId> = sites.iter().copied().collect();
        let mut on_worklist: HashSet<BlockId> = sites.iter().copied().collect();
        let mut placed: HashSet<BlockId> = HashSet::new();
        while let Some(b) = worklist.pop() {
            for &df in dom.frontier(b) {
                if placed.insert(df) {
                    has_phi[df].insert(sym);
                    if on_worklist.insert(df) {
                        worklist.push(df);
                    }
                }
            }
        }
    }

    // Build dominator-tree children for the rename DFS.
    let mut children: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    for b in 0..n {
        if b == cfg.entry || !dom.reachable(b) {
            continue;
        }
        if let Some(d) = dom.idom(b) {
            children[d].push(b);
        }
    }

    // Rename: a dominator-tree DFS maintaining, per symbol, a stack of reaching
    // values tagged with a globally-unique version id. The version id lets a copy
    // `s = src` check, at each use of `s`, whether `src` still holds the exact
    // version it held at the copy (same id ⇒ unchanged ⇒ safe to propagate).
    let mut next_ver: u32 = 0;
    let mut fresh = || {
        let v = next_ver;
        next_ver += 1;
        v
    };
    let mut stacks: HashMap<SymbolId, Vec<(u32, Reaching)>> = HashMap::new();
    for &sym in &symbols {
        stacks.insert(sym, vec![(fresh(), Reaching::NonConst)]); // entry / param value
    }

    let mut resolved: Vec<Resolved> = Vec::new();

    // Iterative DFS over the dom tree. Each frame remembers, per symbol, how many
    // pushes to undo when the subtree is done.
    enum Step {
        Enter(BlockId),
        Pop(Vec<(SymbolId, usize)>),
    }
    let mut stack: Vec<Step> = vec![Step::Enter(cfg.entry)];

    while let Some(step) = stack.pop() {
        match step {
            Step::Pop(pushes) => {
                for (sym, count) in pushes {
                    let s = stacks.get_mut(&sym).unwrap();
                    s.truncate(s.len() - count);
                }
            }
            Step::Enter(b) => {
                let mut pushes: HashMap<SymbolId, usize> = HashMap::new();

                // Phi results: a fresh non-constant version at the block head.
                for &sym in &has_phi[b] {
                    let v = fresh();
                    stacks.get_mut(&sym).unwrap().push((v, Reaching::NonConst));
                    *pushes.entry(sym).or_default() += 1;
                }

                for ev in &cfg.blocks[b].events {
                    match ev {
                        Event::Use { sym, kind, node } => {
                            let reaching = stacks.get(sym).and_then(|s| s.last()).map(|(_, r)| *r);
                            match reaching {
                                Some(Reaching::Concrete(val)) => {
                                    resolved.push(Resolved::Const {
                                        node: *node,
                                        kind,
                                        val,
                                    });
                                }
                                Some(Reaching::Copy { src, src_version }) => {
                                    // Propagate only a bare (scalar) read, and only
                                    // if `src` still holds the copied version.
                                    let cur =
                                        stacks.get(&src).and_then(|s| s.last()).map(|(v, _)| *v);
                                    if matches!(kind, UseKind::Scalar) && cur == Some(src_version) {
                                        resolved.push(Resolved::Copy { node: *node, src });
                                    }
                                }
                                _ => {}
                            }
                        }
                        Event::Def { sym, val, .. } => {
                            let reaching = match val {
                                DefVal::Copy(src) => {
                                    let sv = stacks
                                        .get(src)
                                        .and_then(|s| s.last())
                                        .map(|(v, _)| *v)
                                        .unwrap_or(0);
                                    Reaching::Copy {
                                        src: *src,
                                        src_version: sv,
                                    }
                                }
                                other => Reaching::Concrete(other),
                            };
                            let v = fresh();
                            stacks.get_mut(sym).unwrap().push((v, reaching));
                            *pushes.entry(*sym).or_default() += 1;
                        }
                        Event::Barrier => {}
                    }
                }

                // Schedule the pop after the whole subtree, then enter children.
                stack.push(Step::Pop(pushes.into_iter().collect()));
                for &c in &children[b] {
                    stack.push(Step::Enter(c));
                }
            }
        }
    }

    resolved
}

#[cfg(test)]
mod tests {
    //! SSA-validity-style checks expressed through the resolver: a use folds iff
    //! a single concrete const def reaches it. These run on hand-built CFGs.
    use super::*;
    use crate::ir::cfg::{Block, DefVal, Key};

    // Helper to fabricate a tiny CFG with synthetic events. We use a dummy
    // SymbolId and synthetic NodeIds via Address(usize).
    fn addr(n: usize) -> NodeId {
        // SAFETY-free: Address is a transparent usize newtype used only as a key.
        unsafe { oxc_allocator::Address::from_ptr(n as *const u8) }
    }
    fn sym(n: u32) -> SymbolId {
        SymbolId::new(n as usize)
    }

    // Build a CFG from explicit blocks (events) + succs and resolve.
    fn run(blocks: Vec<Vec<Event<'static>>>, succs: Vec<Vec<BlockId>>) -> Vec<NodeId> {
        let cfg = Cfg {
            blocks: blocks.into_iter().map(|events| Block { events }).collect(),
            succs: succs.clone(),
            entry: 0,
        };
        let dom = Dominators::compute(&cfg.succs, 0);
        resolve(&cfg, &dom).into_iter().map(|r| r.node()).collect()
    }

    #[test]
    fn dominating_def_folds() {
        // b0: s = <concrete const> ; use s    → folds. (Array(empty) stands in as
        // a concrete const value; resolution only checks concreteness, not shape.)
        let s = sym(0);
        let folded = run(
            vec![vec![
                Event::Def {
                    sym: s,
                    val: DefVal::Array(vec![]),
                    assign: None,
                    decl_init: None,
                },
                Event::Use {
                    sym: s,
                    kind: UseKind::Member(Key::Index(0)),
                    node: addr(1),
                },
            ]],
            vec![vec![]],
        );
        assert_eq!(folded, vec![addr(1)]);
    }

    #[test]
    fn use_before_def_does_not_fold() {
        let s = sym(0);
        let folded = run(
            vec![vec![
                Event::Use {
                    sym: s,
                    kind: UseKind::Scalar,
                    node: addr(1),
                },
                Event::Def {
                    sym: s,
                    val: DefVal::Array(vec![]),
                    assign: None,
                    decl_init: None,
                },
            ]],
            vec![vec![]],
        );
        assert!(folded.is_empty(), "use before def must see the entry value");
    }

    #[test]
    fn branch_merge_inserts_phi() {
        // b0 -> {b1: s=A, b2: s=B} -> b3: use s   → phi at b3, no fold.
        let s = sym(0);
        let folded = run(
            vec![
                vec![], // b0
                vec![Event::Def {
                    sym: s,
                    val: DefVal::Array(vec![]),
                    assign: None,
                    decl_init: None,
                }], // b1
                vec![Event::Def {
                    sym: s,
                    val: DefVal::Array(vec![]),
                    assign: None,
                    decl_init: None,
                }], // b2
                vec![Event::Use {
                    sym: s,
                    kind: UseKind::Scalar,
                    node: addr(9),
                }], // b3
            ],
            vec![vec![1, 2], vec![3], vec![3], vec![]],
        );
        assert!(
            folded.is_empty(),
            "merge of two defs must be a phi (non-const)"
        );
    }

    #[test]
    fn reassignment_trap_no_fold() {
        // b0: s=A ; b0->{b1: s=B}; b1,b0 -> b2: use s
        // s=A dominates the use, but the c-path reassigns to B → phi at b2.
        let s = sym(0);
        let folded = run(
            vec![
                vec![Event::Def {
                    sym: s,
                    val: DefVal::Array(vec![]),
                    assign: None,
                    decl_init: None,
                }], // b0
                vec![Event::Def {
                    sym: s,
                    val: DefVal::Array(vec![]),
                    assign: None,
                    decl_init: None,
                }], // b1
                vec![Event::Use {
                    sym: s,
                    kind: UseKind::Scalar,
                    node: addr(7),
                }], // b2
            ],
            vec![vec![1, 2], vec![2], vec![]],
        );
        assert!(
            folded.is_empty(),
            "reassignment on one path must block the fold"
        );
    }

    #[test]
    fn short_circuit_dominating_def_folds() {
        // models `a && (s = A) && use(s[0])`:
        // b0(eval a) -> {b1, b3}; b1(s=A) -> {b2, b3}; b2(use s) -> b3; b3 merge.
        let s = sym(0);
        let folded = run(
            vec![
                vec![], // b0 eval a
                vec![Event::Def {
                    sym: s,
                    val: DefVal::Array(vec![]),
                    assign: None,
                    decl_init: None,
                }], // b1 s=A
                vec![Event::Use {
                    sym: s,
                    kind: UseKind::Member(Key::Index(0)),
                    node: addr(5),
                }], // b2 use
                vec![], // b3 merge
            ],
            vec![vec![1, 3], vec![2, 3], vec![3], vec![]],
        );
        assert_eq!(folded, vec![addr(5)], "use dominated by the def folds");
    }
}
