//! Dominator tree and dominance frontier.
//!
//! Pure graph code over a block-numbered CFG — no oxc types — so it is testable
//! in isolation (see the unit tests at the bottom: diamonds, loops, nested).
//!
//! The dominator tree uses the iterative Cooper–Harvey–Kennedy algorithm ("A
//! Simple, Fast Dominance Algorithm"). Functions here are tiny, so the iterative
//! form's clarity beats Lengauer–Tarjan's asymptotics (plan §11 Q3).
//!
//! `BlockId::MAX` is the sentinel for "undefined idom" during the fixpoint.

pub type BlockId = usize;

const UNDEFINED: BlockId = usize::MAX;

/// Dominator information for one CFG.
pub struct Dominators {
    /// Immediate dominator of each block. `idom[entry] == entry`. A block
    /// unreachable from entry has `idom == UNDEFINED`.
    idom: Vec<BlockId>,
    /// Reverse-postorder numbering: `rpo_num[b]` is `b`'s index in RPO, or
    /// `UNDEFINED` if `b` is unreachable.
    rpo_num: Vec<BlockId>,
    /// Dominance frontier of each block.
    frontier: Vec<Vec<BlockId>>,
}

impl Dominators {
    /// Compute dominators + dominance frontier from a successor list and entry.
    pub fn compute(succs: &[Vec<BlockId>], entry: BlockId) -> Dominators {
        let n = succs.len();
        let preds = predecessors(succs);

        let rpo = reverse_postorder(succs, entry);
        let mut rpo_num = vec![UNDEFINED; n];
        for (i, &b) in rpo.iter().enumerate() {
            rpo_num[b] = i;
        }

        let mut idom = vec![UNDEFINED; n];
        idom[entry] = entry;

        let mut changed = true;
        while changed {
            changed = false;
            // Process in RPO, skipping the entry (first element).
            for &b in rpo.iter().skip(1) {
                // New idom = intersection of all already-processed preds.
                let mut new_idom = UNDEFINED;
                for &p in &preds[b] {
                    if idom[p] == UNDEFINED {
                        continue; // pred not yet reached
                    }
                    new_idom = if new_idom == UNDEFINED {
                        p
                    } else {
                        intersect(&idom, &rpo_num, p, new_idom)
                    };
                }
                if new_idom != UNDEFINED && idom[b] != new_idom {
                    idom[b] = new_idom;
                    changed = true;
                }
            }
        }

        let frontier = dominance_frontier(&preds, &idom);
        Dominators { idom, rpo_num, frontier }
    }

    /// Is `b` reachable from entry?
    pub fn reachable(&self, b: BlockId) -> bool {
        self.rpo_num[b] != UNDEFINED
    }

    /// Immediate dominator (entry's idom is itself).
    pub fn idom(&self, b: BlockId) -> Option<BlockId> {
        match self.idom[b] {
            UNDEFINED => None,
            d => Some(d),
        }
    }

    /// Dominance frontier of `b`.
    pub fn frontier(&self, b: BlockId) -> &[BlockId] {
        &self.frontier[b]
    }

    /// Does `a` dominate `b` (reflexive: a block dominates itself)? Walks the
    /// idom chain from `b` up to entry.
    pub fn dominates(&self, a: BlockId, b: BlockId) -> bool {
        if !self.reachable(a) || !self.reachable(b) {
            return false;
        }
        let mut x = b;
        loop {
            if x == a {
                return true;
            }
            let d = self.idom[x];
            if d == x {
                // reached entry without hitting a
                return false;
            }
            x = d;
        }
    }
}

/// Intersect two nodes per CHK: walk the deeper one up by RPO number until they
/// meet. Both must have a defined idom.
fn intersect(idom: &[BlockId], rpo_num: &[BlockId], mut a: BlockId, mut b: BlockId) -> BlockId {
    while a != b {
        while rpo_num[a] > rpo_num[b] {
            a = idom[a];
        }
        while rpo_num[b] > rpo_num[a] {
            b = idom[b];
        }
    }
    a
}

fn predecessors(succs: &[Vec<BlockId>]) -> Vec<Vec<BlockId>> {
    let mut preds = vec![Vec::new(); succs.len()];
    for (b, ss) in succs.iter().enumerate() {
        for &s in ss {
            preds[s].push(b);
        }
    }
    preds
}

