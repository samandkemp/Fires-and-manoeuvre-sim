//! Turning a headless [`TerrainGrid`] into something on screen: find and load
//! scenarios, and rasterise the elevation into a hypsometric-tint × hillshade texture.
//! This is the only place the app interprets terrain visually; the maths stays in
//! `sim_core`.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use ndarray::Array2;
use sim_core::scenario::{Libraries, Scenario, ScenarioError};
use sim_core::terrain::{TerrainGrid, TerrainType};
use std::path::{Path, PathBuf};

/// Everything the app loads from `scenarios/` at startup.
pub struct LoadedData {
    /// The scenario currently loaded.
    pub scenario: Scenario,
    /// Every stat-block library the scenario resolves against.
    pub libs: Libraries,
    /// Bare name of the loaded scenario (`"air_raid"`), for the title bar and picker.
    pub scenario_name: String,
    /// Every scenario found in `scenarios/`, for the in-app picker.
    pub available: Vec<String>,
}

/// The workspace `scenarios/` directory, resolved from this crate's manifest dir so the
/// working directory doesn't matter.
#[must_use]
pub fn scenarios_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios")
}

/// Load a scenario by bare name (`"air_raid"`, resolved inside `scenarios/`) or by path,
/// together with all stat-block libraries.
///
/// # Errors
/// If the scenario or any library fails to read or parse.
pub fn load_scenario(name: &str) -> Result<LoadedData, ScenarioError> {
    let dir = scenarios_dir();
    // A name with a separator or a `.toml` suffix is a path the caller means literally;
    // anything else is a scenario living in `scenarios/`.
    let path = if name.ends_with(".toml") || name.contains('/') || name.contains('\\') {
        PathBuf::from(name)
    } else {
        dir.join(format!("{name}.toml"))
    };
    let scenario_name = path
        .file_stem()
        .map_or_else(|| name.to_owned(), |s| s.to_string_lossy().into_owned());
    Ok(LoadedData {
        scenario: Scenario::load(&path)?,
        libs: Libraries::load_dir(&dir)?,
        scenario_name,
        available: list_scenarios(),
    })
}

/// Every scenario in `scenarios/`, by bare name, sorted.
///
/// The directory mixes scenarios with stat-block libraries (`units.toml` and friends),
/// and rather than hard-code which names are which — a list that would rot the moment a
/// library is added — a file counts as a scenario **if it parses as one**. `Scenario`
/// requires a `name` and a `[terrain]` block, which no library has.
#[must_use]
pub fn list_scenarios() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(scenarios_dir()) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .filter(|p| Scenario::load(p).is_ok())
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    names.sort();
    names
}

