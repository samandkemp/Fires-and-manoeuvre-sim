//! Sweep one scenario dial across a set of values and report what it changed.
//!
//! This is the general form of every bespoke sweep in this crate. Any field reachable by a
//! dotted path in a scenario file is sweepable, because the override is applied to the TOML
//! before it is parsed ([`experiments::patch`]) — so a dial added next month is sweepable
//! without touching this binary.
//!
//! ```text
//! # Does holding a track longer help, and by how much?
//! sweep air_raid --param sim.track_hold_s --values 10,20,45,90 --seeds 500
//!
//! # A log-spaced grid instead of a list
//! sweep default --param sim.p_suppress --from 0.05 --to 0.8 --steps 8 --seeds 2000
//!
//! # Which allocation solver, holding everything else fixed? (string-valued dial)
//! sweep fire_allocation --param sim.allocation \
//!       --values independent,greedy,optimal --seeds 500 --metric first_detection_s
//!
//! # Sweep one dial with another pinned
//! sweep ad_c2 --param sim.max_batteries_per_air_target --values 1,2,3 \
//!       --set sim.allocation=greedy --seeds 1000
//! ```
//!
//! # The comparison is paired, by construction
//!
//! Every arm runs seeds `0..N` on the same map, so arm *k* and arm 0 are matched trial for
//! trial and the difference is taken seed by seed. The variance the two arms share — the
//! map, most of the luck — cancels, which is usually most of it. The report prints that
//! paired difference against the **first** arm, with its standard error and t statistic,
//! and says in as many words whether to believe it. See [`experiments::stats`] for why
//! this crate offers no unpaired comparison.

use experiments::outcome::{Outcome, COLUMNS};
use experiments::patch::{self, scenario_with_overrides, Override};
use experiments::stats::paired;
use experiments::study::{column, run_study, StudyConfig};
use experiments::{csv, flag, flag_or, flags, has_flag};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || has_flag(&args, "--help") {
        println!("{}", usage());
        return;
    }

    let scenario_arg = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "default".to_owned());
    let dir = PathBuf::from(flag(&args, "--dir").unwrap_or_else(|| "scenarios".to_owned()));
    let out_dir = PathBuf::from(flag(&args, "--out").unwrap_or_else(|| "out".to_owned()));
    let Some(param) = flag(&args, "--param") else {
        eprintln!("--param is required\n\n{}", usage());
        std::process::exit(2);
    };
    let cfg = StudyConfig {
        seeds: flag_or(&args, "--seeds", 200),
        until_s: flag_or(&args, "--until", 600.0),
        progress: !has_flag(&args, "--quiet"),
    };
    let metric_name = flag(&args, "--metric").unwrap_or_else(|| "red_losses".to_owned());
    let Some(metric) = Outcome::column(&metric_name) else {
        eprintln!(
            "unknown metric '{metric_name}'. Known: {}",
            COLUMNS.join(", ")
        );
        std::process::exit(2);
    };

    let values = match sweep_values(&args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}\n\n{}", usage());
            std::process::exit(2);
        }
    };
    // `--set path=value` overrides are applied to *every* arm, so a sweep can hold one
    // dial fixed away from the scenario's own setting while varying another.
    let fixed: Vec<Override> = match flags(&args, "--set")
        .iter()
        .map(|s| Override::parse(s))
        .collect()
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!("bad --set: {e}");
            std::process::exit(2);
        }
    };

    let path = resolve_scenario(&dir, &scenario_arg);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("could not read {}: {e}", path.display());
            std::process::exit(2);
        }
    };
    let name = path.file_stem().map_or_else(
        || scenario_arg.clone(),
        |s| s.to_string_lossy().into_owned(),
    );

    println!(
        "=== sweep: {name} \u{b7} {param} over {} value(s) \u{d7} {} seeds, {:.0} s each, {} threads ===",
        values.len(),
        cfg.seeds,
        cfg.until_s,
        rayon::current_num_threads()
    );
    if !fixed.is_empty() {
        println!(
            "    holding: {}",
            fixed
                .iter()
                .map(|o| format!("{}={}", o.path, o.value))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }

    let mut trials = csv::trial_header(&["value", "seed"]);
    let mut summary = csv::summary_header(&["param", "value"]);
    let mut arms: Vec<(String, Vec<Outcome>)> = Vec::with_capacity(values.len());
    let started = Instant::now();

    for value in &values {
        let mut overrides = fixed.clone();
        overrides.push(Override {
            path: param.clone(),
            value: experiments::patch::parse_value(value),
        });
        // A path naming a stat-block library patches that file; everything else patches
        // the scenario. Both go through the loaders a file on disk would.
        let (lib_overrides, scenario_overrides) = patch::split(&overrides);
        let libs = match patch::libraries_with_overrides(&dir, &lib_overrides) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("  {param}={value}: {e}");
                std::process::exit(2);
            }
        };
        let scn = match scenario_with_overrides(&text, &scenario_overrides) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  {param}={value}: {e}");
                std::process::exit(2);
            }
        };
        println!("  {param} = {value}");
        let outcomes = match run_study(&scn, &libs, cfg) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("  {param}={value}: does not resolve ({e})");
                std::process::exit(2);
            }
        };
        for (seed, o) in outcomes.iter().enumerate() {
            csv::push_trial(&mut trials, &[value.clone(), seed.to_string()], o);
        }
        let summaries = csv::push_summary(&mut summary, &[param.clone(), value.clone()], &outcomes);
        println!("      {metric_name:<20} {}", summaries[metric]);
        arms.push((value.clone(), outcomes));
    }

    report(&param, &metric_name, metric, &arms);

    let stem = format!("{name}_{}", param.replace('.', "_"));
    csv::write(&out_dir.join(format!("{stem}.csv")), &trials);
    csv::write(&out_dir.join(format!("{stem}_summary.csv")), &summary);
    let secs = started.elapsed().as_secs_f64();
    let n: usize = arms.iter().map(|(_, o)| o.len()).sum();
    println!(
        "\n{n} trials in {secs:.1} s ({:.0}/s)",
        n as f64 / secs.max(1e-9)
    );
}

