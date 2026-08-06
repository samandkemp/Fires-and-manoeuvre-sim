//! Everything drawn over the map as gizmos: force markers, routes, EW bubbles,
//! air-defence envelopes, and the line-of-sight probe.
//!
//! Marker sizes are multiplied by the camera's orthographic scale (`px` below) so they
//! stay legible at any zoom instead of shrinking to nothing on a 10 km map.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use sim_core::air::Terminal;
use sim_core::los;
use sim_core::sim::Side;
use sim_core::suppression::Suppression;

use crate::selection;
use crate::state::{Probe, SimRes, UiState, PROBE_HEIGHT_M};

/// Force markers: sensors as circles, units as diamonds; blue/red by side; a white
/// ring marks units the enemy has detected. Sizes scale with zoom.
pub fn draw_markers(
    sim: Res<SimRes>,
    ui_state: Res<UiState>,
    camera: Query<&Projection, With<Camera2d>>,
    mut gizmos: Gizmos,
) {
    let px = match camera.single() {
        Ok(Projection::Orthographic(o)) => o.scale,
        _ => 1.0,
    };

    // Selection highlight: one yellow ring per selected asset, whatever kind it is.
    // Driven off `selection::position` rather than a match here, so an asset class that
    // becomes selectable cannot end up selectable-but-invisible.
    for sel in &ui_state.selected {
        if let Some(p) = selection::position(&sim.sim, *sel) {
            gizmos.circle_2d(
                Isometry2d::from_translation(p),
                20.0 * px,
                Color::srgb(1.0, 0.9, 0.2),
            );
        }
    }
    let side_color = |side: Side| match side {
        Side::Blue => Color::srgb(0.25, 0.55, 1.0),
        Side::Red => Color::srgb(0.95, 0.30, 0.25),
    };

    // Routes: a faint line from each moving unit's current position through its
    // remaining waypoints, with a marker at each waypoint.
    for u in sim.sim.units() {
        if !u.alive() || u.route_idx >= u.route.len() {
            continue;
        }
        let c = side_color(u.side).with_alpha(0.5);
        let mut prev = u.pos;
        for &wp in &u.route[u.route_idx..] {
            gizmos.line_2d(prev, wp, c);
            gizmos.circle_2d(Isometry2d::from_translation(wp), 4.0 * px, c);
            prev = wp;
        }
    }

    // Jammers: a dashed-look bubble (two rings) in a magenta hue.
    for j in sim.sim.jammers() {
        let c = Color::srgb(0.85, 0.2, 0.9);
        gizmos.circle_2d(
            Isometry2d::from_translation(j.jammer.pos),
            j.jammer.radius_m,
            c.with_alpha(0.5),
        );
        gizmos.circle_2d(Isometry2d::from_translation(j.jammer.pos), 6.0 * px, c);
    }

    // C2 posts: the coordination radius, drawn under everything else because it is the
    // largest ring on the map and would otherwise bury the envelopes inside it. Every
    // battery within it allocates as one group (docs/DESIGN.md §11) — so this circle is
    // literally the boundary of who is cooperating with whom.
    for post in sim.sim.c2() {
        let alive = post.alive();
        let c = if alive {
            Color::srgb(0.95, 0.75, 0.2)
        } else {
            Color::srgb(0.45, 0.42, 0.35)
        };
        gizmos.circle_2d(
            Isometry2d::from_translation(post.pos),
            post.stats.coordination_range_m,
            c.with_alpha(if alive { 0.22 } else { 0.07 }),
        );
        if alive {
            // A small square, so a post reads as neither a shooter nor a sensor.
            let d = 7.0 * px;
            gizmos.rect_2d(
                Isometry2d::from_translation(post.pos),
                Vec2::splat(d * 2.0),
                c,
            );
            gizmos.circle_2d(Isometry2d::from_translation(post.pos), 3.0 * px, c);
        } else {
            cross(&mut gizmos, post.pos, 8.0 * px);
        }
    }

    // Air defence: the engagement envelope as a ring, plus a live line to whatever it is
    // currently engaging — the counter-air fight made visible.
    for ad in sim.sim.air_defence() {
        // A destroyed battery keeps its marker (greyed) but loses its envelope: the ring
        // means "this ground is covered", and once the battery is dead it is not.
        if !ad.alive() {
            cross(&mut gizmos, ad.pos, 9.0 * px);
            continue;
        }
        let c = Color::srgb(0.2, 0.85, 0.75);
        gizmos.circle_2d(
            Isometry2d::from_translation(ad.pos),
            ad.stats.max_range_m,
            c.with_alpha(0.30),
        );
        if ad.stats.min_range_m > 0.0 {
            gizmos.circle_2d(
                Isometry2d::from_translation(ad.pos),
                ad.stats.min_range_m,
                c.with_alpha(0.20),
            );
        }
        gizmos.circle_2d(Isometry2d::from_translation(ad.pos), 9.0 * px, c);
        gizmos.circle_2d(Isometry2d::from_translation(ad.pos), 5.0 * px, c);
        // Battle damage: a battery is N launchers, and losing some is not the same as
        // losing all of them (docs/DESIGN.md §12).
        if ad.elements < ad.stats.element_count {
            strength_bar(
                &mut gizmos,
                ad.pos,
                px,
                ad.elements as f32 / ad.stats.element_count.max(1) as f32,
            );
        }
        for e in &ad.engagements {
            if let Some(target) = sim.sim.air().get(e.target).filter(|a| a.alive) {
                gizmos.line_2d(ad.pos, target.pos, Color::srgb(1.0, 0.85, 0.2));
            }
        }
    }

    // Air assets: a triangle pointing along the heading, so course is readable at a
    // glance; an orbit plan draws its circle.
    for a in sim.sim.air() {
        let c = side_color(a.side);
        if !a.alive {
            cross(&mut gizmos, a.pos, 7.0 * px); // shot down
            continue;
        }
        // Remaining flight plan.
        let mut prev = a.pos;
        for &wp in a.plan.waypoints.iter().skip(a.route_idx) {
            gizmos.line_2d(prev, wp, c.with_alpha(0.4));
            prev = wp;
        }
        if let (Terminal::Orbit { radius_m, .. }, Some(centre)) =
            (a.plan.terminal, a.plan.destination())
        {
            gizmos.circle_2d(
                Isometry2d::from_translation(centre),
                radius_m,
                c.with_alpha(0.35),
            );
        }

        // Nose-forward triangle. Size grows a little with altitude so height reads on
        // the map without needing a label.
        let scale = (9.0 + a.altitude_m / 250.0) * px;
        let fwd = Vec2::from_angle(a.heading_deg.to_radians());
        let side = Vec2::new(-fwd.y, fwd.x);
        let nose = a.pos + fwd * scale * 1.6;
        let left = a.pos - fwd * scale * 0.7 + side * scale * 0.9;
        let right = a.pos - fwd * scale * 0.7 - side * scale * 0.9;
        gizmos.line_2d(nose, left, c);
        gizmos.line_2d(left, right, c);
        gizmos.line_2d(right, nose, c);

        if a.detected {
            gizmos.circle_2d(Isometry2d::from_translation(a.pos), 15.0 * px, Color::WHITE);
        }
    }

    for s in sim.sim.sensors() {
        // A carried sensor rides its airframe, which already has a marker.
        if s.carrier.is_some() {
            continue;
        }
        let c = side_color(s.side);
        gizmos.circle_2d(Isometry2d::from_translation(s.pos), 8.0 * px, c);
        gizmos.circle_2d(Isometry2d::from_translation(s.pos), 3.0 * px, c);

        // Field of regard: the wedge the sensor is actually watching. Only drawn for a
        // sensor that has one — an all-round sensor would just be a circle, and the
        // whole point of the wedge is to show what is *not* being watched. With
        // `[sim] sensor_tasking` on, this is where the belief-driven search is visible:
        // the wedges swing about between epochs (docs/DESIGN.md §10.3).
        if let Some(width) = s.stats.for_width_deg {
            let reach = s.stats.max_range_m;
            let facing = s.facing_deg.to_radians();
            let half = width.to_radians() * 0.5;
            let edge = |a: f32| s.pos + Vec2::from_angle(a) * reach;
            let faint = c.with_alpha(0.35);
            gizmos.line_2d(s.pos, edge(facing - half), faint);
            gizmos.line_2d(s.pos, edge(facing + half), faint);
            // The far arc, as a short polyline.
            const STEPS: usize = 12;
            for k in 0..STEPS {
                let t0 = k as f32 / STEPS as f32;
                let t1 = (k + 1) as f32 / STEPS as f32;
                let a0 = facing - half + 2.0 * half * t0;
                let a1 = facing - half + 2.0 * half * t1;
                gizmos.line_2d(edge(a0), edge(a1), faint);
            }
        }
    }
    for u in sim.sim.units() {
        if !u.alive() {
            cross(&mut gizmos, u.pos, 8.0 * px); // killed
            continue;
        }
        let c = side_color(u.side);
        gizmos.rect_2d(
            Isometry2d::new(u.pos, Rot2::degrees(45.0)),
            Vec2::splat(11.0 * px),
            c,
        );
        if u.detected {
            gizmos.circle_2d(Isometry2d::from_translation(u.pos), 13.0 * px, Color::WHITE);
        }
        // Suppression ring: amber = Suppressed, red = Pinned.
        match u.suppression {
            Suppression::Free => {}
            Suppression::Suppressed => {
                gizmos.circle_2d(
                    Isometry2d::from_translation(u.pos),
                    16.0 * px,
                    Color::srgb(0.95, 0.65, 0.1),
                );
            }
            Suppression::Pinned => {
                gizmos.circle_2d(
                    Isometry2d::from_translation(u.pos),
                    16.0 * px,
                    Color::srgb(0.95, 0.2, 0.15),
                );
            }
        }
        // A strength bar appears once the unit has lost an element.
        if u.strength() < 0.999 {
            strength_bar(&mut gizmos, u.pos, px, u.strength());
        }
    }
}