/// Reverse postorder of the blocks reachable from `entry`.
fn reverse_postorder(succs: &[Vec<BlockId>], entry: BlockId) -> Vec<BlockId> {
    let n = succs.len();
    let mut visited = vec![false; n];
    let mut post = Vec::new();
    // Iterative DFS to avoid recursion depth issues on big functions.
    let mut stack: Vec<(BlockId, usize)> = vec![(entry, 0)];
    visited[entry] = true;
    while let Some(&mut (b, ref mut i)) = stack.last_mut() {
        if *i < succs[b].len() {
            let s = succs[b][*i];
            *i += 1;
            if !visited[s] {
                visited[s] = true;
                stack.push((s, 0));
            }
        } else {
            post.push(b);
            stack.pop();
        }
    }
    post.reverse();
    post
}

/// Dominance frontier per CHK: for each join (≥2 preds), walk each pred up the
/// idom chain until the block's idom, adding the block to every visited node's
/// frontier.
fn dominance_frontier(preds: &[Vec<BlockId>], idom: &[BlockId]) -> Vec<Vec<BlockId>> {
    let n = preds.len();
    let mut df: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    for b in 0..n {
        if idom[b] == UNDEFINED {
            continue; // unreachable
        }
        if preds[b].len() < 2 {
            continue;
        }
        for &p in &preds[b] {
            if idom[p] == UNDEFINED {
                continue;
            }
            let mut runner = p;
            while runner != idom[b] {
                if !df[runner].contains(&b) {
                    df[runner].push(b);
                }
                runner = idom[runner];
            }
        }
    }
    df
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_line() {
        // 0 -> 1 -> 2
        let succs = vec![vec![1], vec![2], vec![]];
        let d = Dominators::compute(&succs, 0);
        assert_eq!(d.idom(0), Some(0));
        assert_eq!(d.idom(1), Some(0));
        assert_eq!(d.idom(2), Some(1));
        assert!(d.dominates(0, 2));
        assert!(d.dominates(1, 2));
        assert!(!d.dominates(2, 1));
    }

    #[test]
    fn diamond() {
        // 0 -> {1,2} -> 3
        let succs = vec![vec![1, 2], vec![3], vec![3], vec![]];
        let d = Dominators::compute(&succs, 0);
        assert_eq!(d.idom(3), Some(0)); // merge dominated by the split, not a branch
        assert!(d.dominates(0, 3));
        assert!(!d.dominates(1, 3));
        assert!(!d.dominates(2, 3));
        // Dominance frontier of each branch arm is the merge.
        assert_eq!(d.frontier(1), &[3]);
        assert_eq!(d.frontier(2), &[3]);
        assert!(d.frontier(0).is_empty());
    }

    #[test]
    fn loop_back_edge() {
        // 0 -> 1 -> 2 -> 1 (loop), 2 -> 3 (exit)
        let succs = vec![vec![1], vec![2], vec![1, 3], vec![]];
        let d = Dominators::compute(&succs, 0);
        assert_eq!(d.idom(1), Some(0));
        assert_eq!(d.idom(2), Some(1));
        assert_eq!(d.idom(3), Some(2));
        // The loop header's frontier includes itself (back edge target).
        assert!(d.frontier(2).contains(&1));
    }

    #[test]
    fn nested_branches() {
        // 0 -> {1,4}; 1 -> {2,3}; {2,3} -> 5; {4,5} -> 6
        let succs = vec![
            vec![1, 4], // 0
            vec![2, 3], // 1
            vec![5],    // 2
            vec![5],    // 3
            vec![6],    // 4
            vec![6],    // 5
            vec![],     // 6
        ];
        let d = Dominators::compute(&succs, 0);
        assert_eq!(d.idom(5), Some(1));
        assert_eq!(d.idom(6), Some(0));
        assert!(d.dominates(1, 5));
        assert!(!d.dominates(1, 6));
        assert_eq!(d.frontier(2), &[5]);
        assert_eq!(d.frontier(3), &[5]);
    }

    #[test]
    fn unreachable_block() {
        // 0 -> 1; block 2 is unreachable.
        let succs = vec![vec![1], vec![], vec![1]];
        let d = Dominators::compute(&succs, 0);
        assert!(d.reachable(0));
        assert!(d.reachable(1));
        assert!(!d.reachable(2));
        assert_eq!(d.idom(2), None);
    }
}
