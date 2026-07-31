//! Run a folder of scenarios headlessly over many seeds and write the results as CSV.
//!
//! The app shows one battle; this shows the average and the spread, which is what a study
//! needs. Every metric is read back from the sim's own event logs, so there is no separate
//! measurement path to drift.
//!
//! ```text
//! cargo run -p experiments --release --bin batch -- scenarios/ --seeds 50
//! cargo run -p experiments --release --bin batch -- scenarios/ --seeds 20 --until 900 --out out/
//! ```
//!
//! Writes `<out>/<scenario>.csv` (a row per seed) and `<out>/summary.csv` (a row per
//! scenario, mean and standard error). `.gitignore` covers `out/` and `*.csv`.

use sim_core::scenario::{Libraries, Scenario};
use sim_core::sim::{Side, Sim};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// What one run produced. Every field comes from the sim's own logs and final state.
#[derive(Default, Clone, Copy)]
struct Outcome {
    blue_losses: f64,
    red_losses: f64,
    blue_units_killed: f64,
    red_units_killed: f64,
    detections: f64,
    /// Sim time of the first detection either way; the run length if nothing was seen.
    first_detection_s: f64,
    fire_events: f64,
    /// Air: launched, shot down, and those that survived to release a munition.
    air_launched: f64,
    air_downed: f64,
    air_leakers: f64,
    munitions_released: f64,
    ground_casualties_from_air: f64,
}

/// Column order, shared by the per-seed and summary files so they read the same way.
const COLUMNS: [&str; 12] = [
    "blue_losses",
    "red_losses",
    "blue_units_killed",
    "red_units_killed",
    "detections",
    "first_detection_s",
    "fire_events",
    "air_launched",
    "air_downed",
    "air_leakers",
    "munitions_released",
    "ground_casualties_from_air",
];

impl Outcome {
    fn values(&self) -> [f64; 12] {
        [
            self.blue_losses,
            self.red_losses,
            self.blue_units_killed,
            self.red_units_killed,
            self.detections,
            self.first_detection_s,
            self.fire_events,
            self.air_launched,
            self.air_downed,
            self.air_leakers,
            self.munitions_released,
            self.ground_casualties_from_air,
        ]
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "scenarios".to_owned());
    let seeds: u64 = flag(&args, "--seeds")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let until_s: f64 = flag(&args, "--until")
        .and_then(|v| v.parse().ok())
        .unwrap_or(600.0);
    let out_dir = PathBuf::from(flag(&args, "--out").unwrap_or_else(|| "out".to_owned()));

    let dir = Path::new(&dir);
    let libs = match Libraries::load_dir(dir) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "could not load stat-block libraries from {}: {e}",
                dir.display()
            );
            std::process::exit(2);
        }
    };

    let scenarios = find_scenarios(dir);
    if scenarios.is_empty() {
        eprintln!("no scenarios found in {}", dir.display());
        std::process::exit(2);
    }
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("could not create {}: {e}", out_dir.display());
        std::process::exit(2);
    }

    println!(
        "=== batch: {} scenario(s) x {seeds} seeds, {until_s:.0} s each -> {} ===",
        scenarios.len(),
        out_dir.display()
    );

    let mut summary = String::from("scenario,seeds");
    for c in COLUMNS {
        let _ = write!(summary, ",{c}_mean,{c}_se");
    }
    summary.push('\n');

    for (name, path) in scenarios {
        let scn = match Scenario::load(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  {name}: skipped ({e})");
                continue;
            }
        };

        let mut rows = String::from("seed");
        for c in COLUMNS {
            let _ = write!(rows, ",{c}");
        }
        rows.push('\n');

        // Build the map **once** at the scenario's own seed, then reset per trial. That
        // keeps the terrain fixed while the dice vary, which is the question being asked
        // ("what happens on this map, on average") rather than averaging over maps too —
        // and it skips regenerating a 1000x1000 raster for every seed.
        let Ok(mut sim) = Sim::new(&scn, &libs, scn.default_seed) else {
            eprintln!("  {name}: does not resolve; skipped");
            continue;
        };
        let mut outcomes = Vec::with_capacity(seeds as usize);
        for seed in 0..seeds {
            if sim.reset_to_scenario(&scn, &libs, seed).is_err() {
                eprintln!("  {name}: does not resolve; skipped");
                break;
            }
            let outcome = run_one(&mut sim, until_s);
            let _ = write!(rows, "{seed}");
            for v in outcome.values() {
                let _ = write!(rows, ",{}", tidy(v));
            }
            rows.push('\n');
            outcomes.push(outcome);
        }
        if outcomes.is_empty() {
            continue;
        }

        let per_seed = out_dir.join(format!("{name}.csv"));
        if let Err(e) = std::fs::write(&per_seed, rows) {
            eprintln!("  {name}: could not write {}: {e}", per_seed.display());
        }

        let _ = write!(summary, "{name},{}", outcomes.len());
        print!("  {name:<16}");
        for (i, col) in COLUMNS.iter().enumerate() {
            let xs: Vec<f64> = outcomes.iter().map(|o| o.values()[i]).collect();
            let (mean, se) = mean_and_se(&xs);
            let _ = write!(summary, ",{},{}", tidy(mean), tidy(se));
            // Keep the console line readable: the headline metrics only.
            if matches!(*col, "blue_losses" | "red_losses" | "air_leakers") {
                print!("  {col} {mean:>6.2}+-{se:<5.2}");
            }
        }
        summary.push('\n');
        println!();
    }

    let summary_path = out_dir.join("summary.csv");
    match std::fs::write(&summary_path, summary) {
        Ok(()) => println!("\nwrote {}", summary_path.display()),
        Err(e) => eprintln!("could not write {}: {e}", summary_path.display()),
    }
}

