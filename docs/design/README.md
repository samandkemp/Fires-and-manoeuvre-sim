# Design spec

The deep, quantitative spec: model formulations, equations, state machines, and the
validation reference for each subsystem. [`docs/MATHS.md`](../MATHS.md) argues *why* each
OR strand is the right tool and states it in its own symbols; these pages work each one
out in full. A section is written **before** its subsystem is implemented, and every model
states the analytical result or invariant its tests check against.

New to the codebase? Read [`docs/HOW_IT_WORKS.md`](../HOW_IT_WORKS.md) first — it walks the
same models in plain terms, with worked numbers and the code path for each, and points
back here for the specification.

## By section

Section numbers are stable. Roughly three hundred `§N.M` references in the source tree
point at them, so a section keeps its number even when its contents are amended by a later
phase.

| § | Page | What it specifies | Gates |
|---|---|---|---|
| 0 | [The strand index](00-strand-index.md) | Each body of theory, its canonical object, and where that object is worked out | — |
| 1 | [Terrain & LOS](01-terrain-and-los.md) | Coordinate and height conventions, the derived layers, the LOS query contract, viewsheds | V1–V13, V53 |
| 2 | [Fires](02-fires.md) | Direct-fire hit probability; indirect dispersion, the Carleton kernel and its closed form | V19–V24 |
| 3 | [Sensing & detection](03-sensing.md) | The glimpse-rate model, and the first statement of the two-clock loop | V14–V18 |
| 4 | [Suppression & attrition](04-suppression-and-attrition.md) | Units as N elements; the Free/Suppressed/Pinned chain; Lanchester | V28–V31 |
| 5 | [Movement as DP](05-movement-as-dp.md) | Least-risk pathing as Bellman over the grid; the risk raster; the exchange rate `w` | V25–V27 |
| 6 | [Game theory](06-game-theory.md) | The zero-sum solver by fictitious play, and the interdiction payoff | V32–V39 |
| 7 | [The simulation loop](07-the-simulation-loop.md) | Two clocks and why; phase order; **the determinism contract and the identity discipline**; the input contract | V67 |
| 8 | [EW & partial observability](08-ew-and-partial-observability.md) | Jamming as a modifier on the rate; the belief filter and negative information | V40–V43 |
| 9 | [Air & counter-air](09-air-and-counter-air.md) | Altitude and the slant-range convention; flight kinematics; strike; two AD models; the cueing timeline | V44–V52 |
| 10 | [The decision layer](10-the-decision-layer.md) | Track lifecycle; side-wide fire allocation; belief-driven sensor tasking | V54–V58, V61 |
| 11 | [Command and control](11-command-and-control.md) | The C2 post, the air-defence payoff, ground fires and the net | V59, V62, V63, V68 |
| 12 | [SEAD](12-sead.md) | Air defence as a target; what death costs; anti-radiation homing; counter-battery | V60, V64, V65 |
| 13 | [The kill chain](13-the-kill-chain.md) | Directed targeting: priority tiers, orders, eligibility, and the target lock | V66 |

**§7 is the one to read if you read one.** It was written deliberately last: it specifies
the order the models run in and the guarantees that order buys, which only settled once
several phases had each added to it.

## By phase

The same material in the order it was built, which is how to trace a decision back to the
session that took it. Phases are not sections: a later phase usually amends earlier ones
rather than adding a section of its own.

| Phase | What it added | Where it landed |
|---|---|---|
| 1 | Terrain, derived layers, LOS, viewshed | §1 |
| 2 | Sensing and detection — deliberately ordered *before* fires | §3 |
| 3 | Direct and indirect fires | §2 |
| 4 | Units as elements, suppression, attrition | §4 |
| 5 | Movement as dynamic programming | §5 |
| 6 | The game-theoretic layer | §6 |
| 7 | Visualisation | — (app only) |
| 8 | Electronic warfare and the belief filter | §8 |
| 9 | Air, counter-air, and the slant-range convention | §9, and §1.2 for height |
| 10 | The decision layer: tracks, allocation, tasking | §10 |
| 11 | Command and control | §11 |
| 12 | SEAD — air defence becomes attritable | §12 |
| 13 | Closing the §11/§12 gaps (V61–V65) | **amended §10.4, §11.2–§11.5, §12.3–§12.6** |
| 14 | The kill chain: directed targeting | §13 |
| — | Review pass: the input contract (V67), and a confirmed §11.2 two-sided bug | **§7.6**, and §11.2 |

Note the collision worth knowing about: **§13 is the work of Phase 14**, not Phase 13.
Phase 13 lifted five limitations across three existing sections and so has no section of
its own. Section numbers and phase numbers agree up to §12 and part company after it.

## Conventions these pages share

- **Slant range everywhere** (§9.1). Flat ground with equal actor heights reduces to plain
  horizontal distance exactly, which is why adopting it needed no re-baseline.
- **Determinism.** Same binary, same `(scenario, seed)` → bit-identical output. Every phase
  added since has been *appended* to the loop and draws zero RNG when its inputs are empty,
  so each new subsystem reduces to an exact identity when switched off (§7.4).
- **Validated before optimised.** No model is made fast until it has a gate against a
  closed form. Where that meant keeping a slow reference — the brute-force viewshed, the
  fixed-step LOS oracle, the greedy allocator — the reference is kept.
- **Dials are data.** Every number is a TOML dial with a default; the spec states the
  functional form, not the value.
- **Notation.** Maths is LaTeX — `$…$` inline, `$$…$$` display — and backticks are kept
  for things that name something in the source tree: a file, a function, a TOML dial. So
  `move_cost` sits in backticks beside the equation's $c_{\text{move}}$. A symbol should
  carry a gloss naming the dial it corresponds to, since that is what ties a page to the
  schema. [`docs/MATHS.md`](../MATHS.md) follows this throughout; these design pages still
  use Unicode in prose in places, which is worth converting when a section is edited
  anyway. Rust doc comments stay Unicode — rustdoc typesets no LaTeX.

See [`docs/VALIDATION.md`](../VALIDATION.md) for the gate table as a whole, and what each
gate is checked *against*.
