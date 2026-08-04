//! The two map overlays, both computed from the same sensing model the simulation uses.
//!
//! - **Coverage** answers "what can this sensor see?" — per cell, `P(detect by T)`.
//! - **Belief** answers the harder question "given Blue has seen nothing, where could Red
//!   be?" — the §8.2 negative-information posterior, with Red's jamming folded in.
//!
//! Both are read-only views of `sim_core`; neither feeds anything back into the model.

use bevy::prelude::*;
use sim_core::sensing;
use sim_core::sim::Side;

use crate::state::{Overlay, PlacedSensor, SimRes};
use crate::terrain_view;

/// Pd coverage from the most recently placed Blue sensor against a reference target
/// (`afv` if defined): per cell, `P(detect by T) = 1 − e^{−λT}` painted as the overlay.
pub fn rebuild_coverage_overlay(
    sim: &SimRes,
    exposure_s: f32,
    overlay: &mut Overlay,
    commands: &mut Commands,
    images: &mut Assets<Image>,
) {
    // Most recently placed live Blue sensor. `sensor_active` matters now that a sensor
    // can be carried: a shot-down recce drone's sensor must not paint coverage.
    let Some((s_idx, sensor)) = sim
        .sim
        .sensors()
        .iter()
        .enumerate()
        .rev()
        .find(|(i, s)| s.side == Side::Blue && sim.sim.sensor_active(*i))
    else {
        return;
    };
    let reference = sim
        .data
        .libs
        .units
        .get("afv")
        .or_else(|| sim.data.libs.units.values().next())
        .expect("at least one unit type");

    let terrain = sim.sim.terrain();
    let t0 = std::time::Instant::now();
    let (h, w) = (terrain.height(), terrain.width());
    // Read the sensor's *effective* placement: for a carried sensor this is the
    // airframe's position, altitude and heading, not the ground mount height.
    let (s_pos, s_height, s_facing) = sim.sim.sensor_view(s_idx);
    let signature = reference.signature_in(sensor.stats.modality);
    let mut pd = ndarray::Array2::<f32>::zeros((h, w));
    ndarray::Zip::indexed(&mut pd).par_for_each(|(iy, ix), v| {
        let target = terrain.transform().cell_center(ix, iy);
        let lambda = sensing::detection_rate_against(
            terrain,
            &sensor.stats,
            s_pos,
            s_height,
            s_facing,
            target,
            reference.height_m,
            signature,
            sensing::concealment_at(terrain, target),
        );
        *v = 1.0 - (-lambda * exposure_s).exp();
    });
    info!(
        "coverage overlay for '{}' took {:?}",
        sensor.id,
        t0.elapsed()
    );

    let handle = images.add(terrain_view::viewshed_image(&pd));
    let cell = terrain.transform().cell_size_m();
    let (ex, ey) = (w as f32 * cell, h as f32 * cell);
    if let Some(e) = overlay.0.take() {
        commands.entity(e).despawn();
    }
    overlay.0 = Some(
        commands
            .spawn((
                Sprite::from_image(handle),
                Transform {
                    translation: Vec3::new(ex / 2.0, ey / 2.0, 1.0),
                    scale: Vec3::splat(cell),
                    ..default()
                },
            ))
            .id(),
    );
}

/// Belief overlay: assuming Blue has *not* detected Red, where could an `afv` be hiding?
/// The product of each Blue sensor's no-detection likelihood (including Red jamming),
/// normalised — mass concentrates in dead ground and inside Red's EW bubbles. This is the
/// Phase 8 partial-observability picture (POMDP negative information + EW).
pub fn rebuild_belief_overlay(
    sim: &SimRes,
    exposure_s: f32,
    overlay: &mut Overlay,
    commands: &mut Commands,
    images: &mut Assets<Image>,
) {
    // Live Blue sensors, each paired with its *effective* placement — a carried sensor
    // reports its airframe's position, altitude and heading (docs/DESIGN.md §9).
    let blue: Vec<PlacedSensor> = sim
        .sim
        .sensors()
        .iter()
        .enumerate()
        .filter(|(i, s)| s.side == Side::Blue && sim.sim.sensor_active(*i))
        .map(|(i, s)| (s, sim.sim.sensor_view(i)))
        .collect();
    if blue.is_empty() {
        return;
    }
    let reference = sim
        .data
        .libs
        .units
        .get("afv")
        .or_else(|| sim.data.libs.units.values().next())
        .expect("a unit type");
    let red_jammers: Vec<sim_core::ew::Jammer> = sim
        .sim
        .jammers()
        .iter()
        .filter(|j| j.side == Side::Red)
        .map(|j| j.jammer)
        .collect();

    let terrain = sim.sim.terrain();
    let (h, w) = (terrain.height(), terrain.width());
    let cell = terrain.transform().cell_size_m();

    // A moderate grid (parallel over cells): full-resolution over two long-range sensors
    // is ~10 s even threaded, so a light stride keeps the button interactive while still
    // being far crisper than before. Per cell: P(Red stays undetected for the exposure
    // window) = Π_sensors e^{−λ·T} — near 0 inside coverage, ~1 in dead ground/jamming.
    const STRIDE: usize = 3;
    let (cw, ch) = (w / STRIDE, h / STRIDE);
    let t0 = std::time::Instant::now();
    let mut belief = ndarray::Array2::<f32>::zeros((ch, cw));
    ndarray::Zip::indexed(&mut belief).par_for_each(|(cy, cx), v| {
        let ix = (cx * STRIDE + STRIDE / 2).min(w - 1);
        let iy = (cy * STRIDE + STRIDE / 2).min(h - 1);
        let pos = terrain.transform().cell_center(ix, iy);
        let concealment = sensing::concealment_at(terrain, pos);
        let mut p = 1.0f32;
        for (s, (s_pos, s_height, s_facing)) in &blue {
            let lambda = sensing::detection_rate_against(
                terrain,
                &s.stats,
                *s_pos,
                *s_height,
                *s_facing,
                pos,
                reference.height_m,
                reference.signature_in(s.stats.modality),
                concealment,
            ) * sim_core::ew::jamming_factor(pos, &red_jammers);
            p *= (-lambda * exposure_s).exp();
        }
        *v = p;
    });
    let sum = belief.sum();
    if sum > 0.0 {
        belief /= sum;
    }
    info!(
        "belief overlay ({} Blue sensors, {cw}x{ch}) took {:?}",
        blue.len(),
        t0.elapsed()
    );

    let handle = images.add(terrain_view::belief_image(&belief));
    let (ex, ey) = (w as f32 * cell, h as f32 * cell);
    if let Some(e) = overlay.0.take() {
        commands.entity(e).despawn();
    }
    overlay.0 = Some(
        commands
            .spawn((
                Sprite::from_image(handle),
                Transform {
                    translation: Vec3::new(ex / 2.0, ey / 2.0, 1.0),
                    scale: Vec3::splat(cell * STRIDE as f32),
                    ..default()
                },
            ))
            .id(),
    );
}

/// In screenshot mode, compute the belief overlay once (a few frames in) so the capture
/// shows the Phase 8 partial-observability picture.
pub fn screenshot_belief(
    sim: Res<SimRes>,
    mut overlay: ResMut<Overlay>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut frame: Local<u32>,
    mut done: Local<bool>,
) {
    *frame += 1;
    if !*done && *frame == 12 {
        *done = true;
        rebuild_belief_overlay(&sim, 60.0, &mut overlay, &mut commands, &mut images);
    }
}
