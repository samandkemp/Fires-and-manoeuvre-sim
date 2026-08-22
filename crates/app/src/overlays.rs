//! The two map overlays, both computed from the same sensing model the simulation uses.
//!
//! - **Coverage** answers "what can this sensor see?" - per cell, `P(detect by T)`.
//! - **Belief** answers the harder question "given Blue has seen nothing, where could Red
//!   be?" - the §8.2 negative-information posterior, with Red's jamming folded in.
//!
//! Both are read-only views of `sim_core`; neither feeds anything back into the model.

use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future::poll_once, AsyncComputeTaskPool};
use sim_core::sensing;
use sim_core::terrain::TerrainGrid;
use std::sync::Arc;

use crate::state::{
    overlay_fingerprint, Overlay, OverlayKind, OverlayRaster, OverlayRequest, SimRes,
};
use crate::terrain_view;

/// Stride the raster is sampled at.
///
/// A full-resolution pass over a 1000x1000 map is seconds even threaded, because every cell
/// asks every sensor for a detection rate and each of those walks a sightline. A third in
/// each direction is a ninth of the work and still far finer than the decision it informs.
const STRIDE: usize = 3;

/// Shortest interval between automatic rebuilds, real seconds.
///
/// The overlay depends on what has been detected as well as where things are, so a running
/// battle changes its inputs several times a second. Rebuilding that often would spin a core
/// continuously to produce a picture changing far faster than anyone could read it.
const AUTO_REFRESH_S: f32 = 2.0;

/// Ask for an overlay. Returns immediately; the raster is computed on a background task.
///
/// The inputs are **snapshotted here**, on the main thread, while the simulation is
/// borrowable - the task then owns everything it needs. Terrain is shared rather than
/// copied, which is sound because terrain does not change during a run.
pub fn request_overlay(sim: &SimRes, request: OverlayRequest, overlay: &mut Overlay) {
    // Cache the terrain on first use. One clone per scenario, not per overlay.
    if overlay.terrain.is_none() {
        overlay.terrain = Some(Arc::new(sim.sim.terrain().clone()));
    }
    let Some(terrain) = overlay.terrain.clone() else {
        return;
    };
    let fingerprint = overlay_fingerprint(&sim.sim);

    // Snapshot the sensors this overlay is computed from, each with its *effective*
    // placement - a carried sensor reports its airframe's position, height and heading.
    let sensors: Vec<OwnedSensor> = sim
        .sim
        .sensors()
        .iter()
        .enumerate()
        .filter(|(i, s)| s.side == request.side && sim.sim.sensor_active(*i))
        .map(|(i, s)| {
            let (pos, height, facing) = sim.sim.sensor_view(i);
            OwnedSensor {
                stats: s.stats.clone(),
                pos,
                height,
                facing,
            }
        })
        .collect();

    // Coverage reads only the most recently placed sensor; belief pools them all.
    let sensors = match request.kind {
        OverlayKind::Coverage => sensors.into_iter().next_back().into_iter().collect(),
        _ => sensors,
    };
    if sensors.is_empty() && request.kind != OverlayKind::SimBelief {
        return;
    }

    // The enemy's jammers degrade what these sensors achieve (§8.1).
    let jammers: Vec<sim_core::ew::Jammer> = sim
        .sim
        .jammers()
        .iter()
        .filter(|j| j.side != request.side)
        .map(|j| j.jammer)
        .collect();

    let reference = sim
        .data
        .libs
        .units
        .get("afv")
        .or_else(|| sim.data.libs.units.values().next())
        .cloned();
    let Some(reference) = reference else {
        return;
    };

    // The sim's own filter is already a coarse raster; it is read here rather than
    // recomputed, so it needs no background work at all.
    if request.kind == OverlayKind::SimBelief {
        let cells = sim.sim.belief_of(request.side).belief().clone();
        overlay.job = None;
        overlay.pending = None;
        overlay.ready = Some(OverlayRaster {
            cells,
            request,
            fingerprint,
            took: std::time::Duration::ZERO,
        });
        return;
    }

    let pool = AsyncComputeTaskPool::get();
    overlay.pending = Some(request);
    overlay.job = Some(pool.spawn(async move {
        let t0 = std::time::Instant::now();
        let cells = compute_raster(&terrain, &sensors, &jammers, &reference, request);
        OverlayRaster {
            cells,
            request,
            fingerprint,
            took: t0.elapsed(),
        }
    }));
}

/// A sensor and its effective placement, owned so a background task can hold it.
struct OwnedSensor {
    stats: sim_core::sensing::SensorType,
    pos: Vec2,
    height: f32,
    facing: f32,
}

