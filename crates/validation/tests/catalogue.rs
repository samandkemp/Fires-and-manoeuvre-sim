//! The catalogue must not drift from the suite.
//!
//! `validation::gates::GATES` is the machine-readable twin of `docs/DESIGN.md`'s
//! validation tables, and a catalogue that quietly disagrees with the tests is worse than
//! none — it would report a gate as held when nothing checks it. These two gates make
//! that impossible in both directions: every catalogued test exists, and every `vNN_*`
//! test is catalogued.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use validation::gates::GATES;

/// Every source file that can hold a gate: this crate's suites, plus the one unit test
/// left inside sim_core (V52's zero-draw half, which tests the RNG stream).
fn gate_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<PathBuf> = fs::read_dir(root.join("tests"))
        .expect("tests dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .collect();
    files.push(root.join("../sim_core/src/sim/mod.rs"));
    files.sort();
    files
}

/// Every `fn name(` defined across those sources.
fn defined_fns() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for path in gate_sources() {
        let text = fs::read_to_string(&path).unwrap_or_default();
        for line in text.lines() {
            let line = line.trim_start();
            if let Some(rest) = line.strip_prefix("fn ") {
                if let Some((name, _)) = rest.split_once('(') {
                    out.insert(name.trim().to_owned());
                }
            }
        }
    }
    out
}

#[test]
fn every_catalogued_gate_names_a_test_that_exists() {
    let defined = defined_fns();
    let mut missing = Vec::new();
    for gate in GATES {
        assert!(
            !gate.tests.is_empty(),
            "{} claims no test — a gate with nothing enforcing it is a lie",
            gate.id
        );
        for t in gate.tests {
            if !defined.contains(*t) {
                missing.push(format!("{} -> {t}", gate.id));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "catalogue names tests that do not exist: {missing:#?}"
    );
}

#[test]
fn every_gate_test_in_the_suite_is_catalogued() {
    let catalogued: BTreeSet<&str> = GATES.iter().flat_map(|g| g.tests.iter().copied()).collect();
    let orphans: Vec<String> = defined_fns()
        .into_iter()
        .filter(|f| {
            // A gate test is named for its V-number: `v14_...`, `v52_...`.
            f.starts_with('v')
                && f.len() > 1
                && f[1..].chars().next().is_some_and(|c| c.is_ascii_digit())
                && !catalogued.contains(f.as_str())
        })
        .collect();
    assert!(
        orphans.is_empty(),
        "these V-numbered tests are missing from the catalogue (and so from the report \
         and docs/DESIGN.md): {orphans:#?}"
    );
}

#[test]
fn gate_ids_are_unique_and_in_order() {
    let mut previous = 0u32;
    for gate in GATES {
        let n: u32 = gate
            .id
            .strip_prefix('V')
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("gate id {} is not V<number>", gate.id));
        assert!(
            n > previous,
            "gate ids must be unique and ascending: {} follows V{previous}",
            gate.id
        );
        previous = n;
        assert!(
            !gate.reference.is_empty(),
            "{} states no reference",
            gate.id
        );
    }
}
