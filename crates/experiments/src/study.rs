//! Running one arm of a study: N seeds of one scenario, in parallel.
//!
//! # How the parallelism is arranged, and why that way
//!
//! Trials are independent, so they parallelise perfectly — except that each needs a
//! [`Sim`], and building one means generating the terrain, which is the expensive part
//! (1–3 s for a 1000×1000 map; the tick itself is under 15 µs).
//!
//! So the seed list is cut into exactly one chunk per worker thread, and each worker builds
//! **one** sim and resets it between trials. That gives `threads` terrain builds — paid
//! once, concurrently — rather than one per trial, and rules out the alternative
//! (`rayon::map_init`) whose init closure is called an unspecified number of times.
//!
//! Every worker builds terrain from `scenario.default_seed`, not from the trial seed, so
//! all workers get the **same** map and the study still asks "what happens on this map, on
//! average". Terrain generation is deterministic given its seed, so that is exact, not
//! approximate.
//!
//! # Determinism
//!
//! Results come back in seed order regardless of how the work was scheduled: rayon's
//! `collect` preserves order, and each trial is a fresh `reset_to_scenario` whose RNG
//! stream depends only on its seed. A parallel study therefore returns byte-identical
//! numbers to a serial one — pinned by a test at the bottom of this file, because "we
//! parallelised the study and the answer changed" is exactly the failure that would
//! otherwise be discovered by a confusing result months later.

use crate::outcome::{run_one, Outcome};
use rayon::prelude::*;
use sim_core::scenario::{Libraries, Scenario, ScenarioError};
use sim_core::sim::Sim;
use std::sync::atomic::{AtomicUsize, Ordering};

/// What one arm of a study runs.
#[derive(Clone, Copy, Debug)]
pub struct StudyConfig {
    /// Seeds `0..seeds` are run. Starting from zero keeps arms paired by construction.
    pub seeds: u64,
    /// Sim seconds per trial.
    pub until_s: f64,
    /// Print a progress line to stderr as trials complete.
    pub progress: bool,
}

impl Default for StudyConfig {
    fn default() -> Self {
        Self {
            seeds: 20,
            until_s: 600.0,
            progress: false,
        }
    }
}

/// Run `cfg.seeds` trials of `scn` in parallel, returning outcomes in seed order.
///
/// # Errors
/// [`ScenarioError`] if the scenario does not resolve against the libraries — reported
/// once rather than once per trial.
pub fn run_study(
    scn: &Scenario,
    libs: &Libraries,
    cfg: StudyConfig,
) -> Result<Vec<Outcome>, ScenarioError> {
    // Resolve once up front so a bad scenario fails here with one clear error, rather than
    // inside a worker where it would be a panic or N identical messages.
    Sim::new(scn, libs, scn.default_seed)?;

    let seeds: Vec<u64> = (0..cfg.seeds).collect();
    if seeds.is_empty() {
        return Ok(Vec::new());
    }
    let threads = rayon::current_num_threads().max(1);
    let chunk = seeds.len().div_ceil(threads);
    let done = AtomicUsize::new(0);
    let total = seeds.len();

    let outcomes: Vec<Outcome> = seeds
        .par_chunks(chunk)
        .flat_map_iter(|chunk| {
            // One sim per chunk: terrain built once here, then reset per trial.
            let mut sim = Sim::new(scn, libs, scn.default_seed)
                .expect("scenario resolved above, so it resolves here");
            let mut out = Vec::with_capacity(chunk.len());
            for &seed in chunk {
                sim.reset_to_scenario(scn, libs, seed)
                    .expect("scenario resolved above, so it resolves here");
                out.push(run_one(&mut sim, cfg.until_s));
                if cfg.progress {
                    report(done.fetch_add(1, Ordering::Relaxed) + 1, total);
                }
            }
            out
        })
        .collect();

    if cfg.progress && std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        eprintln!();
    }
    Ok(outcomes)
}

