//! Run a folder of scenarios headlessly over many seeds and write the results as CSV.
//!
//! The app shows one battle; this shows the average and the spread, which is what a study
//! needs. Every metric is read back from the sim's own event logs, so there is no separate
//! measurement path to drift.
//!
//! ```text
//! cargo run -p experiments --release --bin batch -- scenarios --seeds 50
//! cargo run -p experiments --release --bin batch -- scenarios --seeds 2000 --until 900 --out out/
//! cargo run -p experiments --release --bin batch -- scenarios --only air_raid --seeds 10000
//! ```
//!
//! Writes `<out>/<scenario>.csv` (a row per seed) and `<out>/summary.csv` (a row per
//! scenario, mean and standard error). `.gitignore` covers `out/` and `*.csv`.
//!
//! Trials run in parallel, one sim per worker thread - see [`experiments::study`] for how
//! that is arranged and why it cannot change the answer.
//!
//! To compare *dials* rather than scenarios, use `sweep`: it runs the same scenario at
//! several values of one parameter over a shared seed set and reports paired differences.

use experiments::csv;
use experiments::outcome::COLUMNS;
use experiments::study::{run_study, StudyConfig};
use experiments::{flag, flag_or, has_flag};
use sim_core::scenario::{Libraries, Scenario};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Metrics worth putting on the console line; the CSV carries all of them.
const HEADLINE: [&str; 4] = [
    "blue_losses",
    "red_losses",
    "air_leakers",
    "first_detection_s",
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if has_flag(&args, "--help") {
        println!("{}", usage());
        return;
    }
    let dir = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "scenarios".to_owned());
    let cfg = StudyConfig {
        seeds: flag_or(&args, "--seeds", 20),
        until_s: flag_or(&args, "--until", 600.0),
        progress: !has_flag(&args, "--quiet"),
    };
    let out_dir = PathBuf::from(flag(&args, "--out").unwrap_or_else(|| "out".to_owned()));
    let only = flag(&args, "--only");

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

    let mut scenarios = find_scenarios(dir);
    if let Some(name) = &only {
        scenarios.retain(|(n, _)| n == name);
    }
    if scenarios.is_empty() {
        eprintln!("no scenarios found in {}", dir.display());
        std::process::exit(2);
    }

    println!(
        "=== batch: {} scenario(s) x {} seeds, {:.0} s each, {} threads -> {} ===",
        scenarios.len(),
        cfg.seeds,
        cfg.until_s,
        rayon::current_num_threads(),
        out_dir.display()
    );

    let mut summary = csv::summary_header(&["scenario"]);
    let started = Instant::now();
    let mut trials = 0_u64;

    for (name, path) in scenarios {
        let scn = match Scenario::load(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  {name}: skipped ({e})");
                continue;
            }
        };
        println!("  {name}");
        let outcomes = match run_study(&scn, &libs, cfg) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("  {name}: does not resolve ({e}); skipped");
                continue;
            }
        };
        if outcomes.is_empty() {
            continue;
        }
        trials += outcomes.len() as u64;

        let mut rows = csv::trial_header(&["seed"]);
        for (seed, o) in outcomes.iter().enumerate() {
            csv::push_trial(&mut rows, &[seed.to_string()], o);
        }
        csv::write(&out_dir.join(format!("{name}.csv")), &rows);

        let summaries = csv::push_summary(&mut summary, std::slice::from_ref(&name), &outcomes);
        for metric in HEADLINE {
            let i = COLUMNS
                .iter()
                .position(|c| *c == metric)
                .expect("headline metric exists");
            println!("      {metric:<20} {}", summaries[i]);
        }
    }

    csv::write(&out_dir.join("summary.csv"), &summary);
    let secs = started.elapsed().as_secs_f64();
    println!(
        "\n{trials} trials in {secs:.1} s ({:.0}/s)",
        trials as f64 / secs.max(1e-9)
    );
}

fn usage() -> String {
    format!(
        "batch: run a folder of scenarios over many seeds\n\
         \n\
         usage: batch [dir] [--seeds N] [--until SECONDS] [--out DIR] [--only NAME] [--quiet]\n\
         \n\
           dir       folder of scenarios and stat-block libraries (default: scenarios)\n\
           --seeds   trials per scenario, seeds 0..N (default: 20)\n\
           --until   sim seconds per trial (default: 600)\n\
           --out     output folder (default: out)\n\
           --only    just this one scenario, by bare name\n\
           --quiet   no progress line\n\
         \n\
         metrics: {}",
        COLUMNS.join(", ")
    )
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