/// LOS probe line: observer → cursor (or demo target), coloured by the result.
pub fn draw_probe(
    sim: Res<SimRes>,
    mut probe: ResMut<Probe>,
    window: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform, &Projection), With<Camera2d>>,
    mut gizmos: Gizmos,
) {
    let Ok(window) = window.single() else { return };
    let Ok((cam, cam_tf, projection)) = camera.single() else {
        return;
    };
    let px = match projection {
        Projection::Orthographic(o) => o.scale,
        _ => 1.0,
    };

    let cursor_world = window
        .cursor_position()
        .and_then(|c| cam.viewport_to_world_2d(cam_tf, c).ok());

    let Some(obs) = probe.observer else { return };
    let Some(target) = cursor_world.or(probe.demo_target) else {
        return;
    };

    let r = los::line_of_sight(
        sim.sim.terrain(),
        obs,
        PROBE_HEIGHT_M,
        target,
        PROBE_HEIGHT_M,
    );

    let color = if !r.clear {
        Color::srgb(0.90, 0.20, 0.15)
    } else if r.transmittance < 0.5 {
        Color::srgb(0.95, 0.65, 0.10)
    } else {
        Color::srgb(0.15, 0.85, 0.25)
    };

    gizmos.line_2d(obs, target, color);
    gizmos.circle_2d(Isometry2d::from_translation(obs), 10.0 * px, Color::WHITE);
    gizmos.circle_2d(Isometry2d::from_translation(obs), 7.5 * px, color);
    if let Some(s) = r.blocked_at {
        let hit = obs + (target - obs).normalize_or_zero() * s;
        gizmos.circle_2d(
            Isometry2d::from_translation(hit),
            5.0 * px,
            Color::srgb(0.95, 0.2, 0.15),
        );
    }
    probe.last = Some(r);
}

