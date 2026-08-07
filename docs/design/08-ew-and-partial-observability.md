[Index](README.md) · [← §7 The simulation loop](07-the-simulation-loop.md) · [§9 Air: drones & counter-air →](09-air-and-counter-air.md)

---

## 8. Electronic warfare & partial observability *(Phase 8)*

EW degrades sensing; with imperfect detection the observer reasons over a **belief** (a
probability distribution) rather than ground truth — the POMDP layer. EW is a clean
modifier on the sensing channel: with no jammers it is the identity, so EW-off reduces
bit-for-bit to §3 (V40).

### 8.1 EW — jamming as a sensing modifier

A jammer protects its own side's units by degrading the enemy's detection of them. A
jammer at `p` with `power ∈ [0,1]` and `radius` contributes a factor
`g = 1 − power·(1 − d/radius)` inside its radius (so `1−power` at the centre, `1` at the
edge); factors compose multiplicatively. The glimpse rate becomes
`λ_eff = λ · Π_j g_j(target)` over the target's own side's jammers — identity when there
are none. Data-driven: `[[side.jammers]] pos/power/radius_m` in the scenario.

### 8.2 Belief state — the POMDP layer

An **inference layer over the sim**, not sim state. `bayes_update(prior, likelihood)`
is the generic discrete posterior; a `SpatialBelief` holds a per-cell distribution over
enemy position with `update` (Bayes by an observation-likelihood raster), `predict` (a
diffusion motion model that raises entropy), `entropy`, and `most_likely_cell`. The key
observation model is **negative information**: `no_detection_likelihood` = per cell,
`P(no detection | enemy there)`, which reuses the sensing model *and EW* — cells the
sensor covers well get low likelihood (the enemy would have been seen), dead ground and
jammed cells get ~1. Multiplying an uninformative prior by the exposure-window
no-detection likelihood is "where an undetected enemy could be" — the app's belief
heatmap (green = cleared coverage, magenta = plausible hiding ground).

### 8.3 Validation gates (V40–V43)

| # | Property | Reference |
|---|----------|-----------|
| V40 | EW modifier | no jammers ⇒ factor exactly 1 and detection unchanged (EW-off = §3); a jammer cuts detection monotonically in power/proximity, sharply over a target |
| V41 | Tiger problem | `bayes_update` reproduces the exact posteriors (0.85 after one hear-left; 0.9698 after two; symmetric reversal) |
| V42 | belief well-formed | belief stays a normalised non-negative distribution; a peaked (detection) likelihood concentrates it and lowers entropy |
| V43 | negative information | repeatedly *not* detecting shifts belief out of coverage into dead ground; the motion model raises entropy |
