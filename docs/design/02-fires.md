[Index](README.md) · [← §1 Terrain & LOS](01-terrain-and-los.md) · [§3 Sensing & detection →](03-sensing.md)

---

## 2. Fires *(Phase 3 — after sensing)*

Direct fire (LOS-gated hit probability) and indirect fire (ballistic dispersion + area
effect + terrain interaction). Weapons are data-driven stat blocks
(`scenarios/weapons.toml`); a unit type may carry a `weapon` id. Every model has a
closed-form validation gate — this is the phase where "the maths is the product" is most
literally true.

> Each modelling choice below records the alternatives considered. All numbers are
> placeholder dials, not real munition performance data.

### 2.1 Weapon stat blocks (`scenarios/weapons.toml`)

Common: `class` (`direct` | `indirect`), `rof_rounds_per_min`, `max_range_m`,
`min_range_m` (indirect only, default 0).

- **Direct:** `dispersion_mrad` — 1σ angular aiming error (milliradians); the linear
  dispersion at range `r` is `σ(r) = dispersion_mrad · r / 1000` metres.
  `p_kill_given_hit` — lethality once a round strikes the target silhouette.
  `moving_target_penalty` — factor inflating σ against a moving target (dormant until
  movement, Phase 5).
- **Indirect:** `cep_m` — circular error probable at any range (a dial; range-dependent
  CEP is a documented later refinement). `lethal_radius_m` — Carleton damage scale `R_L`.

### 2.2 Direct-fire hit model

Requires LOS `clear` (Phase 1) and `r ≤ max_range`. The round's impact scatters as an
isotropic 2-D Gaussian about the aim point (= target centre) with σ = σ(r). The target
presents a rectangle of width `W` (deflection) and height `H` (elevation) — `W` from the
unit's `silhouette_width_m`, `H` from its `height_m`. Deflection and elevation errors are
independent, so

$$
P_{\text{hit}}(r) = \mathrm{erf}\left(\frac{W}{2\sigma(r)\sqrt{2}}\right)
\cdot \mathrm{erf}\left(\frac{H}{2\sigma(r)\sqrt{2}}\right)
$$

$$
P_{\text{kill}} = P_{\text{hit}} \cdot p_{\text{kill|hit}}
\cdot \big(1 - \text{cover}(\text{cell}(\text{target}))\big)
$$

`erf` via the Abramowitz–Stegun 7.1.26 rational approximation (max error ~1.5e-7 — no
new dependency). *(Alternative considered: a single circular-target `P_hit =
1 − exp(−R²/2σ²)`; rejected because the rectangle silhouette gives distinct
deflection/elevation behaviour and a cleaner MC gate.)*

### 2.3 Indirect-fire dispersion & area effect

Impact point `b = aim + N(0, σ²I)` with `σ = cep_m / 1.1774` (the circular-Gaussian CEP
identity `CEP = σ·√(2 ln 2)`). Damage to a target at burst-to-target distance `ρ` is the
**Carleton function** `D(ρ) = exp(−ρ² / (2·R_L²))` — the standard OR incapacitation
kernel. For a single round aimed with offset `d` from a point target, the expected
damage marginalising over the Gaussian burst has a **closed form** (a Gaussian
convolution):

$$
\mathbb{E}[D(d)] = \frac{R_L^2}{\sigma^2 + R_L^2}
\exp\left(-\frac{d^2}{2 (\sigma^2 + R_L^2)}\right)
$$

Delivered damage multiplies by `(1 − cover(cell(target)))` (urban/woods shielding —
reuses the terrain cover layer; no new dial). *(Alternatives: cookie-cutter lethality
disc — simpler but no smooth gate; elliptical range/deflection error (PER/PED) — more
realistic, needs the gun-target azimuth to orient the ellipse; deferred, circular CEP
first.)*

Crest clearance (a ridge blocking the ballistic arc) reuses the Phase 1 LOS traversal
with a **parabolic** height profile — the reason `line_of_sight` took a profile closure.
Wired as a later refinement; Phase 3 assumes a clear trajectory.

### 2.4 Fires in the sim loop

A `FireMission { shooter_unit, target }` executes on tick boundaries at the weapon's
rate of fire; each round draws from the seeded RNG (one draw block per round, fixed
order). Direct rounds sample hit/kill; indirect rounds sample a burst point and apply
`D·(1−cover)` damage. Units carry `strength ∈ [0,1]`, reduced by delivered damage;
`strength ≤ 0` ⇒ killed (removed from sensing/among live targets). This is the attrition
state Phase 4 validates against Lanchester.

### 2.5 Validation gates (V19–V24)

| # | Property | Reference |
|---|----------|-----------|
| V19 | direct-fire P_hit | MC fraction of impacts inside the `W×H` rectangle within CI of the erf-product |
| V20 | P_hit monotonicity | falls with range, rises with target size, falls with cover; = 0 when blocked or beyond max range |
| V21 | indirect CEP | empirical median miss distance of sampled bursts = `cep_m` within CI (Rayleigh) |
| V22 | area-damage closed form | MC mean of Carleton damage over sampled bursts = `E[D(d)]` within CI, swept over `d` |
| V23 | damage monotonicity | `E[D]` falls with offset `d`, with cover, and rises with `lethal_radius` |
| V24 | fires determinism | same (scenario, seed, mission) → identical round outcomes and final strengths |
