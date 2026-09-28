//! The validation report: run every gate and print what was checked, against what, and
//! whether it held.
//!
//! `cargo test` answers "are the tests green". This answers the question the project
//! actually cares about - *is the maths still right, and right against what?* - by
//! printing each gate beside the closed form or invariant it is compared to.
//!
//! Run: `cargo run -p validation --bin validation_report`
//!      `cargo run -p validation --release --bin validation_report   # much faster`
//!
//! `--markdown` prints the same catalogue as the table `docs/VALIDATION.md` publishes,
//! grouped by section, and runs nothing. The published table is regenerated from here
//! rather than hand-maintained: `tests/catalogue.rs` already pins `GATES` to the suite in
//! both directions, so a generated table cannot claim a gate the tests do not enforce.

use std::collections::BTreeMap;
use std::process::Command;
use validation::gates::GATES;

fn main() {
    if std::env::args().any(|a| a == "--markdown") {
        print_markdown();
        return;
    }
    let release = cfg!(not(debug_assertions));
    println!("=== Validation report (docs/THEORY.md) ===");
    println!(
        "running the gate suites{}...\n",
        if release { " [release]" } else { "" }
    );

    // Both packages: nearly every gate lives in `validation`, but V52's zero-draw half
    // asserts a property of the RNG stream and so stays a unit test inside sim_core.
    let mut results: BTreeMap<String, bool> = BTreeMap::new();
    for pkg in ["validation", "sim_core"] {
        match run_tests(pkg, release) {
            Ok(r) => results.extend(r),
            Err(e) => {
                eprintln!("could not run tests for {pkg}: {e}");
                std::process::exit(2);
            }
        }
    }

    let (mut passed, mut failed, mut missing) = (0u32, 0u32, 0u32);
    println!(
        "{:<5} {:<36} {:<8} checked against",
        "gate", "property", "result"
    );
    println!("{}", "-".repeat(110));
    for gate in GATES {
        // A gate is green only if *every* test enforcing it is green.
        let outcomes: Vec<Option<bool>> = gate
            .tests
            .iter()
            .map(|t| results.get(*t).copied())
            .collect();
        let status = if outcomes.iter().any(Option::is_none) {
            missing += 1;
            "MISSING"
        } else if outcomes.iter().all(|o| o == &Some(true)) {
            passed += 1;
            "ok"
        } else {
            failed += 1;
            "FAILED"
        };
        println!(
            "{:<5} {:<36} {:<8} {}",
            gate.id, gate.property, status, gate.reference
        );
    }

    println!("{}", "-".repeat(110));
    println!(
        "{passed} gates held, {failed} failed, {missing} missing (of {})",
        GATES.len()
    );
    if failed > 0 || missing > 0 {
        std::process::exit(1);
    }
}

/// Print the catalogue as a Markdown table, grouped by section, for `docs/VALIDATION.md`.
///
/// `GATES` is in gate-number order, which is chronological rather than structural, so the
/// distinct sections are collected and sorted numerically. A plain string sort would put
/// §10 before §2 - the sort of thing that only bites once there are ten sections.
fn print_markdown() {
    let mut sections: Vec<&str> = GATES.iter().map(|g| g.section).collect();
    sections.sort_by_key(|s| section_order(s));
    sections.dedup();

    println!("<!-- Generated:  cargo run -p validation --bin validation_report -- --markdown");
    println!("     Edit crates/validation/src/gates.rs, not this table. -->");
    for section in sections {
        println!("\n#### {section} - {}\n", section_title(section));
        println!("| Gate | Property | Checked against |");
        println!("|---|---|---|");
        for gate in GATES.iter().filter(|g| g.section == section) {
            // A pipe inside a cell would end it early. Nothing else in these strings is
            // Markdown-significant - they are written as plain prose.
            let property = gate.property.replace('|', r"\|");
            let reference = gate.reference.replace('|', r"\|");
            println!("| {} | {} | {} |", gate.id, property, reference);
        }
    }
    println!("\n{} gates.", GATES.len());
}

/// The `docs/THEORY.md` title for a section label, for the generated table's headings.
///
/// Presentation only, and so it lives in the presenter rather than in `gates.rs`: a title per
/// gate would be the same string repeated a dozen times, and a section's title is the one part
/// of it that may be reworded without any gate changing meaning.
fn section_title(section: &str) -> &'static str {
    match section_order(section) {
        1 => "Terrain and line of sight",
        2 => "Fires",
        3 => "Sensing and detection",
        4 => "Suppression and attrition",
        5 => "Movement as dynamic programming",
        6 => "The game-theoretic layer",
        7 => "The simulation loop",
        8 => "Electronic warfare and partial observability",
        9 => "Air: drones and counter-air",
        10 => "The decision layer",
        11 => "Command and control",
        12 => "SEAD: air defence as a target",
        13 => "The kill chain: directed targeting",
        14 => "The measurement machinery",
        _ => "Unnumbered",
    }
}

/// Numeric sort key for a section label such as `"§10"` or `"§7.6"`.
fn section_order(section: &str) -> u32 {
    section
        .trim_start_matches('§')
        .split('.')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(u32::MAX)
}

/// Run a package's tests and return `test name -> passed`.
///
/// Parses libtest's human output rather than its JSON, which still requires nightly.
/// Integration tests print bare function names, which is exactly what the catalogue
/// stores.
fn run_tests(pkg: &str, release: bool) -> Result<BTreeMap<String, bool>, String> {
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args(["test", "-p", pkg]);
    if release {
        cmd.arg("--release");
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let mut results = BTreeMap::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("test ") else {
            continue;
        };
        let Some((name, outcome)) = rest.split_once(" ... ") else {
            continue;
        };
        // Unit tests inside sim_core carry a module path; the catalogue names the fn.
        let name = name.rsplit("::").next().unwrap_or(name).trim();
        let ok = outcome.trim().starts_with("ok");
        results.insert(name.to_owned(), ok);
    }
    Ok(results)
}
