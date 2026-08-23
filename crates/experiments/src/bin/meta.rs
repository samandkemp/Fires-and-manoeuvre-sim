//! Meta-analysis: sweep **every** dial and report which ones the answer actually rests on.
//!
//! ```text
//! cargo run -p experiments --release --bin meta -- <scenario> [options]
//! cargo run -p experiments --release --bin meta -- air_raid --metric air_leakers
//! cargo run -p experiments --release --bin meta -- air_raid --only sensors,air_defence
//! cargo run -p experiments --release --bin meta -- air_raid --emit studies/generated.toml
//! ```
//!
//! # Why screening first
//!
//! A full variance decomposition over every dial is not affordable and would not be worth it
//! if it were. Sobol costs `n(k+2)` design points for `k` dials; at the sixty-odd dials a
//! shipped library set produces, and twenty seeds each, that is millions of trials.
//!
//! Morris elementary effects cost `(k+1) x trajectories` instead - a few thousand - and
//! answer the question that actually comes first: **which dials can be ignored?** That is the
//! standard two-stage practice in sensitivity analysis, and this command is the first stage
//! over everything, with `sensitivity` remaining the second stage over the survivors.
//!
//! So the output is a ranking, not a decomposition. It says what to study next.
//!
//! # What is left out, and why
//!
//! Flags and named choices. Morris perturbs a continuous input and reads the gradient; a
//! boolean has no gradient and `optimal` is not halfway between `greedy` and `independent`.
//! Those belong in a factorial design, and the report says so rather than quietly dropping
//! them.

use experiments::dials::{self, Dial, Range};
use experiments::outcome::COLUMNS;
use experiments::patch::{self, scenario_with_overrides, Override};
use experiments::sensitivity::{morris_design, morris_indices, Dial as SweepDial, Point};
use experiments::study::{run_design, StudyConfig};
use experiments::{flag, flag_or};
use sim_core::scenario::{Libraries, Scenario};
use std::path::Path;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(name) = args.first().filter(|a| !a.starts_with("--")) else {
        eprintln!("usage: meta <scenario> [--metric M] [--only prefix,...] [--trajectories N] [--seeds N] [--until S] [--emit FILE]");
        std::process::exit(2);
    };

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = root.join("scenarios");
    let libs = match Libraries::load_dir(&dir) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot load libraries: {e}");
            std::process::exit(2);
        }
    };
    let path = dir.join(format!("{name}.toml"));
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read {}: {e}", path.display());
            std::process::exit(2);
        }
    };
    let Ok(scn) = Scenario::from_toml_str(&text) else {
        eprintln!("{name}: does not parse");
        std::process::exit(2);
    };

    let metric_name = flag(&args, "--metric").unwrap_or_else(|| "red_cleared_s".to_owned());
    let Some(metric) = COLUMNS.iter().position(|c| *c == metric_name) else {
        eprintln!(
            "unknown metric '{metric_name}'; one of: {}",
            COLUMNS.join(", ")
        );
        std::process::exit(2);
    };
    let trajectories: usize = flag_or(&args, "--trajectories", 12);
    let seeds: u64 = flag_or(&args, "--seeds", 8);
    let until_s: f64 = flag_or(&args, "--until", 300.0);
    let levels: usize = flag_or(&args, "--levels", 4);

    // `--only` filters by path prefix, so a run can be confined to one subsystem. Without it
    // every dial is in, which is the point of the command.
    let only: Vec<String> = flag(&args, "--only")
        .map(|s| s.split(',').map(|p| p.trim().to_owned()).collect())
        .unwrap_or_default();

    let all = dials::enumerate(&libs);
    let (continuous, categorical): (Vec<Dial>, Vec<Dial>) = all
        .into_iter()
        .filter(|d| only.is_empty() || only.iter().any(|p| d.path.starts_with(p)))
        .partition(|d| d.range.is_continuous());

    if continuous.is_empty() {
        eprintln!("no continuous dials matched; nothing to screen");
        std::process::exit(2);
    }

    // Resolve each dial's absolute bounds. A relative range needs the value the library
    // actually gives it, which is why this cannot be a constant table.
    let mut names = Vec::new();
    let mut bounds = Vec::new();
    let mut integral = Vec::new();
    let mut unresolved = Vec::new();
    for d in &continuous {
        match resolve(d, &text, &dir) {
            Some((lo, hi)) if hi > lo => {
                names.push(d.path.clone());
                bounds.push((lo, hi));
                integral.push(matches!(d.range, Range::Integer(..)));
            }
            // A dial whose current value is zero has no relative range - twice nothing is
            // nothing - and one the scenario does not set cannot be read. Reported rather
            // than silently dropped, because "swept everything" has to mean something.
            _ => unresolved.push(d.path.clone()),
        }
    }

    if let Some(out) = flag(&args, "--emit") {
        emit_study(&out, name, &metric_name, &names, &bounds, &continuous);
        return;
    }

    let sweep_dials: Vec<SweepDial> = names
        .iter()
        .zip(&bounds)
        .map(|(path, &(lo, hi))| SweepDial {
            path: path.clone(),
            lo,
            hi,
        })
        .collect();
    let design: Vec<Point> = morris_design(names.len(), trajectories, levels, 0xB1A5);

    println!("Meta-analysis of `{name}`, metric `{metric_name}`");
    println!(
        "  {} continuous dials screened, {} trajectories, {seeds} seeds each = {} trials",
        names.len(),
        trajectories,
        design.len() as u64 * seeds
    );
    if !categorical.is_empty() {
        println!(
            "  {} categorical dial(s) excluded - a gradient is undefined for them; use `factorial`:",
            categorical.len()
        );
        for d in &categorical {
            println!("      {}", d.path);
        }
    }
    if !unresolved.is_empty() {
        println!(
            "  {} dial(s) could not be given a range (currently zero, or not set by this scenario):",
            unresolved.len()
        );
        for p in unresolved.iter().take(8) {
            println!("      {p}");
        }
        if unresolved.len() > 8 {
            println!("      ... and {} more", unresolved.len() - 8);
        }
    }
    println!();

    let started = Instant::now();

    // Build every design point up front and hand the lot to `run_design`, which builds
    // terrain once per worker rather than once per point. Evaluating points one at a time
    // rebuilds a 1000x1000 map thousands of times and dominates everything else.
    let built: Vec<(Scenario, Libraries)> = design
        .iter()
        .map(|pt| {
            let overrides: Vec<Override> = sweep_dials
                .iter()
                .zip(pt)
                .zip(&integral)
                .map(|((d, &u), &is_int)| Override {
                    path: d.path.clone(),
                    // A whole-number dial must be written as one. `allocation_horizon = 1.0`
                    // is refused by the schema, and rightly: the strictness that catches a
                    // misspelt dial catches a mistyped one too.
                    value: if is_int {
                        toml::Value::Integer(d.at(u).round() as i64)
                    } else {
                        toml::Value::Float(d.at(u))
                    },
                })
                .collect();
            let (lib_over, scn_over) = patch::split(&overrides);
            let libs = patch::libraries_with_overrides(&dir, &lib_over).unwrap_or_else(|e| {
                eprintln!("design point rejected: {e}");
                std::process::exit(2);
            });
            let s = scenario_with_overrides(&text, &scn_over).unwrap_or_else(|e| {
                eprintln!("design point rejected: {e}");
                std::process::exit(2);
            });
            (s, libs)
        })
        .collect();

    let cfg = StudyConfig {
        seeds,
        until_s,
        progress: true,
    };
    let per_point = match run_design(&scn, &libs, &built, cfg) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{name}: design does not resolve ({e})");
            std::process::exit(2);
        }
    };
    let values: Vec<f64> = per_point
        .iter()
        .map(|trials| {
            let n = trials.len().max(1) as f64;
            trials.iter().map(|o| o.values()[metric]).sum::<f64>() / n
        })
        .collect();
    let effects = morris_indices(&design, &values, names.len(), trajectories, levels);

    println!("--- what the answer rests on (Morris elementary effects) ---");
    println!("  {:<52} {:>10} {:>10}", "dial", "mu*", "sigma");
    let mut ranked: Vec<(usize, f64, f64)> = effects
        .iter()
        .enumerate()
        .map(|(i, e)| (i, e.mu_star, e.sigma))
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (i, mu, sigma) in ranked.iter().take(20) {
        println!("  {:<52} {mu:>10.4} {sigma:>10.4}", names[*i]);
    }
    if ranked.len() > 20 {
        println!("  ... and {} more, all smaller", ranked.len() - 20);
    }

    let total: f64 = ranked.iter().map(|r| r.1).sum();
    let carrying: usize = if total > 0.0 {
        let mut acc = 0.0;
        ranked
            .iter()
            .take_while(|r| {
                let keep = acc < 0.9 * total;
                acc += r.1;
                keep
            })
            .count()
    } else {
        0
    };
    println!(
        "\n  {carrying} of {} dials carry 90% of the total effect.",
        names.len()
    );
    println!(
        "  A high sigma against its own mu* means the dial's effect depends on where the\n  \
         others are - an interaction, which Morris can see but not size. Take the survivors\n  \
         to `sensitivity` for that."
    );
    println!("\nscreened in {:.1?}", started.elapsed());
}