/// Print each arm against the first, paired seed by seed.
fn report(param: &str, metric_name: &str, metric: usize, arms: &[(String, Vec<Outcome>)]) {
    let Some((base_value, base)) = arms.first() else {
        return;
    };
    println!("\n--- {metric_name}, paired against {param} = {base_value} ---");
    let base_col = column(base, metric);
    for (value, outcomes) in arms {
        let col = column(outcomes, metric);
        if value == base_value {
            println!("  {param:>28} = {value:<10}  baseline {:.3}", mean(&col));
            continue;
        }
        let p = paired(&col, &base_col);
        println!("  {param:>28} = {value:<10}  {}", p.report());
    }
    // A sweep whose arms are all tied is a sweep of a dial nothing reads — worth saying
    // out loud, because the natural reading of "no significant effect" is the opposite.
    if arms.len() > 1 {
        let all_tied = arms[1..].iter().all(|(_, o)| {
            let p = paired(&column(o, metric), &base_col);
            p.ties == p.n
        });
        if all_tied {
            println!(
                "\n  NB every arm is identical to the baseline on every seed. Either this\n  \
                 dial does not reach '{metric_name}' in this scenario, or the scenario has\n  \
                 nothing for it to act on."
            );
        }
    }
}

fn mean(xs: &[f64]) -> f64 {
    experiments::mean_and_se(xs).0
}

/// The values to sweep: an explicit `--values a,b,c`, or a `--from/--to/--steps` grid.
fn sweep_values(args: &[String]) -> Result<Vec<String>, String> {
    if let Some(list) = flag(args, "--values") {
        let values: Vec<String> = list
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        return if values.is_empty() {
            Err("--values was empty".to_owned())
        } else {
            Ok(values)
        };
    }
    let (Some(from), Some(to)) = (flag(args, "--from"), flag(args, "--to")) else {
        return Err("give either --values a,b,c or --from X --to Y [--steps N]".to_owned());
    };
    let from: f64 = from.parse().map_err(|_| "--from is not a number")?;
    let to: f64 = to.parse().map_err(|_| "--to is not a number")?;
    let steps: usize = flag_or(args, "--steps", 5_usize).max(2);
    // Inclusive of both ends: a sweep from 10 to 90 that stopped at 74 would be a trap.
    Ok((0..steps)
        .map(|k| {
            let t = k as f64 / (steps - 1) as f64;
            format_value(from + t * (to - from))
        })
        .collect())
}

/// Format a swept value so integral ones stay integers — `2`, not `2.0`, because a `u32`
/// dial refuses a TOML float (see [`experiments::patch::parse_value`]).
fn format_value(x: f64) -> String {
    if (x - x.round()).abs() < 1e-9 {
        format!("{}", x.round() as i64)
    } else {
        format!("{x:.6}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

/// A bare name resolves inside `dir`; anything with a separator or `.toml` is a path.
fn resolve_scenario(dir: &Path, arg: &str) -> PathBuf {
    if arg.ends_with(".toml") || arg.contains('/') || arg.contains('\\') {
        PathBuf::from(arg)
    } else {
        dir.join(format!("{arg}.toml"))
    }
}

fn usage() -> String {
    format!(
        "sweep: vary one scenario dial and report the paired difference it makes\n\
         \n\
         usage: sweep <scenario> --param PATH (--values a,b,c | --from X --to Y [--steps N])\n\
         \x20              [--seeds N] [--until SECONDS] [--metric NAME] [--set PATH=VALUE]...\n\
         \x20              [--dir DIR] [--out DIR] [--quiet]\n\
         \n\
           <scenario>  bare name inside --dir, or a path to a .toml\n\
           --param     dotted path into the scenario, e.g. sim.track_hold_s\n\
           --values    explicit list to sweep\n\
           --from/--to/--steps   linear grid instead, inclusive of both ends\n\
           --seeds     trials per arm, seeds 0..N; all arms share them (default: 200)\n\
           --until     sim seconds per trial (default: 600)\n\
           --metric    which column the paired report is about (default: red_losses)\n\
           --set       hold another dial fixed across every arm; repeatable\n\
         \n\
         metrics: {}",
        COLUMNS.join(", ")
    )
}
