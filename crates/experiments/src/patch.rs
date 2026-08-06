//! Overriding a dial in a scenario before it is parsed.
//!
//! A sweep needs to ask "what if `track_hold_s` were 20 instead of 45?" without ten copies
//! of a scenario file differing by one line. The patch happens on the **TOML**, not on the
//! parsed [`Scenario`]: the file is read as a `toml::Value`, the named leaf is replaced,
//! and the result is handed back to `Scenario::from_toml_str`.
//!
//! Doing it that way means the sweep needs no knowledge of the scenario schema at all —
//! any field reachable by a dotted path is sweepable, including ones added later — and,
//! more importantly, the patched scenario goes through **exactly the same validation** as
//! one loaded from disk. A typo'd path or an out-of-range value fails at load with the
//! normal error, not silently.
//!
//! ```text
//! sim.track_hold_s = 20        # a [sim] dial
//! sim.allocation = greedy      # a string-valued dial
//! red.air.0.altitude_m = 250   # numeric segments index into an array
//! ```
//!
//! **Limitation.** This reaches the *scenario* file only. Stat-block library dials — a
//! sensor's `lambda0_per_s`, a weapon's `cep_m` — live in `sensors.toml` and `weapons.toml`
//! and are not reachable this way. Sweeping those needs the same treatment applied to
//! `Libraries::load_dir`; see `docs/EXPERIMENTS.md`.

use sim_core::scenario::{Scenario, ScenarioError};

/// One `path=value` override, already split.
#[derive(Clone, Debug, PartialEq)]
pub struct Override {
    pub path: String,
    pub value: toml::Value,
}

impl Override {
    /// Parse a `sim.track_hold_s=20` command-line argument.
    ///
    /// # Errors
    /// A string with no `=` in it.
    pub fn parse(arg: &str) -> Result<Self, String> {
        let (path, value) = arg
            .split_once('=')
            .ok_or_else(|| format!("expected path=value, got '{arg}'"))?;
        Ok(Self {
            path: path.trim().to_owned(),
            value: parse_value(value.trim()),
        })
    }
}

/// Turn a command-line word into the TOML type it most obviously is.
///
/// Integer **before** float, deliberately: `max_shooters_per_target` is a `u32` and would
/// refuse a float, whereas every float dial in the schema accepts an integer (serde's
/// numeric visitors widen). So `2` must stay an integer and `2.0` must stay a float.
#[must_use]
pub fn parse_value(text: &str) -> toml::Value {
    if let Ok(i) = text.parse::<i64>() {
        return toml::Value::Integer(i);
    }
    if let Ok(f) = text.parse::<f64>() {
        return toml::Value::Float(f);
    }
    match text {
        "true" => toml::Value::Boolean(true),
        "false" => toml::Value::Boolean(false),
        other => toml::Value::String(other.to_owned()),
    }
}

/// Load a scenario from TOML text with `overrides` applied.
///
/// # Errors
/// [`ScenarioError::Invalid`] if a path does not exist or does not lead to a leaf, and the
/// usual parse/validation errors if the patched scenario is not a valid one.
pub fn scenario_with_overrides(
    text: &str,
    overrides: &[Override],
) -> Result<Scenario, ScenarioError> {
    if overrides.is_empty() {
        return Scenario::from_toml_str(text);
    }
    let mut doc: toml::Value =
        toml::from_str(text).map_err(|e| ScenarioError::Invalid(format!("{e}")))?;
    for ov in overrides {
        set_path(&mut doc, &ov.path, ov.value.clone())
            .map_err(|e| ScenarioError::Invalid(format!("override '{}': {e}", ov.path)))?;
    }
    let patched =
        toml::to_string(&doc).map_err(|e| ScenarioError::Invalid(format!("re-encode: {e}")))?;
    Scenario::from_toml_str(&patched)
}

/// Replace the value at a dotted path. Numeric segments index into an array.
///
/// Every segment *but the last* must already exist — `red.air.0` has to be a drone that is
/// there. The **leaf** may be created, and usually is: nearly every dial in `[sim]` has a
/// serde default, so a scenario that is happy with 45 s of track hold simply does not
/// mention `track_hold_s`, and refusing to add it would make most of the interesting dials
/// unsweepable.
///
/// That is safe only because the schema rejects unknown keys (`Scenario`'s
/// `deny_unknown_fields`): a typo'd leaf is created here and then refused by the loader,
/// naming the key. Without that it would take the default and produce a silently
/// meaningless study.
fn set_path(doc: &mut toml::Value, path: &str, value: toml::Value) -> Result<(), String> {
    let mut segments: Vec<&str> = path.split('.').collect();
    let leaf = segments.pop().ok_or("empty path")?;
    let mut cursor = doc;
    for seg in segments {
        cursor = descend(cursor, seg)?;
    }
    match cursor {
        toml::Value::Table(t) => {
            t.insert(leaf.to_owned(), value);
            Ok(())
        }
        toml::Value::Array(a) => {
            let i: usize = leaf
                .parse()
                .map_err(|_| format!("'{leaf}' is not an index"))?;
            *a.get_mut(i).ok_or_else(|| format!("index {i} past end"))? = value;
            Ok(())
        }
        other => Err(format!("'{leaf}' sits under a {}", other.type_str())),
    }
}