/// The raster itself. Pure: terrain in, values out, no access to the simulation.
///
/// This is the expensive part, and the reason it is worth moving off the main thread. It is
/// still parallel across cells - the task pool and rayon compose, because the pool is
/// running one task that internally forks.
fn compute_raster(
    terrain: &TerrainGrid,
    sensors: &[OwnedSensor],
    jammers: &[sim_core::ew::Jammer],
    reference: &sim_core::sensing::UnitType,
    request: OverlayRequest,
) -> ndarray::Array2<f32> {
    let (h, w) = (terrain.height(), terrain.width());
    let (cw, ch) = (w / STRIDE, h / STRIDE);
    let mut out = ndarray::Array2::<f32>::zeros((ch, cw));

    ndarray::Zip::indexed(&mut out).par_for_each(|(cy, cx), v| {
        let ix = (cx * STRIDE + STRIDE / 2).min(w - 1);
        let iy = (cy * STRIDE + STRIDE / 2).min(h - 1);
        let pos = terrain.transform().cell_center(ix, iy);
        let concealment = sensing::concealment_at(terrain, pos);

        // P(nothing detects a mover here within the exposure window) = product over
        // sensors of e^{-lambda T}. Coverage shows its complement; belief shows it
        // directly, because "where could the enemy still be" IS the undetected case.
        let mut undetected = 1.0f32;
        for s in sensors {
            let lambda = sensing::detection_rate_against(
                terrain,
                &s.stats,
                s.pos,
                s.height,
                s.facing,
                pos,
                reference.height_m,
                reference.signature_in(s.stats.modality),
                concealment,
            ) * sim_core::ew::jamming_factor(pos, jammers);
            undetected *= (-lambda * request.exposure_s).exp();
        }
        *v = match request.kind {
            OverlayKind::Coverage => 1.0 - undetected,
            _ => undetected,
        };
    });

    // A belief is a distribution, so it is normalised to integrate to one. Coverage is
    // already a probability per cell and must NOT be rescaled: doing so would make a weak
    // sensor paint like a strong one.
    if request.kind == OverlayKind::BeliefSnapshot {
        let sum = out.sum();
        if sum > 0.0 {
            out /= sum;
        }
    }
    out
}

/// Poll the background job, and install a finished raster as the map sprite.
///
/// Also decides whether what is on screen still describes the simulation: if the assets have
/// moved since it was built, the overlay is stale, and with `auto` on it is rebuilt.
pub fn drive_overlays(
    sim: Res<SimRes>,
    time: Res<Time>,
    mut overlay: ResMut<Overlay>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
) {
    // A job that has finished hands back its raster.
    if let Some(task) = overlay.job.as_mut() {
        if let Some(done) = block_on(poll_once(task)) {
            overlay.job = None;
            overlay.pending = None;
            overlay.ready = Some(done);
        }
    }

    if let Some(done) = overlay.ready.take() {
        install(&done, &sim, &mut overlay, &mut commands, &mut images);
    }

    // Auto-refresh: only when something the overlay depends on has actually changed, never
    // while a job is in flight, and no more often than the throttle allows.
    overlay.since_auto_s += time.delta_secs();
    if overlay.auto && overlay.job.is_none() && overlay.since_auto_s >= AUTO_REFRESH_S {
        if let Some(showing) = overlay.showing {
            if overlay_fingerprint(&sim.sim) != overlay.built_from {
                overlay.since_auto_s = 0.0;
                request_overlay(&sim, showing, &mut overlay);
            }
        }
    }
}

/// Paint a finished raster and replace whatever sprite was there.
fn install(
    done: &OverlayRaster,
    sim: &SimRes,
    overlay: &mut Overlay,
    commands: &mut Commands,
    images: &mut Assets<Image>,
) {
    let terrain = sim.sim.terrain();
    let cell = terrain.transform().cell_size_m();
    let (w, h) = (terrain.width(), terrain.height());
    let (ex, ey) = (w as f32 * cell, h as f32 * cell);

    let image = match done.request.kind {
        OverlayKind::Coverage => terrain_view::viewshed_image(&done.cells),
        _ => terrain_view::belief_image(&done.cells),
    };
    let handle = images.add(image);

    if let Some(e) = overlay.sprite.take() {
        commands.entity(e).despawn();
    }
    // The sprite is stretched to the map: the raster is one cell per STRIDE terrain cells
    // for the computed overlays, and the sim's own belief grid is coarser again, so the
    // scale is derived from the raster's own width rather than assumed.
    let scale = ex / done.cells.dim().1 as f32;
    overlay.sprite = Some(
        commands
            .spawn((
                Sprite::from_image(handle),
                Transform {
                    translation: Vec3::new(ex / 2.0, ey / 2.0, 1.0),
                    scale: Vec3::splat(scale),
                    ..default()
                },
            ))
            .id(),
    );
    overlay.showing = Some(done.request);
    overlay.built_from = done.fingerprint;
    info!(
        "{} overlay ({}x{}) took {:?}",
        done.request.kind.label(),
        done.cells.dim().1,
        done.cells.dim().0,
        done.took
    );
}

/// Remove whatever overlay is on screen and forget what it was.
pub fn clear_overlay(overlay: &mut Overlay, commands: &mut Commands) {
    if let Some(e) = overlay.sprite.take() {
        commands.entity(e).despawn();
    }
    overlay.showing = None;
    overlay.pending = None;
    overlay.job = None;
    overlay.ready = None;
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
        request_overlay(
            &sim,
            OverlayRequest {
                kind: OverlayKind::BeliefSnapshot,
                side: sim_core::sim::Side::Blue,
                exposure_s: 60.0,
            },
            &mut overlay,
        );
    }
    // The capture is frame-locked, so the raster has to be waited for rather than polled
    // across frames: this is the one place a blocking wait is the correct behaviour.
    if let Some(task) = overlay.job.take() {
        overlay.ready = Some(block_on(task));
    }
    if let Some(finished) = overlay.ready.take() {
        install(&finished, &sim, &mut overlay, &mut commands, &mut images);
    }
}
