//! Reading scenarios and stat-block libraries off disk.
//!
//! Kept apart from the schema so that the types a caller matches on are not buried among
//! the file handling that produces them.
//!
//! Every `load_*` below returns [`ScenarioError::Io`] if the file cannot be read and
//! [`ScenarioError::Parse`] if it is not valid TOML for its type; validation is structural,
//! since serde requires every field.

use super::*;

/// Read a file, tagging the path onto any I/O error. Shared by every loader below.
pub(super) fn read_to_string(path: &Path) -> Result<String, ScenarioError> {
    std::fs::read_to_string(path).map_err(|source| ScenarioError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Parse a stat-block library - or the terrain-params table - from TOML **text**.
///
/// The string half of the `load_*` family below. Public so a caller already holding the
/// text need not write it back to disk: `sweep` patches a dial in memory and parses the
/// result, as `Scenario::from_toml_str` allows for a scenario.
///
/// # Errors
/// [`ScenarioError::Parse`] if the text is not valid TOML for `T`.
pub fn library_from_toml_str<T: serde::de::DeserializeOwned>(
    text: &str,
) -> Result<T, ScenarioError> {
    Ok(toml::from_str(text)?)
}

/// Load the per-terrain-type dials (`scenarios/terrain_types.toml`).
pub fn load_terrain_params(path: &Path) -> Result<TerrainParamsTable, ScenarioError> {
    library_from_toml_str(&read_to_string(path)?)
}

/// Load the sensor-type library (`scenarios/sensors.toml`): stat blocks keyed by type id.
pub fn load_sensor_types(path: &Path) -> Result<BTreeMap<String, SensorType>, ScenarioError> {
    library_from_toml_str(&read_to_string(path)?)
}

/// Load the unit-type library (`scenarios/units.toml`).
pub fn load_unit_types(path: &Path) -> Result<BTreeMap<String, UnitType>, ScenarioError> {
    library_from_toml_str(&read_to_string(path)?)
}

/// Load the weapon-type library (`scenarios/weapons.toml`).
pub fn load_weapon_types(path: &Path) -> Result<BTreeMap<String, WeaponType>, ScenarioError> {
    library_from_toml_str(&read_to_string(path)?)
}

/// Load the air-type library (`scenarios/air.toml`). Optional.
pub fn load_air_types(path: &Path) -> Result<BTreeMap<String, AirType>, ScenarioError> {
    library_from_toml_str(&read_to_string(path)?)
}

/// Load the C2 type library (`scenarios/c2.toml`). Optional: without it a scenario set has
/// no way to coordinate air defence.
pub fn load_c2_types(path: &Path) -> Result<BTreeMap<String, C2Type>, ScenarioError> {
    library_from_toml_str(&read_to_string(path)?)
}

/// Load the air-defence type library (`scenarios/air_defence.toml`). Optional.
pub fn load_air_defence_types(
    path: &Path,
) -> Result<BTreeMap<String, AirDefenceType>, ScenarioError> {
    library_from_toml_str(&read_to_string(path)?)
}

/// Every stat-block library a scenario resolves its instances against.
///
/// Bundled into one struct rather than passed as a growing list of positional maps:
/// [`crate::sim::Sim::new`] takes `(scenario, libraries, seed)` and stays that way as new
/// asset classes arrive.
#[derive(Debug, Clone)]
pub struct Libraries {
    /// Per-terrain-type dials.
    pub terrain_params: TerrainParamsTable,
    /// Sensor stat blocks (`sensors.toml`).
    pub sensors: BTreeMap<String, SensorType>,
    /// Unit stat blocks (`units.toml`).
    pub units: BTreeMap<String, UnitType>,
    /// Weapon stat blocks (`weapons.toml`).
    pub weapons: BTreeMap<String, WeaponType>,
    /// Air stat blocks (`air.toml`).
    pub air: BTreeMap<String, AirType>,
    /// Air-defence stat blocks (`air_defence.toml`).
    pub air_defence: BTreeMap<String, AirDefenceType>,
    /// C2 post stat blocks (`c2.toml`).
    pub c2: BTreeMap<String, C2Type>,
}

impl Libraries {
    /// Terrain dials with every stat-block library empty - the base for tests that
    /// supply only the libraries they exercise:
    /// `Libraries { units, ..Libraries::with_terrain(params) }`.
    #[must_use]
    pub fn with_terrain(terrain_params: TerrainParamsTable) -> Self {
        Self {
            terrain_params,
            sensors: BTreeMap::new(),
            units: BTreeMap::new(),
            weapons: BTreeMap::new(),
            air: BTreeMap::new(),
            air_defence: BTreeMap::new(),
            c2: BTreeMap::new(),
        }
    }

    /// Load every library from a `scenarios/`-shaped directory. The air and air-defence
    /// libraries are optional: a directory without them loads as empty maps, so
    /// older scenario sets still work.
    ///
    /// # Errors
    /// As [`Scenario::load`], for any library that exists but fails to parse, or whose
    /// dials fail [`Libraries::validate`].
    pub fn load_dir(dir: &Path) -> Result<Self, ScenarioError> {
        let libs = Self {
            terrain_params: load_terrain_params(&dir.join("terrain_types.toml"))?,
            sensors: load_sensor_types(&dir.join("sensors.toml"))?,
            units: load_unit_types(&dir.join("units.toml"))?,
            weapons: load_weapon_types(&dir.join("weapons.toml"))?,
            air: load_optional(&dir.join("air.toml"), load_air_types)?,
            air_defence: load_optional(&dir.join("air_defence.toml"), load_air_defence_types)?,
            c2: load_optional(&dir.join("c2.toml"), load_c2_types)?,
        };
        libs.validate()?;
        Ok(libs)
    }

    /// Refuse a stat block the models cannot evaluate (`docs/THEORY.md` §7.6).
    ///
    /// The list is **short** by design: most zeros are legitimate, such as a stationary
    /// drone or a battery that engages nothing. Only values reaching a **divisor** are
    /// refused, because those give `NaN` rather than a small answer, and `NaN` loses every
    /// comparison it appears in - the subsystem goes silently inert rather than visibly
    /// wrong.
    ///
    /// Called by [`Libraries::load_dir`] and again by [`crate::sim::Sim::new`], so a library
    /// patched in memory - what `sweep` does - is held to the same terms as one on disk.
    ///
    /// # Errors
    /// [`ScenarioError::Invalid`], naming the library, the stat block and the dial.
    pub fn validate(&self) -> Result<(), ScenarioError> {
        for (id, s) in &self.sensors {
            // Divides the §3.2 range falloff `1 / (1 + (r/range_half)^p)`. At zero the
            // rate is inf or NaN, and a NaN rate means `rng < p_detect` is always false:
            // the sensor never detects anything, and never says why.
            require_positive(&format!("sensors.{id}.range_half_m"), s.range_half_m)?;
        }
        for (id, w) in &self.weapons {
            // The §2.3 Carleton kernel divides by `2·R_L²`. A round landing exactly on the
            // target then computes 0/0, and the kill roll silently always fails.
            if w.class == crate::weapon_effects::WeaponClass::Indirect {
                require_positive(&format!("weapons.{id}.lethal_radius_m"), w.lethal_radius_m)?;
            }
        }
        Ok(())
    }
}

/// Load a library only if the file is there, else an empty map.
///
/// A generic `fn` rather than a closure: a closure would be monomorphised to the first
/// element type it is called with, so a second call for a different library would not
/// type-check.
fn load_optional<T>(
    path: &Path,
    load: fn(&Path) -> Result<BTreeMap<String, T>, ScenarioError>,
) -> Result<BTreeMap<String, T>, ScenarioError> {
    if path.exists() {
        load(path)
    } else {
        Ok(BTreeMap::new())
    }
}