/// The universal "this is destroyed" mark: a dim grey cross.
///
/// One helper rather than four copies — units, airframes, batteries and posts all die,
/// and a reader should not have to check whether they die *differently* on the map.
fn cross(gizmos: &mut Gizmos, pos: Vec2, half: f32) {
    let g = Color::srgb(0.45, 0.45, 0.45);
    gizmos.line_2d(
        pos + Vec2::new(-half, -half),
        pos + Vec2::new(half, half),
        g,
    );
    gizmos.line_2d(
        pos + Vec2::new(-half, half),
        pos + Vec2::new(half, -half),
        g,
    );
}

/// A remaining-strength bar under an asset, `fraction` in `[0, 1]`.
fn strength_bar(gizmos: &mut Gizmos, pos: Vec2, px: f32, fraction: f32) {
    let w = 24.0 * px;
    let y = pos.y - 16.0 * px;
    let left = pos.x - w / 2.0;
    gizmos.line_2d(
        Vec2::new(left, y),
        Vec2::new(left + w, y),
        Color::srgb(0.2, 0.2, 0.2),
    );
    gizmos.line_2d(
        Vec2::new(left, y),
        Vec2::new(left + w * fraction.clamp(0.0, 1.0), y),
        Color::srgb(0.2, 0.9, 0.3),
    );
}
