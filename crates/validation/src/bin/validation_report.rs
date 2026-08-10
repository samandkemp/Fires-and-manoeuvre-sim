//! The validation report: run every gate and print what was checked, against what, and
//! whether it held.
//!
//! `cargo test` answers "are the tests green". This answers the question the project
//! actually cares about - *is the maths still right, and right against what?* - by
//! printing each gate beside the closed form or invariant it is compared to.
//!
//! Run: `cargo run -p validation --bin validation_report`
//!      `cargo run -p validation --release --bin validation_report   # much faster`

use std::collections::BTreeMap;
use std::process::Command;
use validation::gates::GATES;

fn main() {
    let release = cfg!(not(debug_assertions));
    println!("=== Validation report (docs/DESIGN.md) ===");
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