/// Step one segment down, through a table key or an array index.
///
/// A missing *table* is created, for the same reason a missing leaf is: a scenario content
/// with the defaults omits the whole `[sim]` block, and `sim.track_hold_s` still has to be
/// settable on it. A missing *array element* is not — there is no sensible drone to invent.
fn descend<'a>(cursor: &'a mut toml::Value, seg: &str) -> Result<&'a mut toml::Value, String> {
    match cursor {
        toml::Value::Table(t) => Ok(t
            .entry(seg.to_owned())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))),
        toml::Value::Array(a) => {
            let i: usize = seg
                .parse()
                .map_err(|_| format!("'{seg}' is not an index"))?;
            let n = a.len();
            a.get_mut(i)
                .ok_or_else(|| format!("index {i} past end of a {n}-element array"))
        }
        other => Err(format!("'{seg}' sits under a {}", other.type_str())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCN: &str = r#"
name = "patch test"
default_seed = 3

[terrain]
cell_size_m = 10.0
width_cells = 40
height_cells = 40

[terrain.source.flat]
elevation_m = 0.0

[sim]
track_hold_s = 45.0
max_shooters_per_target = 3
allocation = "optimal"

[[red.air]]
id = "uas-1"
type = "recce"
pos = [100.0, 100.0]
altitude_m = 300.0
heading_deg = 90.0
"#;

    #[test]
    fn a_float_dial_is_replaced() {
        let ov = [Override::parse("sim.track_hold_s=20").unwrap()];
        let scn = scenario_with_overrides(SCN, &ov).expect("patches cleanly");
        assert!((scn.sim.track_hold_s - 20.0).abs() < 1e-6);
    }

    /// The integer-before-float rule: a `u32` dial must not receive a TOML float.
    #[test]
    fn an_integer_dial_takes_an_integer() {
        let ov = [Override::parse("sim.max_shooters_per_target=2").unwrap()];
        let scn = scenario_with_overrides(SCN, &ov).expect("patches cleanly");
        assert_eq!(scn.sim.max_shooters_per_target, 2);
        // And a float dial still accepts a bare integer, which is the other half of it.
        let ov = [Override::parse("sim.track_hold_s=30").unwrap()];
        let scn = scenario_with_overrides(SCN, &ov).expect("integers widen to floats");
        assert!((scn.sim.track_hold_s - 30.0).abs() < 1e-6);
    }

    #[test]
    fn a_string_dial_selects_a_solver() {
        use sim_core::scenario::AllocationChoice;
        let ov = [Override::parse("sim.allocation=greedy").unwrap()];
        let scn = scenario_with_overrides(SCN, &ov).expect("patches cleanly");
        assert_eq!(scn.sim.allocation, AllocationChoice::Greedy);
    }

    #[test]
    fn a_numeric_segment_indexes_an_array() {
        let ov = [Override::parse("red.air.0.altitude_m=250").unwrap()];
        let scn = scenario_with_overrides(SCN, &ov).expect("patches cleanly");
        assert!((scn.red.air[0].altitude_m - 250.0).abs() < 1e-6);
    }

    /// The common case, and the reason the leaf may be created: nearly every `[sim]` dial
    /// has a default, so a scenario happy with it simply does not mention the key. If this
    /// failed, most of the interesting dials would be unsweepable.
    #[test]
    fn a_defaulted_dial_absent_from_the_file_can_still_be_set() {
        const BARE: &str = r#"
name = "no sim block"
default_seed = 1

[terrain]
cell_size_m = 10.0
width_cells = 20
height_cells = 20

[terrain.source.flat]
elevation_m = 0.0
"#;
        let plain = Scenario::from_toml_str(BARE).expect("valid without a [sim] block");
        assert!((plain.sim.track_hold_s - 45.0).abs() < 1e-6, "the default");
        let ov = [Override::parse("sim.track_hold_s=12").unwrap()];
        let scn = scenario_with_overrides(BARE, &ov).expect("creates the block");
        assert!((scn.sim.track_hold_s - 12.0).abs() < 1e-6);
    }

    /// A typo'd path must not quietly become a study of a dial nothing reads. The patcher
    /// creates the key; the schema's `deny_unknown_fields` is what rejects it, which is
    /// also what protects hand-written scenario files.
    #[test]
    fn a_misspelt_path_is_refused_by_the_schema() {
        let ov = [Override::parse("sim.track_hold=20").unwrap()];
        let err = scenario_with_overrides(SCN, &ov).expect_err("should refuse");
        assert!(format!("{err}").contains("track_hold"), "{err}");
        let ov = [Override::parse("nope.at.all=1").unwrap()];
        assert!(scenario_with_overrides(SCN, &ov).is_err());
    }

    /// An array element, unlike a table, is never invented: there is no sensible drone to
    /// make up, and a sweep over `red.air.9` when there are three is a mistake.
    #[test]
    fn an_index_past_the_end_is_an_error() {
        let ov = [Override::parse("red.air.9.altitude_m=1").unwrap()];
        let err = scenario_with_overrides(SCN, &ov).expect_err("should refuse");
        assert!(format!("{err}").contains("past end"), "{err}");
    }

    /// The patched text goes through the same loader as a file, so an invalid value is
    /// rejected the same way rather than reaching the sim.
    #[test]
    fn a_patched_scenario_is_validated_like_a_loaded_one() {
        let ov = [Override::parse("terrain.width_cells=-5").unwrap()];
        assert!(scenario_with_overrides(SCN, &ov).is_err());
    }

    #[test]
    fn no_overrides_is_the_plain_loader() {
        let plain = Scenario::from_toml_str(SCN).expect("valid");
        let patched = scenario_with_overrides(SCN, &[]).expect("valid");
        assert_eq!(plain.name, patched.name);
        assert_eq!(plain.default_seed, patched.default_seed);
    }
}
