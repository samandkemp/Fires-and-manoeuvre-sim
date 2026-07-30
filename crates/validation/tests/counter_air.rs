//! V48-V51 - air-defence engagement, time-to-kill laws and the cueing timeline
//! (docs/DESIGN.md §9.4-§9.5).

use glam::Vec2;
use sim_core::air_defence::*;
use sim_core::sim::Side;
use sim_core::SimRng;
use std::collections::BTreeMap;
use validation::flat;

use rand::SeedableRng;

fn gun(rate: f32) -> AirDefenceType {
    AirDefenceType {
        engagement: AdEngagement::Gun {
            kill_rate_per_s: rate,
        },
        max_range_m: 4000.0,
        max_alt_m: 2000.0,
        requires_los: false,
        ..Default::default()
    }
}

fn missile(p: f32, speed: f32, reload: f32) -> AirDefenceType {
    AirDefenceType {
        engagement: AdEngagement::Missile {
            ssk_p: p,
            missile_speed_m_s: speed,
            reload_s: reload,
        },
        max_range_m: 20_000.0,
        max_alt_m: 8000.0,
        requires_los: false,
        ..Default::default()
    }
}

/// Drive one battery against a single target that is permanently in envelope and
/// actionable, returning the time to kill it (or `None` if it survived `limit_s`).
fn time_to_kill(
    stats: AirDefenceType,
    range_m: f32,
    dt_s: f32,
    seed: u64,
    limit_s: f64,
) -> Option<f64> {
    let mut ad = AirDefenceState::new("ad", Side::Blue, Vec2::ZERO, stats, true, None);
    let mut rng = SimRng::seed_from_u64(seed);
    let mut now = 0.0f64;
    let mut out = Vec::new();
    while now < limit_s {
        now += f64::from(dt_s);
        if !ad.engaging(0) && ad.can_open(now) {
            ad.open(0, now, range_m);
        }
        out.clear();
        ad.resolve_due(now, dt_s, &mut rng, &mut out);
        if out.iter().any(|&(_, killed)| killed) {
            return Some(now);
        }
    }
    None
}

// V48: a gun's time-to-kill is Exp(λ) — mean 1/λ, and P(kill by t) = 1 − e^{−λt}.
// Structurally the same gate as V14/V15 for detection, because it is the same law.
#[test]
fn v48_gun_time_to_kill_is_exponential() {
    let (rate, dt) = (0.25f32, 1.0f32);
    let trials = 2000u64;
    let t_check = 4.0f64;

    let mut times = Vec::new();
    let mut killed_by_t = 0u32;
    for seed in 0..trials {
        if let Some(t) = time_to_kill(gun(rate), 500.0, dt, seed, 400.0) {
            if t <= t_check {
                killed_by_t += 1;
            }
            times.push(t);
        }
    }
    assert_eq!(
        times.len() as u64,
        trials,
        "every target should die eventually"
    );

    // Mean TTK = 1/λ. Ticks discretise the exponential, biasing the mean up by dt/2.
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let expected = f64::from(expected_ttk_gun(rate)) + f64::from(dt) / 2.0;
    let se = f64::from(expected_ttk_gun(rate)) / (times.len() as f64).sqrt();
    assert!(
        (mean - expected).abs() < 4.0 * se,
        "mean TTK {mean:.3} vs 1/λ + dt/2 = {expected:.3} (se {se:.3})"
    );

    // P(kill by t) = 1 − e^{−λt}, within a binomial band.
    let p_exact = 1.0 - f64::from(p_leak_gun(rate, t_check as f32));
    let p_hat = f64::from(killed_by_t) / trials as f64;
    let sigma = (p_exact * (1.0 - p_exact) / trials as f64).sqrt();
    assert!(
        (p_hat - p_exact).abs() < 3.5 * sigma,
        "P(kill by {t_check}) = {p_hat:.4} vs closed form {p_exact:.4} (σ {sigma:.4})"
    );
}