/// Rasterise a terrain grid into an RGBA image: a hypsometric colour ramp keyed on
/// normalised elevation, shaded by a hillshade so relief reads at a glance.
///
/// The image's top row is the northernmost grid row (`iy = height − 1`), so that when
/// the sprite is placed with +Y up, north is up — matching the world frame.
pub fn terrain_image(terrain: &TerrainGrid) -> Image {
    let w = terrain.width();
    let h = terrain.height();
    let elev = terrain.elevation();
    let cell_size_m = terrain.transform().cell_size_m();

    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for &z in elev.iter() {
        lo = lo.min(z);
        hi = hi.max(z);
    }
    let range = (hi - lo).max(1e-3);

    let mut data = vec![0u8; w * h * 4];
    for r in 0..h {
        let iy = h - 1 - r; // flip rows: texture row 0 = north
        for ix in 0..w {
            let t = (elev[[iy, ix]] - lo) / range;
            let base = hypsometric(t);
            let shade = hillshade(elev, ix, iy, w, h, cell_size_m);
            // Terrain type tints the elevation colour so cover/concealment terrain is
            // legible at a glance: woods = deeper green, urban = concrete grey.
            let tinted = match terrain.terrain_type()[[iy, ix]] {
                TerrainType::Open => base,
                TerrainType::Trees => [base[0] * 0.45, base[1] * 0.75, base[2] * 0.45],
                TerrainType::Urban => [
                    0.15 * base[0] + 0.47,
                    0.15 * base[1] + 0.46,
                    0.15 * base[2] + 0.50,
                ],
            };

            let px = (r * w + ix) * 4;
            data[px] = to_srgb_byte(tinted[0] * shade);
            data[px + 1] = to_srgb_byte(tinted[1] * shade);
            data[px + 2] = to_srgb_byte(tinted[2] * shade);
            data[px + 3] = 255;
        }
    }

    Image::new(
        Extent3d {
            width: w as u32,
            height: h as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
}

/// A topographic colour ramp: green lowlands → khaki → tan → brown → white peaks.
/// `t` is normalised elevation in `[0, 1]`. Returns linear RGB in `[0, 1]`.
fn hypsometric(t: f32) -> [f32; 3] {
    const STOPS: [(f32, [f32; 3]); 5] = [
        (0.00, [0.20, 0.35, 0.20]),
        (0.35, [0.45, 0.55, 0.30]),
        (0.60, [0.65, 0.60, 0.40]),
        (0.80, [0.55, 0.45, 0.35]),
        (1.00, [0.96, 0.96, 0.96]),
    ];
    let t = t.clamp(0.0, 1.0);
    for pair in STOPS.windows(2) {
        let (t0, c0) = pair[0];
        let (t1, c1) = pair[1];
        if t <= t1 {
            let f = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
            return [
                c0[0] + (c1[0] - c0[0]) * f,
                c0[1] + (c1[1] - c0[1]) * f,
                c0[2] + (c1[2] - c0[2]) * f,
            ];
        }
    }
    STOPS[STOPS.len() - 1].1
}

/// Lambertian hillshade from the local elevation gradient, lit from the north-west and
/// above. Returns a brightness in `[0.35, 1.0]` (floored so shadows aren't pure black).
fn hillshade(elev: &Array2<f32>, ix: usize, iy: usize, w: usize, h: usize, s: f32) -> f32 {
    let xm = ix.saturating_sub(1);
    let xp = (ix + 1).min(w - 1);
    let ym = iy.saturating_sub(1);
    let yp = (iy + 1).min(h - 1);

    // Vertical exaggeration for the *shading only* (elevation data is untouched): gentle
    // tactical relief would otherwise barely register. Purely cosmetic.
    const VERTICAL_EXAGGERATION: f32 = 5.0;
    let dzdx =
        VERTICAL_EXAGGERATION * (elev[[iy, xp]] - elev[[iy, xm]]) / (((xp - xm).max(1)) as f32 * s);
    let dzdy =
        VERTICAL_EXAGGERATION * (elev[[yp, ix]] - elev[[ym, ix]]) / (((yp - ym).max(1)) as f32 * s);

    // Surface normal ∝ (−dz/dx, −dz/dy, 1); light points from NW, high in the sky.
    let normal = Vec3::new(-dzdx, -dzdy, 1.0).normalize();
    let light = Vec3::new(-1.0, 1.0, 2.0).normalize();
    let d = normal.dot(light).max(0.0);
    0.35 + 0.65 * d
}

/// Rasterise a viewshed (per-cell transmittance τ) into a translucent overlay: seen
/// cells glow cyan-green with opacity scaled by τ, unseen cells stay transparent.
/// Row order matches [`terrain_image`] (top row = north).
pub fn viewshed_image(vs: &Array2<f32>) -> Image {
    let (h, w) = vs.dim();
    let mut data = vec![0u8; w * h * 4];
    for r in 0..h {
        let iy = h - 1 - r;
        for ix in 0..w {
            let tau = vs[[iy, ix]];
            if tau > 0.0 {
                let px = (r * w + ix) * 4;
                data[px] = 40;
                data[px + 1] = 220;
                data[px + 2] = 190;
                data[px + 3] = (40.0 + 90.0 * tau) as u8;
            }
        }
    }
    Image::new(
        Extent3d {
            width: w as u32,
            height: h as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
}

/// Rasterise a belief distribution (probability per cell) into a translucent magenta
/// heatmap, scaled to the belief's own maximum. Row order matches [`terrain_image`].
pub fn belief_image(belief: &Array2<f32>) -> Image {
    let (h, w) = belief.dim();
    let max = belief.iter().copied().fold(0.0f32, f32::max).max(1e-12);
    let mut data = vec![0u8; w * h * 4];
    for r in 0..h {
        let iy = h - 1 - r;
        for ix in 0..w {
            let v = (belief[[iy, ix]] / max).clamp(0.0, 1.0);
            if v > 0.02 {
                let px = (r * w + ix) * 4;
                data[px] = (40.0 + 180.0 * v) as u8;
                data[px + 1] = (25.0 * v) as u8;
                data[px + 2] = (60.0 + 180.0 * v) as u8;
                data[px + 3] = (30.0 + 130.0 * v) as u8;
            }
        }
    }
    Image::new(
        Extent3d {
            width: w as u32,
            height: h as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
}

/// Encode a linear `[0, 1]` channel to an sRGB byte.
fn to_srgb_byte(linear: f32) -> u8 {
    let l = linear.clamp(0.0, 1.0);
    let srgb = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0).round().clamp(0.0, 255.0) as u8
}