/// A dial's absolute bounds, resolving a relative range against the value in force.
fn resolve(dial: &Dial, scenario_text: &str, dir: &Path) -> Option<(f64, f64)> {
    match dial.range {
        Range::Absolute(lo, hi) => Some((lo, hi)),
        Range::Integer(lo, hi) => Some((lo as f64, hi as f64)),
        Range::Relative(lo, hi) => {
            let current = experiments::patch::current_value(&dial.path, scenario_text, dir)?;
            (current != 0.0).then_some((current * lo, current * hi))
        }
        Range::Flag | Range::Choice(_) => None,
    }
}

/// Write the screened dial space as a `studies/*.toml` file, so a promising subset can be
/// handed straight to `sensitivity` without retyping it.
fn emit_study(
    out: &str,
    scenario: &str,
    metric: &str,
    names: &[String],
    bounds: &[(f64, f64)],
    dials: &[Dial],
) {
    let mut s = String::new();
    s.push_str(&format!(
        "# Generated by `meta --emit`. Every continuous dial {scenario} exposes.\n\
         #\n\
         # This is a starting point, not a study: a full Sobol decomposition over this many\n\
         # dials is not affordable. Screen with `meta`, delete the rows that do not matter,\n\
         # then run `sensitivity` on what is left.\n\n\
         scenario = \"{scenario}\"\n\
         metric = \"{metric}\"\n\n\
         trajectories = 12\n\
         levels = 4\n\
         sobol_n = 128\n\n\
         [ranges]\n"
    ));
    for ((name, (lo, hi)), dial) in names.iter().zip(bounds).zip(dials) {
        s.push_str(&format!(
            "\n# {} ({}){}\n\"{name}\" = [{lo}, {hi}]\n",
            dial.doc,
            dial.section,
            if dial.unit.is_empty() {
                String::new()
            } else {
                format!(", {}", dial.unit)
            }
        ));
    }
    match std::fs::write(out, s) {
        Ok(()) => println!("wrote {out} with {} dials", names.len()),
        Err(e) => {
            eprintln!("cannot write {out}: {e}");
            std::process::exit(2);
        }
    }
}
