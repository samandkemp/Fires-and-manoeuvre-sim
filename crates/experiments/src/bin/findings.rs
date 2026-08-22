//! Re-run the findings the documentation states, and report any that have drifted.
//!
//! ```text
//! cargo run -p experiments --release --bin findings
//! cargo run -p experiments --release --bin findings -- --only allocation-optimal-vs-greedy
//! cargo run -p experiments --release --bin findings -- --quick
//! ```
//!
//! Exits non-zero if any finding drifted or broke, so it can be run on a schedule and
//! noticed. It is deliberately **not** a `cargo test`: these are thousands of trials and
//! take minutes, which is the wrong shape for a suite that has to stay fast enough to run
//! on every edit.
//!
//! `--quick` runs every finding at a fraction of its seeds. That is enough to catch a
//! finding that has *broken* - a renamed dial, a deleted scenario, an inverted sign - but
//! not enough to judge drift, so it reports rather than failing on the comparison.

use experiments::findings::{check, parse_manifest, Verdict};
use std::path::Path;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = root.join("scenarios");
    let manifest_path = root.join("findings.toml");

    let only = experiments::flag(&args, "--only");
    let quick = experiments::has_flag(&args, "--quick");

    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read {}: {e}", manifest_path.display());
            std::process::exit(2);
        }
    };
    let manifest = match parse_manifest(&text) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{}: {e}", manifest_path.display());
            std::process::exit(2);
        }
    };

    let selected: Vec<_> = manifest
        .findings
        .iter()
        .filter(|f| only.as_ref().is_none_or(|id| &f.id == id))
        .collect();
    if selected.is_empty() {
        eprintln!("no findings matched");
        std::process::exit(2);
    }

    let total: u64 = selected
        .iter()
        .map(|f| if quick { quick_seeds(f.seeds) } else { f.seeds } * 2)
        .sum();
    println!(
        "Re-running {} finding(s), {total} trials{}\n",
        selected.len(),
        if quick { " (quick: seeds reduced)" } else { "" }
    );

    let started = Instant::now();
    let mut drifted = Vec::new();
    let mut broken = Vec::new();

    for f in selected {
        let mut f = (*f).clone();
        if quick {
            f.seeds = quick_seeds(f.seeds);
        }
        let out = check(&f, &dir);
        println!("{}", out.line());
        match out.verdict {
            // Under --quick the seed count is too small to judge a difference, so a
            // comparison failure is reported above and not counted against the exit code.
            Verdict::Drifted if !quick => drifted.push((f.claim.clone(), out)),
            Verdict::Broken => broken.push((f.claim.clone(), out)),
            _ => {}
        }
    }

    println!("\nchecked in {:.1?}", started.elapsed());

    if !broken.is_empty() {
        println!("\n--- broken: these could not be measured at all ---");
        for (claim, c) in &broken {
            println!("  {}: {}", c.id, c.error.as_deref().unwrap_or("?"));
            println!("      claim: {claim}");
        }
    }
    if !drifted.is_empty() {
        println!("\n--- drifted: the model moved and the documents did not ---");
        for (claim, c) in &drifted {
            println!(
                "  {}: documented {:+.3}, measured {:+.3} (drift {:+.3}, tolerance {:.3})",
                c.id,
                c.expect,
                c.paired.as_ref().map_or(f64::NAN, |p| p.mean),
                c.drift().unwrap_or(f64::NAN),
                c.tolerance
            );
            println!("      claim: {claim}");
            if c.documented_in.is_empty() {
                println!("      documented in: (not recorded)");
            } else {
                println!("      update: {}", c.documented_in.join(", "));
            }
        }
    }

    if !drifted.is_empty() || !broken.is_empty() {
        std::process::exit(1);
    }
    if quick {
        // Say only what was established. A quick pass has too few seeds to judge a
        // difference, so any DRIFTED line above is most likely sampling noise - and
        // claiming the findings hold would be exactly the overstatement this tool
        // exists to catch.
        println!(
            "
Every finding ran to completion. Seeds were reduced, so any drift reported above
is not evidence either way - run without --quick to judge it."
        );
    } else {
        println!(
            "
All findings hold."
        );
    }
}

/// Seeds for a `--quick` pass: enough to exercise every code path a finding touches, far too
/// few to judge a difference by. A tenth, floored at eight so a small finding still runs
/// more than a couple of trials.
fn quick_seeds(seeds: u64) -> u64 {
    (seeds / 10).max(8)
}
