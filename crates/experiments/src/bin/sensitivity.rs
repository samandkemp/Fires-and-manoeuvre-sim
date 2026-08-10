//! Which dials actually drive a result, and which are noise?
//!
//! Every number in this project is an abstract placeholder. That is deliberate - the models
//! are the product - but it leaves one question over every finding: **does it matter that
//! these numbers are invented?** A `sweep` cannot answer it, because it varies one dial with
//! the rest pinned wherever the scenario happened to leave them.
//!
//! ```text
//! sensitivity studies/sensing.toml --seeds 40
//! ```
//!
//! The study is a file rather than a pile of flags, because a dial space is a *design* -
//! something to be committed, reviewed and re-run, not retyped. See `studies/README.md`.
//!
//! # Two passes, cheap then thorough
//!
//! **Morris** screens: `r · (k + 1)` design points, ranking dials by `mu_star` and flagging
//! non-linearity with `sigma`. Its job is to say what can be ignored.
//!
//! **Sobol** decomposes the variance: `n · (k + 2)` points giving `S1` (what a dial explains
//! alone) and `ST` (what it is involved in altogether). `ST − S1` is the share of a dial's
//! influence that runs through interactions - invisible to a one-dial sweep, by
//! construction.
//!
//! Cost is the product of design points and simulation seeds, so `--seeds` is deliberately
//! small here: a design point is an *average over seeds*, and the variance being decomposed
//! is the one across the dial space, not across the dice.

use experiments::outcome::{Outcome, COLUMNS};
use experiments::patch::{self, scenario_with_overrides, Override};
use experiments::sensitivity::{
    morris_design, morris_indices, sobol_design, sobol_indices, Dial, Index, Point,
};
use experiments::study::{run_design, StudyConfig};
use experiments::{flag, flag_or, has_flag};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// A study read from TOML: what to run, over which dials, measuring what.
struct Study {
    scenario: String,
    metric: String,
    dials: Vec<Dial>,
    trajectories: usize,
    levels: usize,
    sobol_n: usize,
    design_seed: u64,
}

