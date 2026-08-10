//! Vary several dials at once and report whether they interact.
//!
//! `sweep` answers "what does this dial do". This answers "what do these dials do, and does
//! either one's answer depend on the other" — which is a different question, and on this
//! model it has more than once been the more important one.
//!
//! ```text
//! # The 2x2 that explained the fires_c2 inversion: an overkill cap against how fast
//! # targets are acquired. Neither factor alone tells the story.
//! factorial fires_c2 --factor sim.fires_need_c2=false,true \
//!                    --factor blue.sensors.0.type=mast_optical,ciws_radar \
//!                    --seeds 500 --until 300 --metric red_cleared_s
//!
//! # Three factors, two levels each: eight cells, all paired over one seed set.
//! factorial ad_c2 --factor sim.max_batteries_per_air_target=1,3 \
//!                 --factor sim.allocation=greedy,optimal \
//!                 --factor c2.ad_command_post.coordination_range_m=500,6000 \
//!                 --seeds 1000 --metric ad_rounds_left
//! ```
//!
//! # What it reports, and why in that order
//!
//! **Main effects** first, each averaged over every level of the other factors, so a factor
//! is described by what it does across the design rather than at one corner of it.
//!
//! **Interactions** second, and the closing line says whether any is significant — because
//! that decides whether the main effects above may be read on their own. If two dials
//! interact, "this one is worth −11 s" is a sentence with a missing clause.
//!
//! Every cell runs the same seed set, so every contrast is formed seed by seed. See
//! [`experiments::design`].

use experiments::design::{cells, Factor, Factorial};
use experiments::outcome::{Outcome, COLUMNS};
use experiments::patch::{self, scenario_with_overrides, Override};
use experiments::study::{run_study, StudyConfig};
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

    let factors: Vec<Factor> = match flags(&args, "--factor")
        .iter()
        .map(|s| Factor::parse(s))
        .collect()
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}\n\n{}", usage());
            std::process::exit(2);
        }
    };
    if factors.is_empty() {
        eprintln!("at least one --factor is required\n\n{}", usage());
        std::process::exit(2);
    }

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

    let design = cells(&factors);
    println!(
        "=== factorial: {name} \u{b7} {} factor(s), {} cell(s) \u{d7} {} seeds, {:.0} s each, \
         {} threads ===",
        factors.len(),
        design.len(),
        cfg.seeds,
        cfg.until_s,
        rayon::current_num_threads()
    );
    for f in &factors {
        println!("    {} : {}", f.path, f.levels.join(", "));
    }

    let keys: Vec<&str> = factors.iter().map(|f| f.path.as_str()).collect();
    let mut trial_keys: Vec<&str> = keys.clone();
    trial_keys.push("seed");
    let mut trials = csv::trial_header(&trial_keys);
    let mut summary = csv::summary_header(&keys);
    let mut per_cell: Vec<Vec<Outcome>> = Vec::with_capacity(design.len());
    let started = Instant::now();

    for cell in &design {
        let labels: Vec<String> = cell
            .iter()
            .enumerate()
            .map(|(fi, &li)| factors[fi].levels[li].clone())
            .collect();

        let mut overrides = fixed.clone();
        for (fi, &li) in cell.iter().enumerate() {
            overrides.push(Override {
                path: factors[fi].path.clone(),
                value: patch::parse_value(&factors[fi].levels[li]),
            });
        }
        let (lib_overrides, scenario_overrides) = patch::split(&overrides);
        let libs = match patch::libraries_with_overrides(&dir, &lib_overrides) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("  cell {}: {e}", labels.join(" "));
                std::process::exit(2);
            }
        };
        let scn = match scenario_with_overrides(&text, &scenario_overrides) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  cell {}: {e}", labels.join(" "));
                std::process::exit(2);
            }
        };
        println!("  {}", labels.join("  "));
        let outcomes = match run_study(&scn, &libs, cfg) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("  cell {}: does not resolve ({e})", labels.join(" "));
                std::process::exit(2);
            }
        };
        for (seed, o) in outcomes.iter().enumerate() {
            let mut row = labels.clone();
            row.push(seed.to_string());
            csv::push_trial(&mut trials, &row, o);
        }
        let summaries = csv::push_summary(&mut summary, &labels, &outcomes);
        println!("      {metric_name:<20} {}", summaries[metric]);
        per_cell.push(outcomes);
    }

    let result = Factorial::new(factors, design, &per_cell, metric);
    println!("{}", result.report(&metric_name));

    let stem = format!("{name}_factorial");
    csv::write(&out_dir.join(format!("{stem}.csv")), &trials);
    csv::write(&out_dir.join(format!("{stem}_summary.csv")), &summary);
    let secs = started.elapsed().as_secs_f64();
    let n: usize = per_cell.iter().map(Vec::len).sum();
    println!(
        "{n} trials in {secs:.1} s ({:.0}/s)",
        n as f64 / secs.max(1e-9)
    );
}

/// A bare name resolves inside `dir`; anything with a separator or an extension is a path.
fn resolve_scenario(dir: &Path, arg: &str) -> PathBuf {
    let p = Path::new(arg);
    if p.extension().is_some() || arg.contains('/') || arg.contains('\\') {
        p.to_path_buf()
    } else {
        dir.join(format!("{arg}.toml"))
    }
}

fn usage() -> String {
    format!(
        "factorial: vary several dials at once and report their interaction\n\
         \n\
         usage: factorial <scenario> --factor PATH=v1,v2 [--factor PATH=v1,v2]...\n\
         \x20              [--seeds N] [--until SECONDS] [--metric NAME] [--set PATH=VALUE]...\n\
         \x20              [--dir DIR] [--out DIR] [--quiet]\n\
         \n\
         \x20 <scenario>  bare name inside --dir, or a path to a .toml\n\
         \x20 --factor    a dial and at least two levels; repeatable. Level 1 is the\n\
         \x20             baseline every effect is measured against\n\
         \x20 --seeds     trials per CELL, seeds 0..N; every cell shares them (default: 200)\n\
         \x20 --until     sim seconds per trial (default: 600)\n\
         \x20 --metric    which column the effects are about (default: red_losses)\n\
         \x20 --set       hold another dial fixed across every cell; repeatable\n\
         \n\
         \x20 Cost is the product of the level counts: three 2-level factors is 8 cells.\n\
         \n\
         metrics: {}",
        COLUMNS.join(", ")
    )
}
