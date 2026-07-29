//! Scenario loading: TOML → structs → a deterministic [`TerrainGrid`]. The only I/O the
//! engine performs. See `docs/DESIGN.md` §1.

use crate::fires::WeaponType;
use crate::sensing::{SensorType, UnitType};
use crate::terrain::{TerrainGrid, TerrainParamsTable, TerrainSource};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Everything that can go wrong loading a scenario. Typed so callers (experiments, the
/// app) can distinguish a missing file from a syntax error from a semantic one.
#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    /// The file could not be read.
    #[error("could not read {path}")]
    Io {
        /// The path we tried to read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The file was not valid TOML, or did not match the schema.
    #[error("invalid TOML: {0}")]
    Parse(#[from] toml::de::Error),
    /// The file parsed but is not a usable scenario (e.g. zero-sized terrain).
    #[error("invalid scenario: {0}")]
    Invalid(String),
}

/// A scenario: a named situation the engine can simulate — terrain, sim clock, and the
/// two forces' placed assets. Weapons join this schema in the fires phase.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Scenario {
    /// Human-readable scenario name.
    pub name: String,
    /// The seed used unless a run overrides it.
    #[serde(default)]
    pub default_seed: u64,
    /// Terrain grid definition.
    pub terrain: TerrainConfig,
    /// Sim clock configuration (`docs/DESIGN.md` §3.3); defaults if absent.
    #[serde(default)]
    pub sim: SimConfig,
    /// Blue force starting assets.
    #[serde(default)]
    pub blue: Force,
    /// Red force starting assets.
    #[serde(default)]
    pub red: Force,
}

/// Sim clock + suppression dials (`docs/DESIGN.md` §3.3, §4.3).
#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub struct SimConfig {
    /// Integration tick, seconds.
    #[serde(default = "default_dt_s")]
    pub dt_s: f32,
    /// Decision-epoch length, seconds.
    #[serde(default = "default_epoch_s")]
    pub epoch_s: f32,
    /// A round landing within this distance of a unit is a near-miss (suppresses), m.
    #[serde(default = "default_suppression_radius")]
    pub suppression_radius_m: f32,
    /// Probability a single near-miss steps a unit's suppression up one level.
    #[serde(default = "default_p_suppress")]
    pub p_suppress: f32,
    /// Suppression recovery rate (per second, one level down).
    #[serde(default = "default_recover_per_s")]
    pub recover_per_s: f32,
    /// Outgoing-fire effectiveness multiplier while Suppressed (`< 1`).
    #[serde(default = "default_suppressed_fire_factor")]
    pub suppressed_fire_factor: f32,
}

fn default_dt_s() -> f32 {
    1.0
}

fn default_epoch_s() -> f32 {
    10.0
}

fn default_suppression_radius() -> f32 {
    35.0
}

fn default_p_suppress() -> f32 {
    0.15
}

fn default_recover_per_s() -> f32 {
    0.05
}

fn default_suppressed_fire_factor() -> f32 {
    0.4
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            dt_s: default_dt_s(),
            epoch_s: default_epoch_s(),
            suppression_radius_m: default_suppression_radius(),
            p_suppress: default_p_suppress(),
            recover_per_s: default_recover_per_s(),
            suppressed_fire_factor: default_suppressed_fire_factor(),
        }
    }
}

/// One side's placed assets.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct Force {
    /// Placed sensors.
    #[serde(default)]
    pub sensors: Vec<SensorInstance>,
    /// Placed units.
    #[serde(default)]
    pub units: Vec<UnitInstance>,
    /// Placed jammers (protect this side's units from enemy detection).
    #[serde(default)]
    pub jammers: Vec<JammerInstance>,
}

/// A placed jammer (`docs/DESIGN.md` §8): position + degradation dials.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct JammerInstance {
    /// World position, metres `[x, y]`.
    pub pos: [f32; 2],
    /// Peak detection degradation at the centre, `[0, 1]`.
    pub power: f32,
    /// Effect radius, metres.
    pub radius_m: f32,
}

/// A placed sensor: a type id from `sensors.toml` plus position and facing.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SensorInstance {
    /// Unique-in-scenario id, shown in event feeds.
    pub id: String,
    /// Key into the sensor-type library.
    #[serde(rename = "type")]
    pub type_id: String,
    /// World position, metres `[x, y]`.
    pub pos: [f32; 2],
    /// Facing, degrees (0° = east, CCW); matters only with a finite field of regard.
    #[serde(default)]
    pub facing_deg: f32,
}