// V49: a missile battery's shots are Bernoulli(p), shots-to-kill is Geometric(p), and
// the mean time-to-kill matches t_f/p + (1/p − 1)·t_r.
#[test]
fn v49_missile_time_to_kill_is_geometric() {
    let (p, speed, reload) = (0.4f32, 500.0f32, 6.0f32);
    let range = 2000.0f32;
    let dt = 0.5f32;
    let t_f = flight_time_s(range, speed); // 4 s
    assert_eq!(t_f, 4.0);

    let trials = 3000u64;
    let mut times = Vec::new();
    let mut shots_fired = 0u32;
    let mut hits = 0u32;
    for seed in 0..trials {
        // Count shots as well as time: the per-shot kill fraction must be `p`.
        let mut ad = AirDefenceState::new(
            "ad",
            Side::Blue,
            Vec2::ZERO,
            missile(p, speed, reload),
            true,
            None,
        );
        let mut rng = SimRng::seed_from_u64(10_000 + seed);
        let mut now = 0.0f64;
        let mut out = Vec::new();
        loop {
            now += f64::from(dt);
            if !ad.engaging(0) && ad.can_open(now) {
                ad.open(0, now, range);
                shots_fired += 1;
            }
            out.clear();
            ad.resolve_due(now, dt, &mut rng, &mut out);
            if let Some(&(_, killed)) = out.first() {
                if killed {
                    hits += 1;
                    times.push(now);
                    break;
                }
            }
            assert!(now < 2000.0, "the engagement should terminate");
        }
    }

    // Per-shot kill fraction = p, within a binomial CI.
    let p_hat = f64::from(hits) / f64::from(shots_fired);
    let sigma = (f64::from(p) * (1.0 - f64::from(p)) / f64::from(shots_fired)).sqrt();
    assert!(
        (p_hat - f64::from(p)).abs() < 4.0 * sigma,
        "per-shot kill fraction {p_hat:.4} vs ssk_p {p} (σ {sigma:.4})"
    );

    // E[shots] = 1/p (geometric).
    let mean_shots = f64::from(shots_fired) / trials as f64;
    assert!(
        (mean_shots - 1.0 / f64::from(p)).abs() < 0.08,
        "mean shots to kill {mean_shots:.3} vs 1/p = {:.3}",
        1.0 / f64::from(p)
    );

    // E[TTK] = t_f/p + (1/p − 1)·t_r, allowing the ≤ dt tick quantisation.
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let expected = f64::from(expected_ttk_missile(p, t_f, reload));
    assert!(
        (mean - expected).abs() < 0.6,
        "mean TTK {mean:.3} vs closed form {expected:.3}"
    );
}

// V50: cue latency governs leakage. The effective window shrinks one-for-one with
// latency, leakage rises monotonically, and above the critical latency L* = W − R
// every target leaks — regardless of how lethal the gun is.
#[test]
fn v50_cue_latency_and_leakage() {
    let (rate, window, reaction) = (0.5f32, 20.0f32, 2.0f32);
    let dt = 0.5f32;
    // No early warning: the target is acquired as it enters the envelope.
    let l_star = critical_latency_s(window, 0.0, reaction);
    assert_eq!(l_star, 18.0);

    let leak_fraction = |latency: f32| -> f64 {
        let w_eff = effective_window_s(window, 0.0, latency, reaction);
        let trials = 1500u64;
        let mut leaked = 0u32;
        for seed in 0..trials {
            // The target is in envelope for `window` s but only actionable for
            // `w_eff` of it — exactly the §9.5 timeline.
            match time_to_kill(gun(rate), 500.0, dt, seed, f64::from(w_eff)) {
                Some(_) => {}
                None => leaked += 1,
            }
        }
        f64::from(leaked) / trials as f64
    };

    let mut previous = 0.0f64;
    for latency in [0.0f32, 4.0, 8.0, 12.0, 16.0] {
        let observed = leak_fraction(latency);
        let w_eff = effective_window_s(window, 0.0, latency, reaction);
        let expected = f64::from(p_leak_gun(rate, w_eff));
        assert!(
            (observed - expected).abs() < 0.05,
            "latency {latency}: leakage {observed:.3} vs exp(−λ·W_eff) {expected:.3}"
        );
        assert!(
            observed >= previous - 0.02,
            "leakage must rise with cue latency ({previous:.3} → {observed:.3})"
        );
        previous = observed;
    }

    // At and beyond the critical latency the window is shut: everything leaks.
    assert_eq!(effective_window_s(window, 0.0, l_star, reaction), 0.0);
    assert_eq!(leak_fraction(l_star), 1.0);
    assert_eq!(leak_fraction(l_star + 10.0), 1.0);

    // Early warning buys the latency back, second for second: the clock starts at
    // *detection*, so a cue that has aged in flight costs nothing on arrival.
    for lead in [0.0f32, 5.0, 10.0, 30.0] {
        assert_eq!(
            critical_latency_s(window, lead, reaction),
            window + lead - reaction,
            "each second of warning raises the critical latency by a second"
        );
    }
    // A latency far past the no-warning critical value is harmless given the lead.
    let late = l_star + 25.0;
    assert_eq!(
        effective_window_s(window, late + reaction, late, reaction),
        window,
        "warning that outruns the whole chain leaves the window untouched"
    );
    // And it degrades gracefully in between: 10 s of overrun costs 10 s of window.
    assert_eq!(
        effective_window_s(window, 5.0, 13.0, reaction),
        window - 10.0
    );
    // With `D = 0` the general form must reproduce the familiar `W − L − R`.
    for latency in [0.0f32, 4.0, 9.0, 17.0] {
        assert_eq!(
            effective_window_s(window, 0.0, latency, reaction),
            (window - latency - reaction).max(0.0)
        );
    }

    // The missile form of the same law: shot opportunities fall to zero.
    assert_eq!(shot_opportunities(20.0, 4.0, 6.0), 2); // arrivals at 4 s and 14 s
    assert_eq!(shot_opportunities(3.0, 4.0, 6.0), 0); // first shot can't arrive
    assert!((p_leak_missile(0.5, 2) - 0.25).abs() < 1e-6);
}