fn parse_study(text: &str) -> Result<Study, String> {
    let v: toml::Value = toml::from_str(text).map_err(|e| format!("not valid TOML: {e}"))?;
    let get = |k: &str| v.get(k).and_then(toml::Value::as_str).map(str::to_owned);
    let scenario = get("scenario").ok_or("study needs `scenario = \"name\"`")?;
    let metric = get("metric").unwrap_or_else(|| "red_losses".to_owned());
    let num = |k: &str, d: i64| v.get(k).and_then(toml::Value::as_integer).unwrap_or(d);

    let table = v
        .get("dials")
        .and_then(toml::Value::as_table)
        .ok_or("study needs a [dials] table of `path = [lo, hi]`")?;
    let mut dials = Vec::new();
    for (path, range) in table {
        let pair = range
            .as_array()
            .filter(|a| a.len() == 2)
            .ok_or_else(|| format!("dial '{path}' needs [lo, hi]"))?;
        let f = |i: usize| -> Result<f64, String> {
            pair[i]
                .as_float()
                .or_else(|| pair[i].as_integer().map(|n| n as f64))
                .ok_or_else(|| format!("dial '{path}': bound {i} is not a number"))
        };
        let (lo, hi) = (f(0)?, f(1)?);
        // Only a strict `lo < hi` is acceptable, and `partial_cmp` says so for NaN too: a
        // range that is empty, backwards or not-a-number would otherwise be sampled and the
        // whole study would decompose the variance of nonsense.
        if !matches!(lo.partial_cmp(&hi), Some(std::cmp::Ordering::Less)) {
            return Err(format!("dial '{path}': needs lo < hi, got [{lo}, {hi}]"));
        }
        dials.push(Dial {
            path: path.clone(),
            lo,
            hi,
        });
    }
    if dials.is_empty() {
        return Err("study has no dials".to_owned());
    }
    // BTreeMap order from TOML is already deterministic; state it so a reader knows the
    // report's row order is stable across runs.
    dials.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(Study {
        scenario,
        metric,
        dials,
        trajectories: num("trajectories", 20).max(1) as usize,
        levels: num("levels", 4).max(2) as usize,
        sobol_n: num("sobol_n", 256).max(4) as usize,
        design_seed: num("design_seed", 1) as u64,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || has_flag(&args, "--help") {
        println!("{}", usage());
        return;
    }
    let study_path = PathBuf::from(&args[0]);
    let text = match std::fs::read_to_string(&study_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("could not read {}: {e}", study_path.display());
            std::process::exit(2);
        }
    };
    let study = match parse_study(&text) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: {e}", study_path.display());
            std::process::exit(2);
        }
    };

    let dir = PathBuf::from(flag(&args, "--dir").unwrap_or_else(|| "scenarios".to_owned()));
    let cfg = StudyConfig {
        seeds: flag_or(&args, "--seeds", 30),
        until_s: flag_or(&args, "--until", 600.0),
        progress: false,
    };
    let Some(metric) = Outcome::column(&study.metric) else {
        eprintln!(
            "unknown metric '{}'. Known: {}",
            study.metric,
            COLUMNS.join(", ")
        );
        std::process::exit(2);
    };

    let scn_path = resolve_scenario(&dir, &study.scenario);
    let scn_text = match std::fs::read_to_string(&scn_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("could not read {}: {e}", scn_path.display());
            std::process::exit(2);
        }
    };

    let k = study.dials.len();
    let morris = morris_design(k, study.trajectories, study.levels, study.design_seed);
    let sobol = sobol_design(k, study.sobol_n, study.design_seed ^ 0x5A17);
    let total = morris.len() + sobol.len();

    println!(
        "=== sensitivity: {} \u{b7} {k} dial(s), metric {} ===",
        study.scenario, study.metric
    );
    for d in &study.dials {
        println!("    {:<44} [{}, {}]", d.path, d.lo, d.hi);
    }
    println!(
        "    Morris {} points + Sobol {} points, {} seeds each = {} trials",
        morris.len(),
        sobol.len(),
        cfg.seeds,
        total as u64 * cfg.seeds
    );

    let started = Instant::now();

    // Build every design point up front, then hand the lot to `run_design`, which builds
    // terrain ONCE per worker for the whole design. Evaluating points one at a time through
    // `run_study` rebuilds terrain per point - on this scenario's 1000x1000 map that is
    // ~19,000 builds for the design below, and it dominates everything else by orders of
    // magnitude.
    let build = |points: &[Point]| -> Vec<(sim_core::scenario::Scenario, _)> {
        points
            .iter()
            .map(|pt| point_scenario(&study, pt, &scn_text, &dir))
            .collect()
    };

    let base_libs = match patch::libraries_with_overrides(&dir, &[]) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("could not load stat blocks: {e}");
            std::process::exit(2);
        }
    };
    let base_scn = match scenario_with_overrides(&scn_text, &[]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: {e}", scn_path.display());
            std::process::exit(2);
        }
    };

    let eval = |points: &[Point], label: &str| -> Vec<f64> {
        eprintln!("  {label}: {} design points", points.len());
        let built = build(points);
        let per_point = match run_design(
            &base_scn,
            &base_libs,
            &built,
            StudyConfig {
                progress: true,
                ..cfg
            },
        ) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("design does not resolve: {e}");
                std::process::exit(2);
            }
        };
        per_point
            .iter()
            .map(|trials| {
                let n = trials.len().max(1) as f64;
                trials.iter().map(|o| o.values()[metric]).sum::<f64>() / n
            })
            .collect()
    };

    let m_out = eval(&morris, "morris");
    let m_idx = morris_indices(&morris, &m_out, k, study.trajectories, study.levels);
    let s_out = eval(&sobol, "sobol");
    let s_idx = sobol_indices(&s_out, k, study.sobol_n);

    print!("{}", report(&study, &m_idx, &s_idx));

    let secs = started.elapsed().as_secs_f64();
    println!(
        "{} trials in {secs:.1} s ({:.0}/s)",
        total as u64 * cfg.seeds,
        (total as f64 * cfg.seeds as f64) / secs.max(1e-9)
    );
}