/// Evaluate **many** scenarios over one shared seed set, building terrain once for all of
/// them.
///
/// A sensitivity design is thousands of scenarios that differ only in dials.
/// [`run_study`] builds terrain once per worker *per call*, which is right for a handful of
/// arms and catastrophic for a design: 1,600 design points on a 1000x1000 map is ~19,000
/// terrain builds and the trials themselves become a rounding error. Measured on
/// `air_raid`, that was the difference between a study finishing and a study being
/// abandoned.
///
/// So terrain is built once per worker from `base`, and every design point is placed into it
/// with [`Sim::reset_to_scenario`]. That is exactly the "fix the map, vary the dice" rule
/// the rest of this module follows, applied one level further out — and it means a design
/// **must not** vary a terrain dial, because the map it would ask for is not the map it
/// would get. `sensitivity` refuses those paths for that reason.
///
/// Returns one outcome vector per point, in point order.
///
/// # Errors
/// [`ScenarioError`] if `base` or any point fails to resolve, reported once rather than
/// once per trial.
pub fn run_design(
    base: &Scenario,
    base_libs: &Libraries,
    points: &[(Scenario, Libraries)],
    cfg: StudyConfig,
) -> Result<Vec<Vec<Outcome>>, ScenarioError> {
    // Resolve everything up front, so a bad point fails here with one clear message rather
    // than inside a worker.
    Sim::new(base, base_libs, base.default_seed)?;
    for (scn, libs) in points {
        Sim::new(scn, libs, scn.default_seed)?;
    }
    if points.is_empty() || cfg.seeds == 0 {
        return Ok(vec![Vec::new(); points.len()]);
    }

    let indices: Vec<usize> = (0..points.len()).collect();
    let threads = rayon::current_num_threads().max(1);
    let chunk = indices.len().div_ceil(threads);
    let done = AtomicUsize::new(0);
    let total = points.len();

    let mut results: Vec<(usize, Vec<Outcome>)> = indices
        .par_chunks(chunk)
        .flat_map_iter(|chunk| {
            // The one expensive thing, once per worker for the whole design.
            let mut sim = Sim::new(base, base_libs, base.default_seed)
                .expect("base resolved above, so it resolves here");
            let mut out = Vec::with_capacity(chunk.len());
            for &i in chunk {
                let (scn, libs) = &points[i];
                let mut trials = Vec::with_capacity(cfg.seeds as usize);
                for seed in 0..cfg.seeds {
                    sim.reset_to_scenario(scn, libs, seed)
                        .expect("point resolved above, so it resolves here");
                    trials.push(run_one(&mut sim, cfg.until_s));
                }
                if cfg.progress {
                    report(done.fetch_add(1, Ordering::Relaxed) + 1, total);
                }
                out.push((i, trials));
            }
            out
        })
        .collect();

    if cfg.progress && std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        eprintln!();
    }
    // Chunked collection preserves order, but sorting says so rather than relying on it.
    results.sort_by_key(|(i, _)| *i);
    Ok(results.into_iter().map(|(_, o)| o).collect())
}

/// Run the same arm serially. Kept because it is the reference the parallel path is
/// checked against, and because a profiler reads a single-threaded run far more easily.
///
/// # Errors
/// As [`run_study`].
pub fn run_study_serial(
    scn: &Scenario,
    libs: &Libraries,
    cfg: StudyConfig,
) -> Result<Vec<Outcome>, ScenarioError> {
    let mut sim = Sim::new(scn, libs, scn.default_seed)?;
    let mut out = Vec::with_capacity(cfg.seeds as usize);
    for seed in 0..cfg.seeds {
        sim.reset_to_scenario(scn, libs, seed)?;
        out.push(run_one(&mut sim, cfg.until_s));
    }
    Ok(out)
}

/// Pull one metric out of a set of outcomes, for [`crate::stats::paired`].
#[must_use]
pub fn column(outcomes: &[Outcome], metric: usize) -> Vec<f64> {
    outcomes.iter().map(|o| o.values()[metric]).collect()
}

/// A carriage-returned progress line, thinned so printing never dominates the run.
///
/// Goes to **stderr**, so `... > results.txt` still captures only results — and only when
/// stderr is a terminal, because a carriage return in a captured log is just 100 copies of
/// the same line with no carriage to return.
fn report(done: usize, total: usize) {
    use std::io::IsTerminal;
    if !std::io::stderr().is_terminal() {
        return;
    }
    let step = (total / 100).max(1);
    if done.is_multiple_of(step) || done == total {
        eprint!(
            "\r  {done}/{total} ({:.0}%)          ",
            100.0 * done as f64 / total as f64
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture() -> Option<(Scenario, Libraries)> {
        // Tests run from the crate root; the scenarios live at the workspace root.
        let dir = Path::new("../../scenarios");
        let libs = Libraries::load_dir(dir).ok()?;
        let scn = Scenario::load(&dir.join("flat_range.toml")).ok()?;
        Some((scn, libs))
    }

    /// The property the whole harness rests on: how the work was scheduled cannot change
    /// the answer. If this ever fails, some state is leaking between trials.
    #[test]
    fn parallel_matches_serial_exactly() {
        let Some((scn, libs)) = fixture() else {
            return; // scenarios/ not present (e.g. packaged build) — nothing to check
        };
        let cfg = StudyConfig {
            seeds: 24,
            until_s: 180.0,
            progress: false,
        };
        let par = run_study(&scn, &libs, cfg).expect("fixture resolves");
        let ser = run_study_serial(&scn, &libs, cfg).expect("fixture resolves");
        assert_eq!(par, ser, "parallel scheduling changed the result");
    }

    /// A trial must not inherit anything from the one before it on the same worker.
    /// Re-running one seed alone has to give what it gave inside a batch.
    #[test]
    fn a_trial_is_independent_of_what_ran_before_it() {
        let Some((scn, libs)) = fixture() else {
            return;
        };
        let cfg = StudyConfig {
            seeds: 8,
            until_s: 180.0,
            progress: false,
        };
        let batch = run_study_serial(&scn, &libs, cfg).expect("fixture resolves");
        let mut sim = Sim::new(&scn, &libs, scn.default_seed).expect("fixture resolves");
        for (seed, expected) in batch.iter().enumerate().rev() {
            sim.reset_to_scenario(&scn, &libs, seed as u64)
                .expect("fixture resolves");
            assert_eq!(&run_one(&mut sim, cfg.until_s), expected, "seed {seed}");
        }
    }
}
