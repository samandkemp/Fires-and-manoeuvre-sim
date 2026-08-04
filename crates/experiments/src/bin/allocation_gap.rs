//! What does coordinating fire actually buy? Model: `docs/DESIGN.md` §10.2.
//!
//! Runs every shipped scenario three ways over the same seeds — the optimal (Hungarian)
//! allocation, the greedy heuristic, and `independent`, which reproduces the
//! pre-Phase-10 rule where each shooter chose the nearest enemy for itself.
//!
//! The point is to keep the optimal solver honest. "Optimal is better" is an assumption
//! until it is a number, and if the number turns out to be zero on realistic scenarios,
//! that is worth knowing too — greedy is `O(nm log nm)` and Hungarian is `O(n²m)`.
//!
//! Run: `cargo run -p experiments --release --bin allocation_gap`

use sim_core::scenario::{AllocationChoice, Libraries, Scenario};
use sim_core::sim::{Side, Sim};
use std::path::{Path, PathBuf};

/// How long a battle is given before it is called off, seconds.
const HORIZON_S: f64 = 600.0;

/// What one run produced, from the sim's own logs.
#[derive(Default, Clone, Copy)]
struct Outcome {
    enemy_losses: f64,
    own_losses: f64,
    /// Sim time at which the enemy was wiped out, or [`HORIZON_S`] if it survived.
    ///
    /// The headline metric. Total losses saturate — given long enough, wasteful fire
    /// destroys the same force as efficient fire — so *how fast* is the question, not
    /// *how many*.
    finish_s: f64,
    /// Mean distinct targets engaged per firing epoch: concentration versus spread.
    spread: f64,
}

fn main() {
    let seeds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
    let libs = match Libraries::load_dir(&dir) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("could not load libraries: {e}");
            std::process::exit(2);
        }
    };

    println!("=== allocation gap: {seeds} seeds per scenario ===");
    println!(
        "finish = time to destroy Red (lower is better); spread = targets engaged per epoch
"
    );

    for (name, path) in find_scenarios(&dir) {
        let Ok(scn) = Scenario::load(&path) else {
            continue;
        };
        // Skip scenarios where nobody shoots — the comparison would be all zeros.
        let Ok(probe) = Sim::new(&scn, &libs, scn.default_seed) else {
            continue;
        };
        if probe.units().iter().all(|u| u.weapon.is_none()) {
            continue;
        }

        println!("{name}");
        let mut baseline: Option<f64> = None;
        for choice in [
            AllocationChoice::Independent,
            AllocationChoice::Greedy,
            AllocationChoice::Optimal,
        ] {
            let mut tuned = scn.clone();
            tuned.sim.allocation = choice;
            // Terrain once, dice per seed — the same separation the batch runner makes,
            // so this measures the allocation and not the map.
            let Ok(mut sim) = Sim::new(&tuned, &libs, tuned.default_seed) else {
                continue;
            };
            let mut runs = Vec::with_capacity(seeds as usize);
            for seed in 0..seeds {
                if sim.reset_to_scenario(&tuned, &libs, seed).is_err() {
                    break;
                }
                runs.push(run_one(&mut sim));
            }
            if runs.is_empty() {
                continue;
            }
            let n = runs.len() as f64;
            let mean = |f: fn(&Outcome) -> f64| runs.iter().map(f).sum::<f64>() / n;
            let (red, blue, spread, finish) = (
                mean(|o| o.enemy_losses),
                mean(|o| o.own_losses),
                mean(|o| o.spread),
                mean(|o| o.finish_s),
            );
            // Everything is measured against the old rule, so the gap is the headline.
            // Faster is better, hence the sign.
            let delta = match baseline {
                None => {
                    baseline = Some(finish);
                    String::from("  (baseline)")
                }
                Some(b) if b > 0.0 => format!("  {:+.1}% time", (finish - b) / b * 100.0),
                Some(_) => String::new(),
            };
            println!(
                "  {:<12} finish {finish:>6.1} s   red {red:>6.2}   blue {blue:>5.2}   \
                 spread {spread:>4.2}{delta}",
                format!("{choice:?}").to_lowercase()
            );
        }
        println!();
    }
}

/// Live Red ground elements.
fn red_elements(sim: &Sim) -> u32 {
    sim.units()
        .iter()
        .filter(|u| u.side == Side::Red)
        .map(|u| u.elements)
        .sum()
}

/// One battle, reported from Blue's point of view.
fn run_one(sim: &mut Sim) -> Outcome {
    let initial: Vec<u32> = sim.units().iter().map(|u| u.elements).collect();
    // A scenario with no Red ground force (air_raid) would otherwise read as "finished
    // instantly" — there was never anything to destroy.
    let had_enemy = red_elements(sim) > 0;

    // Step in epochs so the moment Red is finished can be recorded, rather than only
    // whether it happened by the horizon.
    let mut finish_s = HORIZON_S;
    while sim.time_s() < HORIZON_S {
        sim.run_until(sim.time_s() + 10.0);
        if had_enemy && red_elements(sim) == 0 {
            finish_s = sim.time_s();
            break;
        }
    }

    let mut o = Outcome {
        finish_s,
        ..Default::default()
    };
    for (i, u) in sim.units().iter().enumerate() {
        let lost = f64::from(initial[i].saturating_sub(u.elements));
        match u.side {
            Side::Blue => o.own_losses += lost,
            Side::Red => o.enemy_losses += lost,
        }
    }

    // How many distinct targets were being engaged in a typical firing epoch — the
    // concentration-versus-spread question allocation exists to answer.
    let mut epochs: std::collections::BTreeMap<u64, std::collections::BTreeSet<usize>> =
        std::collections::BTreeMap::new();
    for e in sim.fire_events() {
        epochs.entry(e.time_s as u64).or_default().insert(e.target);
    }
    if !epochs.is_empty() {
        o.spread = epochs.values().map(|t| t.len() as f64).sum::<f64>() / epochs.len() as f64;
    }
    o
}

/// Every `*.toml` in `dir` that parses as a scenario, by bare name.
fn find_scenarios(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .filter(|p| Scenario::load(p).is_ok())
        .filter_map(|p| {
            p.file_stem()
                .map(|s| (s.to_string_lossy().into_owned(), p.clone()))
        })
        .collect();
    found.sort();
    found
}