/// One design point, as a patched scenario and stat-block set ready to run.
///
/// Every dial is written as a float, which is why a study's dials are continuous ranges:
/// a categorical dial has no midpoint to sample, and `factorial` is the tool for those.
fn point_scenario(
    study: &Study,
    point: &[f64],
    scn_text: &str,
    dir: &Path,
) -> (sim_core::scenario::Scenario, sim_core::scenario::Libraries) {
    let overrides: Vec<Override> = study
        .dials
        .iter()
        .zip(point)
        .map(|(d, &u)| Override {
            path: d.path.clone(),
            value: toml::Value::Float(d.at(u)),
        })
        .collect();
    let (lib_overrides, scenario_overrides) = patch::split(&overrides);
    let libs = match patch::libraries_with_overrides(dir, &lib_overrides) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("design point rejected: {e}");
            std::process::exit(2);
        }
    };
    let scn = match scenario_with_overrides(scn_text, &scenario_overrides) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("design point rejected: {e}");
            std::process::exit(2);
        }
    };
    (scn, libs)
}

fn report(study: &Study, morris: &[Index], sobol: &[Index]) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "\n--- {}: Morris screening (what to ignore) ---",
        study.metric
    );
    let _ = writeln!(s, "  {:<44} {:>10} {:>10}", "dial", "mu*", "sigma");
    let mut order: Vec<usize> = (0..study.dials.len()).collect();
    order.sort_by(|&a, &b| morris[b].mu_star.total_cmp(&morris[a].mu_star));
    for &i in &order {
        let _ = writeln!(
            s,
            "  {:<44} {:>10.4} {:>10.4}",
            study.dials[i].path, morris[i].mu_star, morris[i].sigma
        );
    }

    let _ = writeln!(
        s,
        "\n--- {}: Sobol variance decomposition ---",
        study.metric
    );
    let _ = writeln!(
        s,
        "  {:<44} {:>8} {:>8} {:>10}",
        "dial", "S1", "ST", "ST - S1"
    );
    let mut order: Vec<usize> = (0..study.dials.len()).collect();
    order.sort_by(|&a, &b| sobol[b].st.total_cmp(&sobol[a].st));
    for &i in &order {
        let (s1, st) = (sobol[i].s1, sobol[i].st);
        let _ = writeln!(
            s,
            "  {:<44} {s1:>8.3} {st:>8.3} {:>10.3}",
            study.dials[i].path,
            st - s1
        );
    }

    let explained: f64 = sobol.iter().map(|i| i.s1).sum();
    let _ = writeln!(
        s,
        "\n  first-order total {explained:.3} \u{2014} {}",
        if explained > 0.9 {
            "the dials are close to additive, so one-at-a-time sweeps are sound here"
        } else {
            "well below 1, so a large share of the variance lives in INTERACTIONS and \
             one-at-a-time sweeps will mislead"
        }
    );

    if let Some(&i) = order.iter().find(|&&i| sobol[i].st - sobol[i].s1 > 0.15) {
        let _ = writeln!(
            s,
            "  {} carries {:.0}% of its influence through interactions \u{2014} sweeping it \
             alone would understate it",
            study.dials[i].path,
            100.0 * (sobol[i].st - sobol[i].s1)
        );
    }
    s.push('\n');
    s
}

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
        "sensitivity: which dials actually drive the answer?\n\
         \n\
         usage: sensitivity <study.toml> [--seeds N] [--until SECONDS] [--dir DIR]\n\
         \n\
         \x20 <study.toml>  the dial space to explore; see studies/README.md\n\
         \x20 --seeds       simulation seeds averaged at each design point (default: 30)\n\
         \x20 --until       sim seconds per trial (default: 600)\n\
         \x20 --dir         scenario folder (default: scenarios)\n\
         \n\
         A study file:\n\
         \n\
         \x20 scenario = \"air_raid\"\n\
         \x20 metric   = \"air_leakers\"\n\
         \x20 sobol_n  = 256          # Sobol base sample; cost is n*(k+2) points\n\
         \x20 trajectories = 20       # Morris trajectories; cost is r*(k+1) points\n\
         \x20 [dials]\n\
         \x20 \"sensors.mast_optical.lambda0_per_s\" = [0.05, 1.0]\n\
         \x20 \"sim.track_hold_s\"                   = [10.0, 120.0]\n\
         \n\
         metrics: {}",
        COLUMNS.join(", ")
    )
}
