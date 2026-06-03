//! Fixpoint pass driver.
//!
//! Runs each pass in order, repeating rounds until a full round makes no change
//! (fixpoint) or `max_rounds` is hit. Each pass rebuilds its own semantic data,
//! so the driver only owns ordering and the convergence loop.

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

    let mut rounds = 0;
    let mut converged = false;

    while rounds < max_rounds {
        rounds += 1;
        let mut round_changed = false;

        for (i, pass) in passes.iter_mut().enumerate() {
            if pass.run(program, allocator) {
                round_changed = true;
                changes[i].1 += 1;
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