/// Run one battle and read the outcome out of the sim's logs.
fn run_one(sim: &mut Sim, until_s: f64) -> Outcome {
    let air_launched = sim.air().len() as f64;
    let initial: Vec<u32> = sim.units().iter().map(|u| u.elements).collect();

    sim.run_until(until_s);

    let mut o = Outcome {
        air_launched,
        detections: (sim.events().len() + sim.air_events().len()) as f64,
        fire_events: sim.fire_events().len() as f64,
        ..Default::default()
    };

    for (i, u) in sim.units().iter().enumerate() {
        let lost = f64::from(initial[i].saturating_sub(u.elements));
        match u.side {
            Side::Blue => {
                o.blue_losses += lost;
                o.blue_units_killed += f64::from(u32::from(!u.alive()));
            }
            Side::Red => {
                o.red_losses += lost;
                o.red_units_killed += f64::from(u32::from(!u.alive()));
            }
        }
    }

    // "Leaker" = an airframe that survived to release: exactly what the strike log records.
    o.air_downed = sim.air().iter().filter(|a| !a.alive).count() as f64;
    o.munitions_released = sim.strike_events().len() as f64;
    o.ground_casualties_from_air = sim
        .strike_events()
        .iter()
        .map(|e| f64::from(e.casualties))
        .sum();
    let mut leakers: Vec<usize> = sim.strike_events().iter().map(|e| e.air).collect();
    leakers.sort_unstable();
    leakers.dedup();
    o.air_leakers = leakers.len() as f64;

    // Time to the first contact of any kind; the full run length if there was none, so
    // the column stays comparable rather than mixing in a sentinel.
    let first_ground = sim.events().first().map(|e| e.time_s);
    let first_air = sim.air_events().first().map(|e| e.time_s);
    o.first_detection_s = match (first_ground, first_air) {
        (Some(a), Some(b)) => a.min(b),
        (Some(t), None) | (None, Some(t)) => t,
        (None, None) => until_s,
    };
    o
}

/// Collapse `-0.0` to `0.0` for output.
///
/// Rust's `f64` sum folds from `-0.0`, not `0.0`, because `-0.0 + x == x` for every `x`
/// whereas `0.0 + (-0.0)` would drop the sign. So a metric summed over an empty log comes
/// out `-0.0` and prints as `-0`, which in a results file reads like a bug.
fn tidy(v: f64) -> f64 {
    if v == 0.0 {
        0.0
    } else {
        v
    }
}

/// Mean and standard error. The SE says whether a difference between two scenarios means
/// anything, so it sits beside every mean.
fn mean_and_se(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    if n == 0.0 {
        return (0.0, 0.0);
    }
    let mean = xs.iter().sum::<f64>() / n;
    if n < 2.0 {
        return (mean, 0.0);
    }
    let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (n - 1.0);
    (mean, (var / n).sqrt())
}

/// Every `*.toml` in `dir` that parses as a scenario, by bare name. Same rule the app
/// uses: a stat-block library is not a scenario because it has no `[terrain]`.
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

/// Value of a `--flag value` argument.
fn flag(args: &[String], name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).cloned()
}
