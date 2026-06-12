//! Fixpoint pass driver.
//!
//! Runs each pass in order, repeating rounds until a full round makes no change
//! (fixpoint) or `max_rounds` is hit. Each pass rebuilds its own semantic data,
//! so the driver only owns ordering and the convergence loop.
//!
//! ## Skipping converged passes
//!
//! Every pass is a deterministic function of the AST: `run` returns `false` iff it
//! made no change, and it builds all its own analysis fresh each call (no
//! cross-run state). So once a pass reports "no change" for a given AST, it will
//! keep reporting "no change" until *some other* pass mutates the tree. The driver
//! exploits this with a monotonic mutation `version`: a pass is re-run only when
//! the version has advanced since it last returned `false`. This turns the wasted
//! tail rounds — where most passes re-run, each rebuilding `Scoping` via a full
//! `SemanticBuilder` traversal, only to report no change — into cheap skips, while
//! producing exactly the same fixpoint (every pass still evaluates the final,
//! stable AST version once). Sound under the same pass contract the convergence
//! check already relies on.

use oxc_allocator::Allocator;
use oxc_ast::ast::Program;

use crate::pass::Pass;

pub struct DriverStats {
    pub rounds: usize,
    pub converged: bool,
    pub changes: Vec<(String, usize)>, // (pass name, rounds in which it reported a change)
}

pub fn run_fixpoint<'a>(
    program: &mut Program<'a>,
    allocator: &'a Allocator,
    passes: &mut [Box<dyn Pass>],
    max_rounds: usize,
) -> DriverStats {
    let mut changes: Vec<(String, usize)> =
        passes.iter().map(|p| (p.name().to_string(), 0)).collect();

    // Monotonic count of AST mutations. A pass that returned `false` at version V
    // need not re-run until `version` advances past V (the tree it already saw to
    // be a fixpoint has not changed). `None` = "must run" (never run, or it last
    // changed the tree, so its no-change verdict isn't yet established).
    let mut version: u64 = 0;
    let mut last_clean: Vec<Option<u64>> = vec![None; passes.len()];

    let mut rounds = 0;
    let mut converged = false;

    while rounds < max_rounds {
        rounds += 1;
        let mut round_changed = false;

        for (i, pass) in passes.iter_mut().enumerate() {
            // Already proved a no-op on the current tree → skip until it mutates.
            if last_clean[i] == Some(version) {
                continue;
            }
            if pass.run(program, allocator) {
                // The tree changed: bump the version (invalidating every other
                // pass's clean mark) and leave this pass eligible to run again.
                version += 1;
                last_clean[i] = None;
                round_changed = true;
                changes[i].1 += 1;
            } else {
                // No change at this version; don't re-run until something mutates.
                last_clean[i] = Some(version);
            }
        }

        if !round_changed {
            converged = true;
            break;
        }
    }

    DriverStats {
        rounds,
        converged,
        changes,
    }
}
