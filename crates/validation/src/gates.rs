//! The catalogue of validation gates: V-number → property → analytical reference → the
//! test that checks it.
//!
//! This is the machine-readable twin of the validation tables in `docs/DESIGN.md`. It
//! exists so `validation_report` can print what was checked and against what, rather than
//! a bare list of green test names — a gate is only meaningful alongside the closed form
//! it is compared to.
//!
//! It cannot silently drift: `tests/catalogue.rs` asserts that every gate here names a
//! test that exists, and that every `vNN_*` test in the suite appears here.

/// One validation gate, as stated in `docs/DESIGN.md`.
pub struct Gate {
    /// Gate number, e.g. `"V14"`.
    pub id: &'static str,
    /// The property being constrained.
    pub property: &'static str,
    /// The closed form or documented invariant it is checked against — the part that
    /// makes it validation rather than a regression test.
    pub reference: &'static str,
    /// The test function(s) that enforce it.
    pub tests: &'static [&'static str],
}

/// Every gate, in number order. `docs/DESIGN.md` is the prose source of truth; this is
/// the executable index into it.
pub const GATES: &[Gate] = &[
    Gate { id: "V1", property: "world<->cell round-trip", reference: "world_to_cell(cell_center(c)) == c for all cells", tests: &["world_cell_roundtrip"] },
    Gate { id: "V2", property: "bilinear exactness", reference: "sampling an affine field z = ax+by+c returns it exactly", tests: &["bilinear_reproduces_planes_exactly"] },
    Gate { id: "V3", property: "derived layers well-formed", reference: "cover, concealment in [0,1]; mobility >= 1; no NaN", tests: &["derived_layers_well_formed"] },
    Gate { id: "V4", property: "generation determinism", reference: "same seed -> bit-identical raster; different seed differs", tests: &["procedural_terrain_is_deterministic", "procedural_type_painting_fractions"] },
    Gate { id: "V5", property: "flat plane visibility", reference: "two actors with h>0 on flat open ground: clear, tau = 1", tests: &["v5_flat_plane_mutual_visibility"] },
    Gate { id: "V6", property: "single wall shadow", reference: "hidden zone and mask_height match the similar-triangles closed form", tests: &["v6_single_wall_closed_form"] },
    Gate { id: "V7", property: "LOS symmetry", reference: "los(a,b) == los(b,a) in clear and tau, on random terrain", tests: &["v7_symmetry"] },
    Gate { id: "V8", property: "LOS monotonicity", reference: "raising either endpoint never loses visibility; tau falls with canopy", tests: &["v8_monotonicity"] },
    Gate { id: "V9", property: "rigid-motion invariance", reference: "invariant under whole-scenario translation and 90-degree rotation", tests: &["v9_rigid_motion_invariance"] },
    Gate { id: "V10", property: "canopy extinction law", reference: "a Trees strip of width w crossed square-on gives tau = exp(-kw) exactly", tests: &["v10_canopy_extinction_law"] },
    Gate { id: "V11", property: "DDA vs fixed-step oracle", reference: "agrees with an independent fixed-step sampler within a step-driven tolerance", tests: &["v11_matches_fixed_step_oracle"] },
    Gate { id: "V12", property: "flat viewshed is a disc", reference: "on a flat plane the viewshed is exactly the in-range cell set", tests: &["v12_flat_viewshed_is_range_disc"] },
    Gate { id: "V13", property: "ridge shadow", reference: "per-column shadow matches the V6 wall closed form", tests: &["v13_wall_shadow_strip_closed_form"] },
    Gate { id: "V14", property: "detection-time distribution", reference: "MC mean detection time = 1/lambda within CI", tests: &["v14_v15_exponential_law_monte_carlo"] },
    Gate { id: "V15", property: "detection closed form", reference: "MC frequency by time t within binomial CI of 1 - e^(-lambda t)", tests: &["v14_v15_exponential_law_monte_carlo"] },
    Gate { id: "V16", property: "rate structure", reference: "lambda monotone in range/concealment, linear in signature, 0 when gated", tests: &["v16_rate_structure"] },
    Gate { id: "V17", property: "tick-size invariance", reference: "compounded per-tick survival equals e^(-lambda t) for any dt", tests: &["v17_tick_size_invariance"] },
    Gate { id: "V18", property: "sensing determinism", reference: "same (scenario, seed) -> identical event log; different seed differs", tests: &["v18_determinism"] },
    Gate { id: "V19", property: "direct-fire hit probability", reference: "MC impacts inside the WxH rectangle within CI of the erf product", tests: &["v19_direct_hit_probability_monte_carlo"] },
    Gate { id: "V20", property: "hit-probability monotonicity", reference: "falls with range and cover, rises with target size; 0 when blocked", tests: &["v20_direct_hit_monotonicity"] },
    Gate { id: "V21", property: "indirect CEP", reference: "empirical median miss distance = cep_m within CI (Rayleigh)", tests: &["v21_indirect_cep"] },
    Gate { id: "V22", property: "area-damage closed form", reference: "MC mean Carleton damage = R^2/(s^2+R^2) exp(-d^2/2(s^2+R^2))", tests: &["v22_area_damage_closed_form"] },
    Gate { id: "V23", property: "damage monotonicity", reference: "falls with offset and cover, rises with lethal radius", tests: &["v23_area_damage_monotonicity"] },
    Gate { id: "V24", property: "fires determinism", reference: "same (scenario, seed, mission) -> identical rounds and strengths", tests: &["v24_fires_attrit_and_are_deterministic"] },
    Gate { id: "V25", property: "zero-risk = shortest path", reference: "closed-form 8-connected distance (max-min) + sqrt(2)*min", tests: &["v25_zero_risk_is_shortest_path"] },
    Gate { id: "V26", property: "risk avoidance monotone", reference: "raising risk_weight never increases exposure along the optimum", tests: &["v26_risk_avoidance_monotone"] },
    Gate { id: "V27", property: "path optimality", reference: "Dijkstra cost matches an independent Bellman-Ford reference", tests: &["v27_matches_bellman_ford"] },
    Gate { id: "V28", property: "suppression stationary distribution", reference: "birth-death chain occupancy pi_k proportional to (beta/mu)^k", tests: &["v28_stationary_distribution"] },
    Gate { id: "V29", property: "recovery time", reference: "mean time Pinned->Free = 2/recover_per_s (two exponential steps)", tests: &["v29_recovery_time"] },
    Gate { id: "V30", property: "Lanchester square law", reference: "aimed-fire duel conserves A^2 - B^2 in the mean", tests: &["v30_lanchester_square_law"] },
    Gate { id: "V31", property: "suppression gates fire", reference: "Pinned emits nothing; Suppressed output = factor x Free output", tests: &["v31_suppression_gates_fire"] },
    Gate { id: "V32", property: "matching pennies", reference: "fictitious play value -> 0, both strategies -> (1/2, 1/2)", tests: &["v32_matching_pennies"] },
    Gate { id: "V33", property: "rock-paper-scissors", reference: "value -> 0, both strategies -> uniform", tests: &["v33_rock_paper_scissors"] },
    Gate { id: "V34", property: "saddle point", reference: "a game with a pure equilibrium converges to that value", tests: &["v34_saddle_point"] },
    Gate { id: "V35", property: "strict dominance", reference: "a strictly dominated strategy converges to ~0 weight", tests: &["v35_strict_dominance"] },
    Gate { id: "V36", property: "skew-symmetric fairness", reference: "A = -A^T implies value 0; the value bracket closes", tests: &["v36_skew_symmetric"] },
    Gate { id: "V37", property: "route following", reference: "a unit on a straight route is at speed*t after t seconds", tests: &["v37_route_following"] },
    Gate { id: "V38", property: "pinned unit halts", reference: "a Pinned unit does not advance along its route", tests: &["v38_pinned_unit_halts"] },
    Gate { id: "V39", property: "interdiction sanity", reference: "an unwatched route is safe, so Red weights it and the value falls", tests: &["v39_interdiction_safe_route"] },
    Gate { id: "V40", property: "EW modifier", reference: "no jammers => factor exactly 1 (EW-off is the identity); jamming cuts detection monotonically", tests: &["v40_no_jammers_is_identity", "v40_ew_degrades_and_off_is_identity"] },
    Gate { id: "V41", property: "Tiger problem", reference: "exact Bayes posteriors: 0.85 after one observation, 0.9698 after two", tests: &["v41_tiger_problem"] },
    Gate { id: "V42", property: "belief well-formed", reference: "stays a normalised distribution; a peaked likelihood lowers entropy", tests: &["v42_belief_is_proper_and_concentrates"] },
    Gate { id: "V43", property: "negative information", reference: "repeated non-detection shifts belief into dead ground; motion raises entropy", tests: &["v43_negative_information_and_diffusion"] },
    Gate { id: "V44", property: "altitude and masking", reference: "an AMSL drone below a crest is masked where the same drone at AGL is not", tests: &["v44_altitude_and_masking"] },
    Gate { id: "V45", property: "slant range", reference: "sqrt(horizontal^2 + dz^2) exactly; reduces to horizontal when dz = 0", tests: &["v45_slant_range"] },
    Gate { id: "V46", property: "orbit kinematics", reference: "radius holds to epsilon; a lap closes in 2*pi*R/v", tests: &["v46_orbit_kinematics"] },
    Gate { id: "V47", property: "transit and turn rate", reference: "straight leg = speed*t; a turn's chord = 2R sin(phi/2) at R = v/omega", tests: &["v47_transit_and_turn_rate"] },
    Gate { id: "V48", property: "gun time-to-kill", reference: "TTK ~ Exp(lambda): mean 1/lambda, P(kill by t) = 1 - e^(-lambda t)", tests: &["v48_gun_time_to_kill_is_exponential"] },
    Gate { id: "V49", property: "missile time-to-kill", reference: "shots ~ Geometric(p); E[TTK] = t_f/p + (1/p - 1) t_r", tests: &["v49_missile_time_to_kill_is_geometric"] },
    Gate { id: "V50", property: "cue latency and leakage", reference: "leakage = exp(-lambda W_eff); critical latency L* = W + D - R", tests: &["v50_cue_latency_and_leakage"] },
    Gate { id: "V51", property: "envelope and magazine gating", reference: "exactly zero engagements outside band/LOS/cue/magazine; channels capped", tests: &["v51_envelope_and_magazine_gating"] },
    Gate { id: "V52", property: "air-off identity and determinism", reference: "empty air phases draw no randomness (log bit-identical); same seed reproduces", tests: &["v52_air_off_is_a_zero_draw_identity", "v52_air_determinism"] },
    Gate { id: "V53", property: "terrain recipes and presets", reference: "recipe+seed reproduces bit-identically; each layer meets its own invariant (woodland fraction, ridge crest lift); layer order is significant; presets differ as their names claim", tests: &["v53_recipes_are_deterministic_and_layers_do_what_they_say", "v53_presets_expand_to_the_maps_they_name"] },
    Gate { id: "V54", property: "asset removal preserves history", reference: "removal tombstones rather than shifting: every index already in an event log still resolves to the same asset", tests: &["v54_removal_tombstones_keep_logged_indices_valid"] },
];