/// A placed unit: a type id from `units.toml` plus position and an optional route.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct UnitInstance {
    /// Unique-in-scenario id.
    pub id: String,
    /// Key into the unit-type library.
    #[serde(rename = "type")]
    pub type_id: String,
    /// World position, metres `[x, y]` (the route start if a route is given).
    pub pos: [f32; 2],
    /// Optional movement route as world waypoints; empty = static.
    #[serde(default)]
    pub route: Vec<[f32; 2]>,
}

/// The terrain block of a scenario: grid dimensions and how to generate the elevation.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TerrainConfig {
    /// Square cell size, metres.
    pub cell_size_m: f32,
    /// Cells east.
    pub width_cells: usize,
    /// Cells north.
    pub height_cells: usize,
    /// How to generate the elevation raster.
    pub source: TerrainSource,
}

impl Scenario {
    /// Load and validate a scenario from a TOML file.
    ///
    /// # Errors
    /// [`ScenarioError::Io`] if the file can't be read, [`ScenarioError::Parse`] if it
    /// isn't valid TOML / schema, [`ScenarioError::Invalid`] if it fails validation.
    pub fn load(path: &Path) -> Result<Self, ScenarioError> {
        let text = std::fs::read_to_string(path).map_err(|source| ScenarioError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_toml_str(&text)
    }

    /// Parse and validate a scenario from an in-memory TOML string (used by tests and
    /// any non-file source).
    ///
    /// # Errors
    /// As [`Scenario::load`], minus the I/O case.
    pub fn from_toml_str(text: &str) -> Result<Self, ScenarioError> {
        let scenario: Scenario = toml::from_str(text)?; // `?` maps toml::de::Error via #[from]
        scenario.validate()?;
        Ok(scenario)
    }

    fn validate(&self) -> Result<(), ScenarioError> {
        let t = &self.terrain;
        if t.width_cells == 0 || t.height_cells == 0 {
            return Err(ScenarioError::Invalid(
                "terrain dimensions must be non-zero".into(),
            ));
        }
        // Reject zero, negative, NaN, and infinity in one explicit test.
        if !t.cell_size_m.is_finite() || t.cell_size_m <= 0.0 {
            return Err(ScenarioError::Invalid(
                "cell_size_m must be positive and finite".into(),
            ));
        }
        Ok(())
    }

    /// Build this scenario's terrain with the given per-type dials and seed.
    ///
    /// Deterministic: same `(scenario, params, seed)` → bit-identical terrain.
    #[must_use]
    pub fn build_terrain(&self, params: &TerrainParamsTable, seed: u64) -> TerrainGrid {
        self.terrain.source.build(
            self.terrain.cell_size_m,
            self.terrain.width_cells,
            self.terrain.height_cells,
            seed,
            params,
        )
    }
}

/// Load the per-terrain-type dials (`scenarios/terrain_types.toml`).
///
/// # Errors
/// As [`Scenario::load`] (validation is structural — serde requires every field).
pub fn load_terrain_params(path: &Path) -> Result<TerrainParamsTable, ScenarioError> {
    let text = std::fs::read_to_string(path).map_err(|source| ScenarioError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(toml::from_str(&text)?)
}

/// Load the sensor-type library (`scenarios/sensors.toml`): a table of stat blocks
/// keyed by type id.
///
/// # Errors
/// As [`Scenario::load`].
pub fn load_sensor_types(path: &Path) -> Result<BTreeMap<String, SensorType>, ScenarioError> {
    let text = std::fs::read_to_string(path).map_err(|source| ScenarioError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(toml::from_str(&text)?)
}

/// Load the unit-type library (`scenarios/units.toml`).
///
/// # Errors
/// As [`Scenario::load`].
pub fn load_unit_types(path: &Path) -> Result<BTreeMap<String, UnitType>, ScenarioError> {
    let text = std::fs::read_to_string(path).map_err(|source| ScenarioError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(toml::from_str(&text)?)
}

/// Load the weapon-type library (`scenarios/weapons.toml`).
///
/// # Errors
/// As [`Scenario::load`].
pub fn load_weapon_types(path: &Path) -> Result<BTreeMap<String, WeaponType>, ScenarioError> {
    let text = std::fs::read_to_string(path).map_err(|source| ScenarioError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(toml::from_str(&text)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::TerrainType;

    /// Path to a file in the workspace `scenarios/` directory, resolved from this
    /// crate's manifest dir so tests don't depend on the working directory.
    fn scenario_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenarios")
            .join(name)
    }

    #[test]
    fn loads_default_scenario() {
        let scn = Scenario::load(&scenario_path("default.toml")).expect("default should load");
        assert_eq!(scn.name, "default");
        assert_eq!(scn.terrain.cell_size_m, 10.0);
        assert!(scn.terrain.width_cells > 0 && scn.terrain.height_cells > 0);
    }

    #[test]
    fn default_scenario_paints_all_terrain_types() {
        let scn = Scenario::load(&scenario_path("default.toml")).unwrap();
        let params = load_terrain_params(&scenario_path("terrain_types.toml")).unwrap();
        let g = scn.build_terrain(&params, scn.default_seed);
        let trees = g
            .terrain_type()
            .iter()
            .filter(|&&t| t == TerrainType::Trees)
            .count();
        let urban = g
            .terrain_type()
            .iter()
            .filter(|&&t| t == TerrainType::Urban)
            .count();
        // Diagnostic bounds, visible with `cargo test -- --nocapture`.
        let mut bounds = (usize::MAX, 0usize, usize::MAX, 0usize);
        for ((iy, ix), &t) in g.terrain_type().indexed_iter() {
            if t == TerrainType::Urban {
                bounds.0 = bounds.0.min(ix);
                bounds.1 = bounds.1.max(ix);
                bounds.2 = bounds.2.min(iy);
                bounds.3 = bounds.3.max(iy);
            }
        }
        println!(
            "urban cells: {urban} (ix {}..{}, iy {}..{})",
            bounds.0, bounds.1, bounds.2, bounds.3
        );
        assert!(trees > 0, "default scenario should paint woods");
        assert!(urban > 0, "default scenario should paint urban blocks");
    }

    #[test]
    fn loads_flat_fixture_scenario() {
        let scn = Scenario::load(&scenario_path("flat_range.toml")).expect("fixture should load");
        let params = load_terrain_params(&scenario_path("terrain_types.toml")).unwrap();
        let g = scn.build_terrain(&params, scn.default_seed);
        assert!(g.elevation().iter().all(|&z| z == 100.0));
    }

    #[test]
    fn loads_terrain_params() {
        let table =
            load_terrain_params(&scenario_path("terrain_types.toml")).expect("params should load");
        assert!(table.get(TerrainType::Trees).extinction_per_m > 0.0);
        assert_eq!(table.get(TerrainType::Open).mobility_cost, 1.0);
    }

    #[test]
    fn rejects_zero_dimensions() {
        let bad = r#"
            name = "bad"
            [terrain]
            cell_size_m = 10.0
            width_cells = 0
            height_cells = 10
            [terrain.source.flat]
            elevation_m = 0.0
        "#;
        assert!(matches!(
            Scenario::from_toml_str(bad),
            Err(ScenarioError::Invalid(_))
        ));
    }

    #[test]
    fn rejects_malformed_toml() {
        assert!(matches!(
            Scenario::from_toml_str("this is = not valid ]"),
            Err(ScenarioError::Parse(_))
        ));
    }

    #[test]
    fn missing_file_is_io_error() {
        let err = Scenario::load(Path::new("definitely-not-here-42.toml")).unwrap_err();
        assert!(matches!(err, ScenarioError::Io { .. }));
    }

    #[test]
    fn builds_terrain_deterministically_from_scenario() {
        let text = r#"
            name = "tiny"
            default_seed = 3
            [terrain]
            cell_size_m = 5.0
            width_cells = 32
            height_cells = 20
            [terrain.source.hills]
            count = 5
            max_height_m = 50.0
            base_radius_m = 40.0
        "#;
        let scn = Scenario::from_toml_str(text).expect("tiny scenario should parse");
        let params = load_terrain_params(&scenario_path("terrain_types.toml")).unwrap();

        let g1 = scn.build_terrain(&params, scn.default_seed);
        assert_eq!(g1.width(), 32);
        assert_eq!(g1.height(), 20);

        let g2 = scn.build_terrain(&params, scn.default_seed);
        assert_eq!(
            g1.elevation(),
            g2.elevation(),
            "same seed must reproduce terrain"
        );
    }
}
