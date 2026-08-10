# Studies

A **study** is a dial space: which dials to explore, over what ranges, measuring what. It is
a file rather than a pile of command-line flags because a dial space is a *design* -
something to commit, review and re-run - not something to retype and half-remember.

```
cargo run -p experiments --release --bin sensitivity -- studies/sensing.toml --seeds 40
```

## The question these answer

Every number in this repository is an **abstract placeholder**. That is deliberate and
stated everywhere, but it leaves one question hanging over every finding: does it matter
that the numbers are invented?

A `sweep` cannot answer it. It varies one dial with the rest pinned wherever the scenario
happened to leave them, so it measures a slice through a space it never explores. If two
dials interact, or the scenario's own values sit somewhere unrepresentative, the slice can be
badly unlike the whole.

Global sensitivity analysis explores the space instead, and reports what share of the
outcome's variance each dial is responsible for.

## The format

```toml
scenario = "air_raid"          # bare name in scenarios/, or a path
metric   = "air_leakers"       # any column from `outcome.rs`

trajectories = 20              # Morris: cost is r * (k + 1) design points
levels       = 4               # Morris grid resolution
sobol_n      = 256             # Sobol: cost is n * (k + 2) design points
design_seed  = 1               # the dial-space sample; separate from simulation seeds

[dials]
# Any dotted path `sweep --param` would take, and the range to explore it over.
"sensors.mast_optical.lambda0_per_s" = [0.05, 1.0]
"sim.track_hold_s"                   = [10.0, 120.0]
```

Dials are **continuous only**. A range is a pair of numbers, and every design point is a
float within it, so a categorical dial like `sim.allocation` has no place here - use
`factorial` for those.

## What comes back

**Morris** first, because it is cheap: `mu_star` ranks the dials by how much they move the
answer, and `sigma` flags one whose effect depends on where the others are. Its job is to say
what can be ignored before the expensive pass runs.

**Sobol** second, as a variance decomposition:

| Column | Meaning |
|---|---|
| `S1` | the share of the outcome's variance this dial explains **alone** |
| `ST` | the share it is involved in altogether, including every interaction |
| `ST − S1` | the share that runs **through** interactions - what a one-dial sweep cannot see |

The closing line adds up the first-order indices. Near 1 means the dials are close to
additive and one-at-a-time sweeps are sound. Well below 1 means a large share of the variance
lives in interactions, and a sweep will mislead.

## Cost, and why `--seeds` is small

Total trials is `(morris points + sobol points) × seeds`. With 4 dials, the defaults are
`20×5 + 256×6 = 1636` design points; at 30 seeds that is ~49,000 trials.

`--seeds` is deliberately modest because a design point is an **average over seeds**, and the
variance being decomposed is the one across the *dial space*, not across the dice. Seeds are
there to stop each point being one noisy draw, not to be the subject of the study.

## Reading a result honestly

A high `ST` says a dial matters *on this scenario, over this range, for this metric*. All
three qualifiers are load-bearing: widen a range and a dial can go from inert to dominant,
because a dial that is already saturated over its range has nothing left to give.

The estimator itself is gated - V71 checks it against the analytic Sobol indices of the
Ishigami function, whose third input has a first-order index of exactly zero and a large
total one. See [`docs/VALIDATION.md`](../docs/VALIDATION.md).