// V51: envelope, cueing and magazine gating are exact — zero engagements outside the
// range band, outside the altitude band, without LOS, without a cue, or with an
// empty magazine; and concurrent engagements never exceed the channel count.
#[test]
fn v51_envelope_and_magazine_gating() {
    let g = flat(200, 200);
    let stats = AirDefenceType {
        engagement: AdEngagement::Gun {
            kill_rate_per_s: 1.0,
        },
        min_range_m: 200.0,
        max_range_m: 2000.0,
        min_alt_m: 50.0,
        max_alt_m: 1000.0,
        requires_los: true,
        channels: 2,
        magazine: 3,
        ..Default::default()
    };
    let ad_pos = Vec2::new(1000.0, 1000.0);

    // Inside every band: engageable.
    assert!(in_envelope(
        &stats,
        &g,
        ad_pos,
        Vec2::new(1800.0, 1000.0),
        400.0
    ));
    // Too close, too far.
    assert!(!in_envelope(
        &stats,
        &g,
        ad_pos,
        Vec2::new(1050.0, 1000.0),
        60.0
    ));
    assert!(!in_envelope(
        &stats,
        &g,
        ad_pos,
        Vec2::new(1000.0, 1000.0),
        3000.0
    ));
    // Below the band and above the ceiling.
    assert!(!in_envelope(
        &stats,
        &g,
        ad_pos,
        Vec2::new(1500.0, 1000.0),
        20.0
    ));
    assert!(!in_envelope(
        &stats,
        &g,
        ad_pos,
        Vec2::new(1500.0, 1000.0),
        1500.0
    ));

    // Cueing (§9.5): no track ⇒ never actionable; otherwise the battery acts on
    // whichever route reaches it first — its own radar, or the net plus the delay.
    let mut ad = AirDefenceState::new("ad", Side::Blue, ad_pos, stats.clone(), true, Some(7));
    ad.stats.cue_latency_s = 12.0;
    ad.stats.reaction_time_s = 3.0;
    assert_eq!(
        ad.actionable_at(None, None),
        None,
        "no track, no engagement"
    );
    assert_eq!(
        ad.actionable_at(Some(100.0), Some(100.0)),
        Some(103.0),
        "own radar ⇒ reaction time only"
    );
    assert_eq!(
        ad.actionable_at(Some(100.0), None),
        Some(115.0),
        "no organic acquisition ⇒ pay the comms latency"
    );
    // The case the old first-detection rule got wrong: another sensor detects first,
    // but this battery's own radar acquires shortly after. It must engage off its
    // radar rather than waiting out a comms hop it never needed.
    assert_eq!(
        ad.actionable_at(Some(100.0), Some(104.0)),
        Some(107.0),
        "own radar at t=104 beats the net cue landing at t=112"
    );
    assert_eq!(
        ad.actionable_at(Some(100.0), Some(130.0)),
        Some(115.0),
        "a late organic acquisition must not delay an already-arrived net cue"
    );

    // `own_sensor_seen` gates that route: switched off, no sensor, or not yet seen
    // all fall back to the network.
    let seen = BTreeMap::from([(7usize, 104.0f64), (2, 101.0)]);
    assert_eq!(
        ad.own_sensor_seen(&seen),
        Some(104.0),
        "reads its own sensor"
    );
    assert_eq!(
        ad.own_sensor_seen(&BTreeMap::from([(2usize, 101.0f64)])),
        None,
        "another sensor's acquisition is not organic"
    );
    ad.self_cue = false;
    assert_eq!(
        ad.own_sensor_seen(&seen),
        None,
        "self-cue off ⇒ no organic route, even though the radar exists"
    );
    assert_eq!(
        ad.actionable_at(Some(100.0), ad.own_sensor_seen(&seen)),
        Some(115.0),
        "so it pays the latency — the configuration for studying net-cued AD"
    );

    // Channels cap concurrency; the magazine caps total commitments.
    let mut ad = AirDefenceState::new("ad", Side::Blue, ad_pos, stats, true, None);
    assert!(ad.can_open(0.0));
    ad.open(0, 0.0, 1000.0);
    ad.open(1, 0.0, 1000.0);
    assert_eq!(ad.engagements.len(), 2);
    assert!(
        !ad.can_open(0.0),
        "a third engagement must wait for a free channel"
    );
    ad.drop_engagements(|t| t == 0); // target 1 left the envelope
    assert!(ad.can_open(0.0));
    ad.open(2, 0.0, 1000.0);
    assert_eq!(
        ad.magazine_left, 0,
        "3 commitments empties a 3-round magazine"
    );
    ad.drop_engagements(|_| false);
    assert!(!ad.can_open(0.0), "an empty magazine cannot engage");

    // An unlimited magazine (0) never runs dry.
    let unlimited = AirDefenceType {
        magazine: 0,
        ..gun(1.0)
    };
    let mut ad = AirDefenceState::new("ad", Side::Blue, ad_pos, unlimited, true, None);
    for i in 0..1000 {
        if ad.can_open(0.0) {
            ad.open(i, 0.0, 500.0);
            ad.drop_engagements(|_| false);
        }
    }
    assert!(ad.can_open(0.0), "magazine = 0 means unlimited");
}
