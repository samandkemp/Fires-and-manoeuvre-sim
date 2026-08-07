[Index](README.md) · [← §2 Fires](02-fires.md) · [§4 Suppression & attrition →](04-suppression-and-attrition.md)

---

## 3. Sensing & detection *(Phase 2 — ordered before fires, decision 2026-07-27)*

The core interactive loop: place sensing assets, try to detect the enemy before being
detected. Detection is **mutual and asymmetric** — both sides' sensors run the same
machinery against the other side's units. Decisions (user, 2026-07-28): one generic
LOS sensor now with the schema shaped for acoustic and EO/IR later; glimpse-rate λ
model; dt = 1 s + 10 s epochs; static user-placed Red.

### 3.1 Stat blocks (all dials; `scenarios/sensors.toml`, `scenarios/units.toml`)

- **Sensor type:** `modality` (enum: `optical` now; `acoustic`, `eo_ir` later — each
  modality brings its own propagation term, which is why it is a tag, not a convention),
  `mount_height_m` (a mast or hovering UAS is just this dial), `max_range_m` (hard
  cutoff), `lambda0_per_s` (peak detection rate: against signature 1, τ = 1, zero
  concealment, at point-blank range), `range_half_m` + `range_exponent` (the falloff
  curve), optional field of regard (`for_width_deg`, sensor instance carries facing;
  default 360°).
- **Unit type:** `height_m` (target height for LOS), and `signature` as a **per-modality
  table** (`signature.optical = 0.6`) — acoustic/EO-IR add keys, not schema changes.
- **Sensor / unit instances** in the scenario: side (`blue`/`red`), type id, position,
  optional facing.

### 3.2 The glimpse-rate model

Detection of unit `u` by sensor `s` is a Poisson process with rate

$$
\lambda(s, u) = \lambda_0 \cdot f(r) \cdot \sigma_m(u) \cdot \tau(s, u) \cdot \big(1 - c(u)\big)
$$

$$
f(r) = \frac{1}{1 + \left(r / r_{1/2}\right)^{n}}
$$

with `λ0` = `lambda0_per_s`, `σ_m` the target's signature in the sensor's modality, `c` the
terrain concealment at its cell, `r_½` = `range_half_m` and `n` = `range_exponent`.

The rate is gated to zero when: LOS not `clear`, `r > max_range`, or `u` outside the field
of regard. `τ` and `clear` come from the Phase 1 LOS query (sensor `mount_height_m` and
unit `height_m` as the endpoint heights) — this is what the rich `LosResult` was built
for. `P(detect by t) = 1 − e^{−λt}`; per tick of length `dt`, `p = 1 − e^{−λ·dt}`.
Memoryless ⇒ results are **independent of the tick size** in distribution (V17 checks
the compounding identity exactly).

Detections were permanent in this phase, with track loss deferred to EW. **§10.1 replaced
that**: a track is held only while it is re-observed, and ages out `track_hold_s` after
its last observation. Acquisition is still the stochastic glimpse described here; only
maintenance was added. Each detection emits an event
`{time, sensor_id, unit_id, unit_pos}` into an append-only log (the POMDP-ready
observation channel, PLAN §5-S3) and marks the unit detected by the sensor's side.

### 3.3 The simulation loop (first appearance — DESIGN §7 made concrete)

`Sim::new(&scenario, seed)` resolves stat blocks and builds `SimState`; `Sim` owns the
`SimRng`. `step_one()` advances one tick `dt_s`; every `epoch_s` of sim time a (currently
empty) decision hook runs — fires/tasking phases fill it. Scenario `[sim]` block:
`dt_s = 1.0`, `epoch_s = 10.0`. Determinism: sensors and units iterate in fixed index
order; one RNG draw per live (sensor, undetected-unit) pair per tick; state carries no
hash-ordered containers.

### 3.4 Validation gates (V14–V18)

| # | Property | Reference |
|---|----------|-----------|
| V14 | detection-time distribution | constant-λ pair: empirical mean over seeded MC runs ≈ 1/λ within CI |
| V15 | closed form | MC frequency of detection by time t within binomial CI of 1 − e^{−λt} |
| V16 | rate structure | λ monotone non-increasing in range and concealment; exactly 0 when blocked, out of range, or outside field of regard; scales linearly in signature and τ |
| V17 | tick invariance | compounded per-tick survival Π(e^{−λ·dt}) equals e^{−λt} for dt ∈ {0.25, 0.5, 1, 2} (identity, float tolerance) |
| V18 | determinism | same (scenario, seed) → identical event log; different seed differs |
