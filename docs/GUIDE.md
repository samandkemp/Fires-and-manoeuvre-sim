# User guide

Everything needed to set the model up, drive it, build situations for it, and get a number
out of it that can be defended. It assumes no Rust and no prior contact with the codebase.

This is the working document - if you are using the model rather than reading about it, you
should be able to stay on this page. [`docs/MODEL.md`](MODEL.md) explains how the thing works
and why; [`docs/REFERENCE.md`](REFERENCE.md) is the lookup table for every field and dial;
[`docs/THEORY.md`](THEORY.md) derives the mathematics. This page tells you what to type.

> **Every unit, weapon and sensor number in this project is an abstract placeholder dial, not
> real munition or sensor performance data.** The models are the product; the numbers are
> knobs. Nothing here is calibrated against any real system.

**Part 1 - Getting it running**
- [1. Setting up](#1-setting-up)
- [2. Running the app](#2-running-the-app)
- [3. Driving the app](#3-driving-the-app)

**Part 2 - Building situations**
- [4. The two kinds of file](#4-the-two-kinds-of-file)
- [5. Adding a type](#5-adding-a-type)
- [6. Building a scenario](#6-building-a-scenario)
- [7. Describing the ground](#7-describing-the-ground)
- [8. Placing things](#8-placing-things)
- [9. The kill chain](#9-the-kill-chain)
- [10. Loading and checking](#10-loading-and-checking)

**Part 3 - Getting numbers out**
- [11. Study design: the two rules](#11-study-design-the-two-rules)
- [12. The study tools](#12-the-study-tools)
- [13. Reading the output](#13-reading-the-output)
- [14. A worked study, end to end](#14-a-worked-study-end-to-end)
- [15. Keeping findings honest](#15-keeping-findings-honest)

**Part 4 - Maintaining it**
- [16. Testing, linting, releasing](#16-testing-linting-releasing)
- [17. Extending the harness](#17-extending-the-harness)
- [18. Troubleshooting](#18-troubleshooting)

---

# Part 1 - Getting it running

## 1. Setting up

Target: **Bevy 0.19** on **stable Rust**. Written for someone new to both. Follow it in
order; each step ends with a check, so you know it worked.

### 1.1 Install Rust

Rust is installed through `rustup`, which manages compiler versions for you.

**Windows** - download and run `rustup-init.exe` from <https://rustup.rs>. It will tell you it
needs the **Visual Studio C++ Build Tools**, the MSVC linker. Let it guide you, or install
"Desktop development with C++" from the Visual Studio Installer first. This is required: Rust
links through the MSVC toolchain on Windows.

**macOS and Linux** - run

```
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

accept the defaults, and restart the terminal.

Then set the default toolchain:

```
rustup default stable
```

**Check it worked.** `rustc --version` and `cargo --version` should both print a version.
`cargo` is Rust's build tool and package manager, and it is the only command you will need.

### 1.2 Editor setup

VSCode extensions, from the Extensions panel:

- **rust-analyzer** (rust-lang) - *essential*. The language server: autocomplete, inline
  types, go-to-definition, error highlighting. This is 90% of the experience.
- **CodeLLDB** (Vadim Chugunov) - debugger with breakpoints, cross-platform.
- **Even Better TOML** - syntax and validation for `Cargo.toml` and the scenario files.
- **Dependi** - shows latest crate versions inline in `Cargo.toml`.
- **Error Lens** - optional, but prints errors inline on the line rather than only
  underlining them, which is a real help early on.

Recommended workspace settings (Ctrl/Cmd+Shift+P, "Preferences: Open Workspace Settings
(JSON)"):

```json
{
  "rust-analyzer.check.command": "clippy",
  "editor.formatOnSave": true,
  "[rust]": { "editor.defaultFormatter": "rust-lang.rust-analyzer" }
}
```

That runs Clippy, Rust's linter, on save and auto-formats with `rustfmt`. Take Clippy's advice
seriously - it teaches idiomatic Rust as you go.

Always open the **project root folder**, not a single file, so rust-analyzer can see the whole
workspace.

### 1.3 First build

```
cargo run -p app
```

**Expect the first build to take several minutes.** You are compiling an entire game engine
from source. This happens once; every build after is seconds. When it finishes, the tactical
map opens on the `default` scenario.

Fast iterative builds are already configured in the repository, so there is nothing to set up:

- `[profile.dev] opt-level = 1` with `[profile.dev.package."*"] opt-level = 3` compiles *your*
  code fast and *dependencies* optimised, so the simulation still runs at a usable speed while
  you iterate. `sim_core` itself is at opt-level 2, because a slow engine makes a slow study.
- `.cargo/config.toml` selects a fast linker (`rust-lld` on Windows, `lld` via clang on Linux;
  macOS needs nothing).
- The app enables Bevy's `dynamic_linking` feature, which massively speeds up rebuilds.

**Remove `dynamic_linking` before making a release build** - it produces a binary that will not
run elsewhere. See [Testing, linting, releasing](#16-testing-linting-releasing).

### 1.4 The network caveat

This machine's connection drops sustained TLS downloads mid-stream, and rustup, cargo and VSIX
downloads are all affected. `~/.cargo/config.toml` sets `net.retry = 10` and disables HTTP/2
multiplexing. If a large download fails anyway, **retry** - resume loops work.

### 1.5 Learning Rust alongside

The audience for this project is not assumed to know Rust, and none of Part 2 or Part 3 requires
it - scenarios and studies are TOML and command lines. If you do want to read the engine, Rust's
ownership model is the thing that makes it feel alien at first, and it is worth an hour before
you start rather than an afternoon of confusion in the middle.

- **The Rust Book** - <https://doc.rust-lang.org/book> - chapters 1-10, especially 4 (Ownership)
  and 10 (Generics and Traits). The single best resource, and free.
- **Rustlings** - <https://github.com/rust-lang/rustlings> - small in-terminal exercises; the
  fastest way to make the concepts stick.
- **Bevy Quick Start** - <https://bevy.org/learn/quick-start> - only needed for the app crate.
  `sim_core`, where all the maths lives, has no Bevy in it at all.

Non-obvious Rust idioms in this codebase carry a one-line comment saying why they are there, so
you can usually read past one without stopping.

### 1.6 Check it worked

```
cargo run -p app                      # the window opens
cargo test --workspace                # everything passes
cargo run -p validation --release --bin validation_report
```

That last one is the interesting check. It runs all 77 validation gates and prints each one
beside **the closed form it is checked against** - which is the artefact worth showing someone
who asks whether the model is any good.

## 2. Running the app

```
cargo run -p app                       # the `default` scenario
cargo run -p app -- air_raid           # by bare name, resolved in scenarios/
cargo run -p app -- path/to/mine.toml  # or by path, for one kept elsewhere
```

An unknown name prints the scenarios it could have opened rather than failing obscurely. The
in-app **scenario** dropdown lists every file in `scenarios/` that parses as a scenario and
switches between them live - terrain, forces and all, no restart.

### The bundled scenarios

Start with these. Each exists to demonstrate one thing, and several carry their measured
result in a header comment.

| File | What it demonstrates |
|---|---|
| `default.toml` | The main scenario: terrain generation, mixed forces, sensor placements |
| `flat_range.toml` | A flat, featureless test range - isolates a model from terrain effects |
| `mountain_pass.toml` | A composable terrain recipe: rolling base, then ridge, woodland, urban |
| `fire_allocation.toml` | Four shooters that can all reach all four targets - where the allocation rule actually matters |
| `sensor_search.toml` | Narrow-arc observers searching by belief (needs `sensor_tasking = true`) |
| `covered_approach.toml` | Movement decisions: what a covered approach is worth as caution rises |
| `kill_chain.toml` | Directed targeting, and ground counter-battery |
| `air_raid.toml` | Counter-air: a drone raid against self-cued and net-cued defences |
| `hunter_uas.toml` | What a drone is allowed to decide for itself (`autonomous`) |
| `ad_c2.toml` | Coordinated against decentralised air defence - delete the `[[blue.c2]]` block to compare |
| `fires_c2.toml` | Ground fires and the net (`fires_need_c2`) |
| `ew_c2.toml` | Jamming the command link - the soft kill on the asset SEAD hard-kills |
| `sead_arm.toml` | Anti-radiation homing, and the go-silent counter |

## 3. Driving the app

**The UI is modeless.** Selection is a set, and the only mode is what a right-click places.

| Input | Does |
|---|---|
| Left-click | Select - units, drones, air-defence batteries or C2 posts |
| Shift + left-click | Add to, or toggle in, the selection |
| Left-drag | Box-select |
| Right-click | Move the selection here, preserving formation |
| Shift + right-click | Append a waypoint instead of replacing the route |
| Ctrl + A | Select all live assets |
| Del | Remove the selection |
| Esc | Clear the selection |
| Middle-drag / scroll | Pan / zoom |
| **Space** | Run / pause |
| **.** | Step one tick |
| **E** | Open or close the experiment runner |

Space and `.` are on the keyboard as well as in the panel deliberately: inspecting a battle
means keeping your eyes on the map, and reaching for a button loses the moment you paused for.

### 3.1 Watching a battle unfold

A scenario resolves in a couple of hundred simulated seconds, which is faster than it can be
watched. The clock panel exists for that.

| Control | What it does |
|---|---|
| **speed** | **Simulated seconds per real second.** 0.2× to study a duel, 60× to skip ahead |
| **+1 s / +10 s** | Step one integration tick, or one decision epoch - the two units the model actually has |
| **pause on** *contact* / *loss* / *air* | Breakpoints: stop *on* the tick that produced the event |
| **Run to** | Jump ahead at headless speed to a given simulated time |
| **Re-run at seed** | Replay the same battle exactly |
| **Reset scenario** / **Clear all** | Reload from file, or empty the map to build one by hand |

**Speed cannot change the outcome.** The wall clock decides *when* a tick happens, never how
big it is: real time accumulates into a budget which is spent in whole `dt_s` ticks. So 0.2×
and 60× produce the same event log, and the slow one is a magnifying glass rather than a
different experiment.

**Breakpoints matter more than they sound.** The moments worth seeing - first contact, a
casualty, a missile away - last a single tick, so slowing down is not enough on its own; it
also requires looking at the right pixel at the right moment. A breakpoint stops the clock on
the tick that tripped it.

### 3.2 Placing assets

Every asset class can be placed for **either** force: pick what to place, then pick the side it
joins. The counter-sensing fight - positioning to see without being seen - is the point of the
tool, so it has to be something the map can express rather than only a scenario file.

| Mode | Places |
|---|---|
| **Sensor** | An observer of the chosen type |
| **Unit** | A manoeuvre or artillery unit, with its weapon resolved from the stat block |
| **Jammer (EW)** | An EW bubble degrading the *other* side's sensing nearby |
| **Drone** | An air asset at the panel's altitude, heading and speed |
| **Air defence** | A battery, with its organic radar |
| **C2 post** | A post coordinating nearby friendly air defence |
| **Objective for selected unit(s)** | Where the selected units should get to, planning their own route |

An **objective** is the alternative to drawing a route by hand. A unit given one re-solves its
route every decision epoch against what its side knows about enemy sensors, so placing a sensor
across its path changes where it goes. The caution slider is the exchange rate: metres of
movement cost the unit will spend to avoid one unit of exposure, with `0` meaning the shortest
route regardless of who is watching.

A planned route is drawn **dashed** and a scripted one **solid**, because they are different
kinds of thing: a scripted route is a commitment, a planned one is this epoch's opinion and will
be re-solved at the next. Setting one cancels the other, in either direction.

### 3.3 Model dials, live

The **Model dials** section edits the model while it runs, grouped by subsystem. A change takes
effect on the next tick.

| Group | Dials |
|---|---|
| Suppression | probability per near miss, recovery rate, near-miss radius, suppressed fire factor |
| Tracks | track hold, probability a track is maintained per epoch |
| Movement and planning | default caution, fire risk weight, re-path margin, allocation horizon |
| Doctrine | strict or weighted, per side (the priority list itself is scenario-level) |

**This is what a sweep cannot give you.** A sweep reports what a dial is worth on average over
hundreds of trials; it says nothing about what the dial *does*. Watching suppression recovery
change how a position behaves under fire is a different kind of knowing, and it is usually what
tells you which sweep is worth running.

**Three dials are read-only**, and shown rather than hidden. `dt_s` and `epoch_s` decide what a
tick and an epoch mean, so changing them mid-run would make the two halves of a trial answer
different questions. `belief_cells` sizes rasters that would have to be rebuilt and re-keyed.

### 3.4 Overlays and inspection

| Button | Shows |
|---|---|
| **Coverage (Pd)** | Detection probability across the map for the chosen side's sensors |
| **Belief snapshot** | Where the enemy could be, given what has and has not been seen |
| **Belief the sim is flying on** | The simulation's own per-side belief - what tasking actually reads |
| **Legend** | What every marker and colour means |

Overlays compute on a **background thread**, so the map stays live while a raster is built -
which matters, because a full pass over a large map with a long-ranged sensor is measured in
seconds.

They also say when they have stopped being true. An overlay is computed from a snapshot of the
assets, so moving a sensor makes it describe where that sensor *used to be*. The panel marks it
**STALE**, and **keep it up to date** rebuilds it automatically when its inputs change,
throttled so a running battle does not queue rasters faster than they finish.

### 3.5 The experiment runner

Press **E**, or click **Experiments...** at the top of the panel, to open a window for
building, queueing and running studies. It exists because setting up *one* sweep is a command
worth typing, and setting up six related ones is an afternoon of remembering dotted paths.

| Part | What it does |
|---|---|
| **Dial browser** | Every dial the loaded libraries expose, filterable, each with its meaning, unit and theory section. `use` puts one into the study being built, pre-filled with its own range |
| **Study builder** | Scenario, metric, kind (sweep / factorial / batch), seeds, horizon, and dials pinned for every arm |
| **Queue** | Runs one after another on a background thread. The map stays live, and one failure does not abandon the rest |
| **Finished** | What each run reported, with the command that produced it |

**It shows the command it will run**, live, as the form is filled in, and refuses to queue a
study that is malformed - naming what to fix rather than only what is wrong. The failure worth
preventing is a batch left running overnight whose third run had no second value to compare
against.

**Save as script** writes the whole queue as a shell script, in order. That file is the record:
whatever was set up by hand, it repeats it. **Save as TOML** writes the same thing in a form the
runner can reload.

> **The rule it is built to.** The runner is not a second way of doing experiments; it is a way
> of writing down the existing one. Every study it runs is a `sweep`, `factorial` or `batch`
> invocation that could have been typed, and the window shows which. An experiment that exists
> only as clicks is not reproducible, and every finding this project keeps depends on being able
> to re-run it.

### 3.6 Screenshots

```
FIRES_SIM_SCREENSHOT=out/shot.png cargo run -p app -- air_raid
```

Captures one frame a few seconds in and exits. The rig runs on a frame-locked path, so its
capture time is reproducible rather than dependent on how fast the machine drew.

---
# Part 2 - Building situations

Everything in this part is TOML in [`scenarios/`](../scenarios/). None of it requires touching
Rust.

## 4. The two kinds of file

This split is the heart of the data model.

- **Libraries** - `units.toml`, `weapons.toml`, `sensors.toml`, `air.toml`, `air_defence.toml`,
  `c2.toml`, `terrain_types.toml` - say **what things are**. One entry per type.
- **Scenarios** - `default.toml`, `air_raid.toml`, `kill_chain.toml` and the rest - say **where
  things are**. Each placement names a `type` from a library.

So this, in `units.toml`:

```toml
[afv]
role = "armour"
height_m = 2.8
silhouette_width_m = 3.2
element_count = 3        # a troop of 3 vehicles
speed_m_s = 6.0
weapon = "afv_cannon"
[afv.signature]
optical = 0.75
```

is referred to by this, in any scenario:

```toml
[[red.units]]
id = "red-1"
type = "afv"
pos = [5600.0, 6400.0]
route = [[5600.0, 6400.0], [4300.0, 5200.0]]
```

Change `element_count` in the library and **every** `afv` in every scenario changes. That is the
point: numbers are dials to be turned, never values baked into code.

A scenario is told apart from a library by *being parseable as a scenario* - it needs a `name`
and a `[terrain]` block, which no library has. There is no hard-coded list of scenario names
anywhere, so adding a library never confuses the app's picker or the batch runner.

`air.toml`, `air_defence.toml` and `c2.toml` are **optional**: a scenario directory without them
loads with those libraries empty, so an older scenario set still works.

## 5. Adding a type

### 5.1 A unit

`units.toml`. A unit is **N sub-elements** removed one at a time, not a single hit-pointed blob.

| Field | Meaning |
|---|---|
| `height_m` | Actor height for line of sight and slant range |
| `silhouette_width_m` | Target width for the direct-fire hit integral |
| `element_count` | How many sub-elements. Drives fire volume *and* how much there is to kill |
| `speed_m_s` | Cross-country pace. `0.0` means emplaced |
| `weapon` | Optional key into `weapons.toml`. No weapon means unarmed |
| `[type.signature]` | Per-modality detectability in $[0,1]$. `optical` is implemented |
| `value` | Optional. What killing this is worth to the enemy's allocation. Omitted means derived |
| `role` | Optional. Free-form label a fire plan can sort on |

Two of these repay thought.

**`signature` is a table, not a number**, so adding acoustic or EO/IR sensing later means adding
a key - `acoustic = 0.8` for a vehicle with a generator running - rather than a schema change.

**`value` is usually better left out.** Omitted, it is derived as
$\text{elements} \times (1 + \theta/\theta_{\max})$, where threat $\theta$ comes from rate of
fire, kill probability and reach - so a stat block with no score is still ranked sensibly by the
allocator. Declare it to say something the derivation cannot, such as "the radar matters more
than its firepower suggests".

**Worked example.** A light reconnaissance vehicle - fast, well-armed for its size, hard to see:

```toml
# units.toml
[recce_vehicle]
role = "recce"
height_m = 2.2
silhouette_width_m = 2.4
element_count = 2
speed_m_s = 9.0            # faster than the AFV troop
weapon = "afv_cannon"      # reuses an existing weapon
[recce_vehicle.signature]
optical = 0.45             # smaller than the 0.75 AFV
```

That is the whole change. It is now placeable in any scenario as `type = "recce_vehicle"`,
selectable in the app's unit dropdown, sweepable by
`sweep --param units.recce_vehicle.speed_m_s`, and it answers to `"recce"` in a priority list.
No code, and no rebuild of anything but the scenario.

### 5.2 A weapon

`weapons.toml`. Two classes, and the choice decides which model resolves the shot.

```toml
[afv_cannon]
class = "direct"           # LOS-gated, hit against the silhouette
rof_rounds_per_min = 6.0
max_range_m = 2500.0
dispersion_mrad = 0.5      # angular error; sigma(r) = dispersion * r / 1000
p_kill_given_hit = 0.7

[howitzer_155]
class = "indirect"         # ballistic; needs a track, not a sightline
rof_rounds_per_min = 3.0
max_range_m = 24000.0
min_range_m = 2000.0       # the inner dead zone
cep_m = 90.0               # circular error probable
lethal_radius_m = 40.0     # the Carleton kernel's scale
```

**Direct fire needs line of sight; indirect fire does not** - it needs a *track*, meaning
somebody on the same side has seen the target and the track has not lapsed. That asymmetry is
the whole difference between the two classes at the decision layer, and it is why an artillery
scenario needs an observer to be worth anything.

`min_range_m` is easy to forget and expensive to forget: place targets inside a howitzer's dead
zone and it will sit silent with nothing on screen to explain why.

**Anti-radiation munitions** add two dials:

```toml
[arm_missile]
class = "indirect"
anti_radiation = true
cep_m = 15.0               # against a transmitting radar: it homes
silent_cep_m = 120.0       # against a silent one: it does not
```

This is the dial that makes "go silent" a real defensive option with a real cost. Every munition
without the flag is unaffected, exactly.

### 5.3 A sensor

`sensors.toml`.

```toml
[narrow_optical]
modality = "optical"       # the propagation channel
mount_height_m = 4.0       # ignored when the sensor is carried by a drone
max_range_m = 3000.0       # hard cutoff
lambda0_per_s = 0.6        # base glimpse rate at zero range
range_half_m = 1400.0      # the range at which the rate has halved
range_exponent = 2.0       # how sharply it falls off
for_width_deg = 70.0       # field of regard; omit for all-round
```

The detection rate is
$\lambda = \lambda_0 f(r) \sigma \tau (1-c)$ with $f(r) = 1/(1 + (r/r_{1/2})^n)$, so
`lambda0_per_s` sets the ceiling and `range_half_m` sets where it collapses.

**`for_width_deg` is what makes a sensor taskable.** A sensor with a finite field of regard can
only watch a slice of the map, so *where it points* is a decision - and it is the only kind of
sensor the tasking layer has anything to do with. An all-round sensor has nothing to task.

### 5.4 A drone

`air.toml`. A drone is an airframe plus, optionally, a sensor and/or a payload.

```toml
[strike_uas]
role = "strike"
height_m = 1.5
silhouette_width_m = 4.0
cruise_speed_m_s = 45.0
max_turn_rate_deg_s = 10.0    # implies r_min = v/omega = 258 m
endurance_s = 2400.0
payload = "guided_bomb"       # an `indirect` weapon from weapons.toml
munitions = 2
expendable = false            # a carrier: releases and flies on
release_range_m = 2500.0      # standoff distance
[strike_uas.signature]
optical = 0.35
```

- `sensor = "uas_optical"` instead makes it a recce platform - the sensor rides the airframe,
  seeing from its altitude and facing its heading.
- `munitions` and `expendable` span the spectrum in two dials: a reusable carrier drops several
  and flies on; a one-way attack munition carries one and dies with it.
- **`max_turn_rate_deg_s` is load-bearing, not decoration.** It is a rate limit on heading, so a
  drone asked to fly a 90° corner cannot - it flies an arc of radius $v/\omega$, arriving late
  and displaced. That error is exactly what an air-defence engagement window is made of.
- **`release_range_m` and the defending battery's `max_range_m` are a matched pair.** Set release
  inside the gun's bubble and the drone must fly through it; set it outside and the long-range
  SAM and its cueing chain are what matter. This is the single most consequential number in a
  counter-air scenario, and getting it wrong is how you build a scenario that measures nothing -
  see [Troubleshooting](#18-troubleshooting).

### 5.5 An air-defence battery

`air_defence.toml`. Two engagement models, because time-to-kill is distributed differently in
each.

```toml
[ciws]
role = "point_defence"
max_range_m = 2000.0
min_alt_m = 0.0
max_alt_m = 1500.0
mount_height_m = 3.0
requires_los = true
reaction_time_s = 2.0         # crew/system delay once a track is actionable
cue_latency_s = 4.0           # comms delay paid on any externally cued track
magazine = 30                 # 0 = unlimited
channels = 1                  # simultaneous engagements: what a raid saturates
sensor = "ciws_radar"         # organic radar: self-cueing, latency 0
[ciws.engagement.gun]
kill_rate_per_s = 0.9         # TTK ~ Exp(0.9), so E[TTK] = 1.1 s

[sam]
# ... envelope as above, but no `sensor`: dependent on the cueing chain, so it always
# pays cue_latency_s. That is the lever the air_raid experiment sweeps.
[sam.engagement.missile]
ssk_p = 0.65
missile_speed_m_s = 700.0
reload_s = 8.0                # E[TTK] = t_f/p + (1/p - 1) * reload_s
```

**A gun and a missile fail differently, not just at different rates.** A gun grinds continuously
(Poisson), so stacking two batteries on a target simply adds kill rates and wastes nothing. A
missile is a discrete round, so overkill is real. That is why coordination pays for missiles and
barely registers for guns - a result worth reproducing before trusting any counter-air
conclusion.

**Omitting `sensor` is a modelling statement**, not an oversight: a battery with no organic radar
can fire only on tracks handed to it over the net, so it always pays `cue_latency_s`. An instance
can also set `self_cue = false` to force that on a battery that *does* have a radar.

### 5.6 A C2 post

`c2.toml`. A post does not shoot, sense, or move. Its only effect is that air-defence batteries
within `coordination_range_m` allocate as one group.

```toml
[ad_command_post]
coordination_range_m = 6000.0
height_m = 4.0
silhouette_width_m = 5.0
link_latency_s = 0.0          # how long a battery must be inside before it is netted
[ad_command_post.signature]
optical = 0.85                # deliberately high
```

The signature is high on purpose. A command post concentrates antennas and vehicles, which is
what makes it findable - and **being findable is the point**, because it is the asset an attacker
most wants to kill first. Killing it costs no battery, no magazine and no envelope; it costs only
the coordination.

### 5.7 Terrain types

`terrain_types.toml`. Per-type dials that every derived layer reads.

| Field | Meaning |
|---|---|
| `feature_height_m` | Canopy or building height above ground - the blocking surface is $z + f$ |
| `extinction_per_m` | $\kappa$, sight attenuation per metre of canopy: $\tau = e^{-\kappa L}$ |
| `cover` | $[0,1]$ protection against fires |
| `concealment` | $[0,1]$ reduction in detectability |
| `mobility_cost` | $\ge 1$ movement multiplier (`inf` means impassable) |

Trees attenuate softly (`extinction_per_m = 0.08`); urban hard-blocks
(`extinction_per_m = 0`, but `feature_height_m = 8.0`). Two different mechanisms, deliberately.

**Cover and concealment are different things.** Cover reduces damage; concealment reduces
detectability. A wood conceals well and protects poorly; a built-up area does the reverse.

### 5.8 Roles

Every stat block may carry a `role`, a free-form string. It exists for one purpose: a fire plan
can sort on it.

Roles never mask classes. An entry in a priority list may name an **id** (`"red-cp"`), a **role**
(`"armour"`), a **class** (`unit`, `air_defence`, `c2`, `air`), or **`"all"`** - and
`"air_defence"` still matches a battery whose role is `"point_defence"`. So inventing a role can
only add precision, never take it away.

## 6. Building a scenario

### 6.1 The skeleton

```toml
name = "my_scenario"
default_seed = 11

[sim]
dt_s = 1.0
epoch_s = 10.0

[terrain]
cell_size_m = 10.0
width_cells = 700
height_cells = 300
[terrain.source.flat]
elevation_m = 0.0

[[blue.units]]
id = "gun-a"
type = "sp_gun"
pos = [800.0, 1500.0]

[[red.units]]
id = "tank-1"
type = "afv"
pos = [4800.0, 1250.0]
```

That is a complete, runnable scenario. `[sim]` and both forces are optional - every dial has a
default - so the minimum is a `name`, a `[terrain]` block and something to look at.

**Positions are world metres, not cells.** A 700 by 300 grid at 10 m per cell is 7 km by 3 km, so
`pos = [4800.0, 1500.0]` sits 4.8 km east and centred north-south. X is east, Y is north, and the
origin is the south-west corner.

### 6.2 The `[sim]` dials

Every dial has a default, so a scenario states only what it wants to change. The full table with
types and domains is in [`docs/REFERENCE.md`](REFERENCE.md#sim-dials); these are the ones you will
actually reach for.

| Dial | Default | What it does |
|---|---|---|
| `dt_s` | 1.0 | Tick length - the continuous cadence |
| `epoch_s` | 10.0 | Decision-epoch length - the discrete cadence |
| `track_hold_s` | 45.0 | How long a track survives unobserved |
| `allocation` | `optimal` | `optimal` / `greedy` / `independent` |
| `sensor_tasking` | `false` | Do steerable sensors search by belief? |
| `fires_need_c2` | `false` | Must a ground shooter be under a live C2 post to coordinate? |
| `risk_weight` | 50.0 | Default exchange rate between movement cost and exposure |
| `p_suppress` | 0.15 | Chance one near miss steps suppression up |

**Four of them are switches back to older behaviour**, which is how one model gets isolated from
another - and that is usually what you want when designing a study:

- **`allocation = "independent"`** restores the rule where every shooter picked the nearest enemy
  for itself. Comparing it against `optimal` is what a `sim.allocation` sweep measures, and the
  answer is that coordinating is worth about 17% off the time to clear the enemy while
  *optimality* over greedy is worth slightly less than nothing.
- **`track_hold_s`** set towards the run length recovers permanent detection - useful for studying
  fires without tracks lapsing underneath them.
- **`sensor_tasking`** is off by default, so a `facing_deg` you write is taken as meant. Turn it on
  to let sensors search, but note that it dissolves any scenario whose premise is a *committed*
  sensor posture.
- **`fires_need_c2`** is off by default, so a side's guns coordinate for free. Turn it on and a
  shooter must be inside a live friendly post's radius to join the side-wide fire plan; one
  outside picks for itself.

## 7. Describing the ground

`[terrain.source]` takes one of four forms. The first two are the originals; the last two let a
map be *described* rather than picked.

```toml
[terrain.source.flat]                       # dead flat - isolates a model from terrain
elevation_m = 0.0

[terrain.source.hills]                      # seeded rolling relief
count = 24
max_height_m = 120.0
base_radius_m = 600.0
woods_fraction = 0.28
urban_blocks = 4

[terrain.source]                            # a named recipe
preset = "mountain_pass"                    # or rolling_hills, wooded_hills,
                                            # light_urban, dense_urban, flat_plain

[terrain.source.layers]                     # a recipe defined in place
base = { hills = { count = 20, max_height_m = 90.0, base_radius_m = 700.0 } }
[[terrain.source.layers.apply]]
ridge = { bearing_deg = 20.0, crest_m = 320.0, width_m = 1400.0 }
[[terrain.source.layers.apply]]
woodland = { fraction = 0.32, patch_scale_m = 450.0 }
[[terrain.source.layers.apply]]
urban = { blocks = 5, min_size_m = 250.0, max_size_m = 500.0 }
```

**Layers apply in the order written** - urban after woodland leaves urban, and the reverse does
not - and all draw from one seeded stream, so a recipe plus a seed always reproduces the same map.
`base` is `flat` or `hills`; `apply` may be empty. `mountain_pass.toml` is a worked example.

The key inside `[[terrain.source.layers.apply]]` is **`apply`**, not `features`. Mistyping it used
to be silently ignored, producing a flat map where a ridge was intended; the schema now rejects
it.

Two forms deserve their reputations. **`flat` is the right choice for a validation fixture** - it
takes terrain out of the answer entirely, so anything measured is the model you intended to
measure. And **`preset` is the right choice for *a* map rather than *this* map** - when you want
representative ground and do not care about its particulars.

**Terrain build is the expensive part of a run**, seconds against microseconds for a trial. That
matters for study design (§11) and it is why a `terrain_types` sweep is slower and noisier than
any other kind.

## 8. Placing things

Every placement names an `id` unique in the scenario, a `type` from a library, and a `pos`. The
extras differ by class.

```toml
[[blue.units]]
id = "red-1"
type = "afv"
pos = [5600.0, 6400.0]
route = [[5600.0, 6400.0], [4300.0, 5200.0]]   # scripted; omit for a static unit

[[blue.units]]
id = "red-2"
type = "afv"
pos = [5600.0, 6800.0]
objective = [4300.0, 5200.0]                   # OR: plan its own way there
risk_weight = 400.0                            # optional; defaults to [sim] risk_weight

[[blue.sensors]]
id = "obs-1"
type = "narrow_optical"
pos = [1000.0, 1500.0]
facing_deg = 90.0                              # 0 = east, anticlockwise; needs a finite for_width_deg

[[blue.jammers]]
pos = [3000.0, 1500.0]
power = 0.8                                    # peak degradation at the centre, [0,1]
radius_m = 1200.0

[[red.air]]
id = "striker-1"
type = "strike_uas"
pos = [9000.0, 3000.0]
altitude_m = 300.0
altitude_ref = "agl"                           # or "amsl" - decides whether terrain masks it
heading_deg = 180.0
waypoints = [[6000.0, 3000.0], [4000.0, 3000.0]]
terminal = { orbit = { radius_m = 800.0, clockwise = true } }   # or "hold"
target = { unit = "sam-1" }                    # or { point = [x, y] }

[[red.air_defence]]
id = "ciws-1"
type = "ciws"
pos = [5200.0, 1500.0]
self_cue = true                                # false forces it onto the net
emitting = true                                # false is EMCON: the radar is off

[[red.c2]]
id = "red-cp"
type = "ad_command_post"
pos = [5400.0, 1500.0]
```

Five of these carry more meaning than they look like they do.

**`target = { unit = "..." }` resolves across units, air-defence batteries and C2 posts** - one
namespace - so sending a strike drone at a SAM needs no special syntax. The key stayed `unit` for
compatibility; `asset` is the clearer alias and means the same thing.

**A unit has a `route` or an `objective`, never both** - declaring both is a load error. A route is
scripted and followed exactly; an objective is planned toward, re-solved each decision epoch
against what the unit's side knows about enemy sensors, so a sensor placed across the way changes
where it goes. `risk_weight` is the exchange rate: at `0` the unit takes the short way regardless
of who is watching. **Sweeping it on one unit while another follows a fixed route is how the trade
between arriving quickly and arriving alive gets measured** - control and treatment on the same
map and the same seed.

**`self_cue` and `emitting` are two different decisions**, and they were once one flag.
`self_cue = false` means the battery waits for a track over the net and pays `cue_latency_s` - its
radar still runs, still detects, and can still be homed on by an anti-radiation missile.
`emitting = false` is EMCON: the radar is off, so the battery detects nothing through it and an
ARM has nothing to ride. Measured, going dark costs a battery its whole contribution - zero
detections, zero shots - in exchange for surviving the missile.

**`altitude_ref` decides whether terrain can mask a drone.** `agl` follows the ground and rides
over ridges; `amsl` holds a constant height above sea level and is masked by anything taller. Same
number, opposite behaviour.

**`autonomous` decides whether a drone may pick its own target.** Off by default. With it on, an
airframe with no live assignment releases on a located enemy that comes inside release range,
ranked by the side's doctrine and value function. It does not divert to go and find one, and an
explicit `target` always wins.

## 9. The kill chain

A side **always** has a fire plan. Omitting the block gives `priority = ["all"]` - one tier
holding everything, ranked by the ordinary payoff, which *is* the undirected behaviour. Declare one
and it is **followed**:

```toml
[blue.doctrine]
priority = ["red-cp", "air_defence", "armour"]   # id, class, role - all valid
mode = "strict"                                  # the default; or "weighted"

[[blue.orders]]                                  # bypass the decision entirely
shooter = "gun-a"
target = "red-cp"
```

**`strict` means a shooter that can reach a higher tier takes it even at a worse shot** - a crew
follows orders, not a kill-probability table. `weighted` scales value by tier instead, so doctrine
biases the optimisation without overriding it. The difference is exactly the cost of directive
control against optimal control, and it is measurable:

```
sweep <scn> --param blue.doctrine.mode --values strict,weighted --seeds 1000 --metric red_cleared_s
```

**A name matching nothing is a load error** listing what would have worked - because a tier that
silently matches nothing is a doctrine nobody is following.

Two rules stop a fire plan wasting ammunition. **Line of sight and range block a pairing**, so a
shooter whose top tier is masked by a ridge falls through to what it can actually engage rather
than idling; an `[[orders]]` entry lapses the same way while its target is unreachable, resuming
when it reappears. And **a shooter holds its target** until it is dead or can no longer be
engaged, rather than re-deciding every epoch and flip-flopping between two similar targets. A new
order is the one thing that breaks a lock.

`kill_chain.toml` is the worked example, with the measured result in its header comment.

## 10. Loading and checking

```
cargo run -p app -- my_scenario        # by bare name, resolved in scenarios/
cargo run -p app -- path/to/mine.toml  # or by path
```

An unknown name prints the ones that would have worked.

**Two classes of mistake are caught at load rather than in the results.**

**Unknown keys are rejected.** Nearly every dial has a default, so a misspelt one - `track_hold`
for `track_hold_s` - used to parse perfectly, take the default, and quietly change what the
scenario meant. Every schema now sets `deny_unknown_fields`, so the loader refuses it and names
the key.

**Values the model cannot run on are rejected.** `dt_s = 0` never advances the clock and
`epoch_s = 0` makes the epoch count infinite, and both hang rather than failing. Probabilities
must lie in $[0,1]$, durations and radii must not be negative, and the two stat-block dials that
reach a divisor - a sensor's `range_half_m`, an indirect weapon's `lethal_radius_m` - must be
positive, because a zero there gives `NaN` and the subsystem goes silently *inert* rather than
visibly wrong.

The list of refusals is deliberately short. Most zeros are legitimate statements: a drone with
`cruise_speed_m_s = 0` is stationary, a battery with `max_range_m = 0` engages nothing.

**You do not have to edit a file to try a different value:**

```
cargo run -p experiments --release --bin sweep -- my_scenario \
    --param sim.track_hold_s --values 10,20,45,90 --seeds 500
```

`--param` patches any dotted path - scenario **or** stat-block library - into the TOML before it
is parsed, and the result goes through exactly the same validation as a file on disk.

### Seeds

Everything random comes from one seeded generator. Same scenario plus same seed gives the same
battle, down to the last round. `default_seed` sets it; the app's **seed** box and `--seeds` on
the runners override it.

One subtlety decides what your experiment is actually measuring, and it is important enough that
§11 opens with it.

---
# Part 3 - Getting numbers out

The app shows one battle. That is the wrong tool for a question, because one battle is one draw
from a distribution and it is not the interesting one. This part is about the other half:
running the same situation thousands of times and reporting what actually changed, with an error
bar.

**Always `--release`.** A debug build of a Monte Carlo study is roughly twenty times slower and
tells you nothing a release build does not.

## 11. Study design: the two rules

Both exist because breaking them produces a confident wrong answer rather than an obvious
failure.

### Rule 1 - fix the map, vary the dice

`Sim::new(scenario, libs, seed)` derives **both** the terrain and the RNG stream from that one
seed. Looping it over seeds therefore varies the map and the luck together, and the result
averages two sources of variance that answer different questions.

Every study here builds the terrain **once per worker, from the scenario's own `default_seed`**,
and resets between trials. So the map is held fixed and the question is "what happens on *this*
map, on average". It is also far faster: terrain generation is seconds, a trial is microseconds.

To average over maps - a fair question, just a different one - sweep `default_seed` itself:

```
sweep default --param default_seed --values 1,2,3,4,5 --seeds 400
```

### Rule 2 - compare paired, always

Two arms of a study run the **same seed set**, so arm A and arm B are matched trial for trial:
same map, same dice, one dial different. The difference is taken seed by seed:

$$
d_k = a_k - b_k, \qquad \mathrm{SE}(\bar d) = \frac{s_d}{\sqrt{n}}
$$

This is **common random numbers**, and it works because

$$
\mathrm{Var}(a - b) = \mathrm{Var}(a) + \mathrm{Var}(b) - 2\,\mathrm{Cov}(a,b)
$$

The two arms share the map and most of the luck, so the covariance is large and positive and most
of the variance cancels. Comparing the two *unpaired* means throws that away and can be an order
of magnitude noisier.

The harness offers `paired()` and **no unpaired comparison at all**, and it panics if the two arms
are different lengths rather than quietly comparing what it has.

### The corollary that catches people

**Compare two arms against each other, never by eye across a shared baseline.** A sweep prints
every arm against the *first* one, which invites reading the difference between two rows. Their
separate errors do not combine the way a paired difference does.

On the allocation comparison, greedy and optimal differ by **0.405 ± 0.051** measured directly,
but read off their shared `independent` baseline the gap looks like 0.4 against standard errors of
about 0.23 - five times noisier, and enough to hide a real effect completely. To compare two other
arms, **re-run with one of them first**.

## 12. The study tools

Four general tools on a shared harness, plus a findings checker and some bespoke probes. Run them
all with `cargo run -p experiments --release --bin <name> -- ...`.

| Tool | Answers |
|---|---|
| `batch` | How do these **scenarios** compare? The regression sweep |
| `sweep` | What is this **one dial** worth? |
| `factorial` | What are these **several dials** worth, and do they interact? |
| `sensitivity` | Which dials drive the answer **at all**? |
| `meta` | Which of **all** the dials should I even be looking at? |
| `findings` | Do the **documented numbers** still hold? |

### 12.1 `batch` - a folder of scenarios

```
batch [dir] [--seeds N] [--until SECONDS] [--out DIR] [--only NAME] [--quiet]
```

```
cargo run -p experiments --release --bin batch -- scenarios --seeds 50
cargo run -p experiments --release --bin batch -- scenarios --only air_raid --seeds 10000
```

Writes `out/<scenario>.csv`, a row per seed with every metric, and `out/summary.csv`, a row per
scenario with mean and standard error. `.gitignore` already covers `out/` and `*.csv`.

`batch` compares **scenarios**. It is the regression sweep - run it after a change to see whether
anything moved that should not have. To compare **dials**, use `sweep`.

### 12.2 `sweep` - one dial, many values

```
sweep <scenario> --param PATH (--values a,b,c | --from X --to Y [--steps N])
                 [--seeds N] [--until SECONDS] [--metric NAME] [--set PATH=VALUE]...
                 [--dir DIR] [--out DIR] [--quiet] [--quantiles 50,90,95]
```

`--param` is a **dotted path**, into either the scenario or a stat-block library:

```bash
sweep air_raid  --param sim.track_hold_s     --values 10,20,45,90         --seeds 500
sweep default   --param sim.p_suppress       --from 0.05 --to 0.8 --steps 8 --seeds 2000
sweep air_raid  --param red.air.0.altitude_m --values 150,300,600,1200    --seeds 1000
sweep ad_c2     --param sim.allocation       --values greedy,optimal      --seeds 1000 \
                --set sim.max_batteries_per_air_target=1

# stat blocks: the sensor, weapon and terrain dials the models actually turn on
sweep air_raid  --param sensors.mast_optical.lambda0_per_s --values 0.05,0.1,0.35,1.0 --seeds 1000
sweep default   --param weapons.mortar.cep_m --from 20 --to 140 --steps 7 --seeds 2000
sweep default   --param terrain_types.trees.concealment --values 0.2,0.5,0.8 --seeds 1000
```

**The first segment decides which file is patched.** `sensors`, `units`, `weapons`, `air`,
`air_defence`, `c2` and `terrain_types` name library files; anything else is a scenario path. The
two namespaces cannot collide, because a scenario's top level is only `name`, `default_seed`,
`terrain`, `sim`, `blue` and `red`. So `air.recce.speed_m_s` is the *stat block* and
`red.air.0.speed_m_s` is one airframe's override of it.

The override is applied to the **TOML, before it is parsed**. Three consequences:

- **Any field is sweepable**, including dials added after this document was written. `sweep` has
  no knowledge of the schema.
- Numeric path segments index arrays (`red.air.0.altitude_m`), and string-valued dials work as-is
  (`sim.allocation=greedy`).
- The patched scenario goes through **exactly the same loader** as a file on disk, so an
  out-of-range value fails the same way rather than reaching the simulation.

A dial absent from the file - most of them, since nearly all have a default - is created. That is
safe because `deny_unknown_fields` is set throughout: `sim.track_hold` for `sim.track_hold_s` is a
load error naming the key, not a silent default.

`--set` is repeatable and applies to **every** arm, for holding one dial away from the scenario's
own setting while varying another. `--metric` chooses which column the paired report is about;
every column is still in the CSV.

Note that a `terrain_types` sweep changes the **map**, not just what happens on it, so each arm
rebuilds terrain. A legitimate question, just a slower and noisier one.

### 12.3 `factorial` - several dials at once

`sweep` answers *what does this dial do*. `factorial` answers *what do these dials do, and does
either one's answer depend on the other* - a different question, and on this model it has more
than once been the more important one.

```
factorial <scenario> --factor PATH=v1,v2 [--factor PATH=v1,v2]...
                     [--seeds N] [--until SECONDS] [--metric NAME] [--set PATH=VALUE]...
```

```bash
cargo run -p experiments --release --bin factorial -- fires_c2 \
    --factor sim.fires_need_c2=false,true \
    --factor blue.sensors.0.type=mast_optical,ciws_radar \
    --seeds 500 --until 300 --metric red_cleared_s
```

Every combination of levels is a **cell**, and every cell runs the same seed set. Cost is the
product of the level counts: three two-level factors is eight cells.

It reports **main effects** first, each averaged over every level of the other factors - so a
factor is described by what it does across the design, not at one corner of it - and
**interactions** second, as the difference of differences formed per seed.

**The closing line says whether any interaction is significant, and that decides whether the main
effects above may be read on their own.** If two dials interact, "this one is worth −11 s" is a
sentence with a missing clause.

A factor may have more than two levels. Main effects are then reported per level against the
baseline, and the interaction is the corner-to-corner contrast across each factor's **range** - a
summary rather than the whole surface. The per-cell CSV holds the rest.

### 12.4 `sensitivity` - which dials drive the answer at all?

Every number in this project is an abstract placeholder. That is deliberate and said everywhere,
but it leaves one question over every finding: *does it matter that the numbers are invented?*

Neither `sweep` nor `factorial` can answer it. Both vary a few dials with the rest pinned wherever
the scenario happened to leave them, so both measure a slice through a space they never explore.
Global sensitivity analysis explores the space and reports what share of the outcome's variance
each dial is responsible for.

```
cargo run -p experiments --release --bin sensitivity -- studies/sensing.toml --seeds 20
```

**The dial space is a file, not a pile of flags**, because it is a design - something to commit,
review and re-run. See [`studies/README.md`](../studies/README.md) for the format.

It reports **Morris** first, because it is cheap: `mu*` ranks dials by how much they move the
answer, `sigma` flags one whose effect depends on where the others are. Its job is to say what can
be ignored before the expensive pass runs. Then **Sobol**, as a variance decomposition: `S1` is
what a dial explains alone, `ST` what it is involved in altogether, and `ST − S1` the share running
*through* interactions - which is exactly what a one-dial sweep cannot see.

**The closing line adds the first-order indices up.** Near 1 means the dials are additive and
one-at-a-time sweeps are sound. Well below 1 means most of the variance lives in interactions, and
a sweep will mislead.

A slightly **negative** `S1` means "indistinguishable from zero" - the estimator is unbiased rather
than non-negative, and clamping it would hide how noisy a near-zero index is.

**Cost** is `(morris points + sobol points) × seeds`, and `--seeds` is deliberately modest: a
design point is an average over seeds, and the variance being decomposed is the one across the
*dial space*, not across the dice. The shipped `sensing.toml` study is about thirteen minutes.

### 12.5 `meta` - screening all the dials

`sweep`, `factorial` and `sensitivity` all need to be *told* which dials to vary, which assumes
the interesting dials are already known - the assumption most worth testing.

```
cargo run -p experiments --release --bin meta -- air_raid --metric air_leakers
```

`meta` enumerates **every** dial the loaded libraries expose and screens them all by Morris
elementary effects, finishing with a line like "9 of 123 dials carry 90% of the total effect".

**Screening, not decomposition.** Sobol costs `n(k+2)` design points; at 123 dials that is millions
of trials. Morris costs `(k+1) × trajectories` and answers the question that comes first: *what can
be ignored?* Take the survivors to `sensitivity` for the variance decomposition. That two-stage
practice is standard, and it is what the tools are shaped for.

`--emit FILE` writes the screened dial space as a `studies/*.toml`, each row carrying the dial's
meaning, unit and theory section, so the second stage does not have to be retyped.

**What it leaves out, and says so.** Flags and named choices have no gradient and no midpoint -
`optimal` is not halfway between `greedy` and `independent` - so they are excluded and *listed*
rather than silently dropped. They belong in a `factorial`. So are dials whose current value is
zero, since a relative range around nothing is nothing.

### 12.6 `findings` - do the documented numbers still hold?

```
cargo run -p experiments --release --bin findings
cargo run -p experiments --release --bin findings -- --quick
```

Re-runs every finding in [`findings.toml`](../findings.toml) and reports any whose measured value
has moved outside the tolerance its author set. Exits non-zero on drift, so it can be run on a
schedule and noticed.

`--quick` cuts the seeds to a tenth. That is enough to catch a finding that has *broken* - a
renamed dial, a deleted scenario, an inverted sign - and far too few to judge drift, which is what
it says when it finishes.

This is not a `cargo test`: the full pass is around 18,000 trials. Run it before a release or
overnight, not on every edit.

### 12.7 The bespoke probes and the benchmarks

Each answers one question its own way and prints a table. They are kept because each does something
`sweep` and `factorial` cannot - search over *positions*, solve a game, or print a closed form
beside the measurement.

```
cargo run -p experiments --bin duel_probe          # a direct-fire duel, pair by pair
cargo run -p experiments --bin sensor_siting       # what a sensor position is worth
cargo run -p experiments --bin interdiction        # the sensing-against-routing game equilibrium
cargo run -p experiments --release --bin air_raid  # a drone raid against air defence
```

```
cargo run -p experiments --release --bin bench        # LOS, viewshed, slant range, tick cost
cargo run -p experiments --release --bin fires_bench  # the fires path alone
```

`bench` also reports the line-of-sight memo hit rate. **Terrain build time is the figure that
matters** - the simulation tick is sub-millisecond and too noisy to optimise against. The fires
path gets its own bench because the tick bench is too sensing-dominated to resolve it.

## 13. Reading the output

```
sim.allocation = greedy      -12.835 +- 0.224 (t = -57.2, n = 2000, 311 tied) significant
```

- **`-12.835`** - the mean paired difference against the *first* arm, in the metric's units.
- **`+- 0.224`** - its standard error. Roughly, the true value is within about two of these.
- **`t = -57.2`** - mean divided by standard error. $|t| > 2$ is the line this harness calls
  significant, two-sided at about 5%. At 57 there is nothing to argue about.
- **`311 tied`** - seeds where the two arms gave *exactly* the same number.
- **`significant` / `NOT significant`**.

**The tie count is the part people skip, and it is the most informative field.** A small difference
with a *high* tie count means the two arms are mostly making the same decision - a different
conclusion from "the effect is real but hard to see". When every arm ties on every seed, the report
says so explicitly, because "no significant effect" reads as evidence of no effect when it is
usually evidence that **the dial does not reach the metric in this scenario at all**.

More seeds shrink the standard error as $1/\sqrt{n}$: to halve the error bar, quadruple the seeds.
If an effect is not visible at 2,000 seeds it is small enough that the honest answer is usually
"smaller than anything else in this model".

### 13.1 What the columns mean

Every metric is read back from the simulation's own event logs and final state, never accumulated
alongside it as it runs. So there is no second bookkeeping path to drift: if a metric is wrong, the
log is wrong, and the app's event feed is showing the same wrong thing.

| Column | Meaning |
|---|---|
| `blue_losses`, `red_losses` | Ground sub-elements destroyed |
| `blue_units_killed`, `red_units_killed` | Whole units reduced to zero elements |
| `detections` | Detection events, both sides, ground and air |
| `first_detection_s` | Time of first contact either way; the run length if none |
| `fire_events` | Fires resolutions that produced casualties |
| `air_launched`, `air_downed` | Airframes placed, and shot down |
| `air_leakers` | Airframes that survived to **release** a munition |
| `munitions_released` | Munitions released |
| `ground_casualties_from_air` | Elements killed by air-delivered bursts |
| `ad_shots` | Air-defence shots taken - the denominator for rounds per kill |
| `ad_rounds_left` | Interceptors remaining, finite magazines only |
| `ad_batteries_killed`, `c2_posts_killed` | What SEAD is trying to achieve |
| `blue_cleared_s`, `red_cleared_s` | When a side lost its last ground element |
| `epochs` | Decision epochs resolved |

Two repay attention.

**`*_cleared_s` is usually the metric that answers "was this better?"** Losses saturate: if
everything on one side dies by 600 s in every arm, `red_losses` is the same number everywhere and
only the *time* distinguishes them. That is how the allocation result had to be measured. A side
that was never cleared reports the run length, so read it with the kill counts beside it - `600`
means "not by 600 s", not "at 600 s". A side fielding no ground units also reports the run length;
there was nothing to clear.

**`ad_rounds_left` is where the C2 result lives.** Coordinating air defence barely changes how many
drones die; it changes how much ammunition is left afterwards, because a missile is a discrete
round and overkill is real. Coordination pays where the shot is a countable resource - and that is
invisible without this column.

### 13.2 How fast, and why

Measured on a twelve-thread machine, release build:

| Study | Trials | Wall time | Rate |
|---|---|---|---|
| `sweep ad_c2 --values 1,2,3,4 --seeds 2500` | 10,000 | 17.8 s | 562/s |
| `batch --only air_raid --seeds 2000` | 2,000 | 8.1 s | 248/s |
| `sweep fire_allocation` (3 arms, 500 seeds) | 1,500 | 0.3 s | 4,715/s |

A trial costs microseconds; **building the terrain costs seconds**. So the seed list is cut into
exactly one chunk per worker thread, and each worker builds one simulation and resets it between
trials. Every worker builds terrain from `default_seed`, so all workers get the **same** map.

**Scheduling cannot change the answer.** Results come back in seed order regardless of how the work
was split, and each trial's RNG stream depends only on its seed. A parallel study returns
byte-identical numbers to a serial one, and a test pins that.

The other lever is `--until`. Most scenarios have decided by 200-300 s and the rest of the run is
empty ticks, so halving `--until` nearly halves the cost. Check `*_cleared_s` first to see whether
time is being spent on nothing happening.

## 14. A worked study, end to end

The question: **is the air-defence overkill cap earning its keep?**

`max_batteries_per_air_target` caps how many batteries may be assigned to one airframe. It defaults
to 2, on the reasoning that a second battery is insurance against the first missing and a third is
nearly always waste. That is a claim, so measure it.

```bash
cargo run -p experiments --release --bin sweep -- ad_c2 \
    --param sim.max_batteries_per_air_target --values 1,2,3,4 \
    --seeds 2500 --metric ad_rounds_left
```

10,000 trials, 17.8 s:

| Cap | `air_downed` | `ad_rounds_left` |
|---|---|---|
| 1 | 9.907 (baseline) | 3.679 (baseline) |
| 2 | −0.002 ± 0.007 - **not significant** | −0.252 ± 0.040 (t = −6.3) |
| 3 | −0.028 ± 0.008 (t = −3.7) | −0.642 ± 0.042 (t = −15.2) |
| 4 | −0.028 | −0.650 - identical to cap 3 |

Read it in three parts.

**Cap 4 is exactly cap 3**, because the scenario has three batteries. That is the harness
confirming it is measuring what it claims to - and it is worth looking for that kind of sanity
check in every study.

**The second battery buys nothing and costs a quarter of a round.** −0.002 kills against a standard
error of 0.007 is a null result at 2,500 paired seeds - not "too small to see", genuinely nothing.

**The third battery is actively worse**: it costs 0.64 rounds of reserve *and* kills 0.028 fewer
drones. Stacking is not free even when ammunition is not the binding constraint, because a battery
committed to an airframe another battery has already covered is not covering a different one.

So on this scenario the default of 2 is defensible but unearned. Whether that holds when batteries
are scarcer relative to the raid is the next question, and it is one `--set` away.

### 14.1 A stat-block dial: what is a better sensor worth?

The same command shape reaches the models themselves.

```bash
cargo run -p experiments --release --bin sweep -- air_raid \
    --param sensors.mast_optical.lambda0_per_s --values 0.05,0.1,0.35,1.0 \
    --seeds 1000 --metric air_leakers
```

| $\lambda_0$ (per s) | Leakers, paired against 0.05 |
|---|---|
| 0.05 | 0.703 (baseline) |
| 0.1 | −0.029 ± 0.023 - not significant |
| 0.35 | −0.087 ± 0.032 (t = −2.7) |
| 1.0 | −0.176 ± 0.031 (t = −5.6) |

A twentyfold better sensor stops a quarter of the leakers. **Note the shape**: the first doubling
is worth nothing measurable, and it takes a factor of seven before the effect clears the noise.
That is what $P = 1 - e^{-\lambda \Delta t}$ looks like from the outside - the sensor is not the
binding constraint until it is bad enough to be one, and after that each increment matters less
than the last.

## 15. Keeping findings honest

A measured finding is a claim about a model at a moment. The model then changes, and unless
something re-runs the claim, a number that was right when written goes on being quoted after it
stopped being true.

`findings.toml` pins each documented claim to the paired comparison that produced it - scenario,
dial, two arms, metric, seeds, the expected difference and a tolerance:

```toml
[[finding]]
id = "allocation-coordination-pays"
claim = "Coordinating fires clears the enemy about 12.8 s sooner than the old nearest-enemy rule."
documented_in = ["README.md", "docs/THEORY.md"]
scenario = "fire_allocation"
param = "sim.allocation"
arms = ["independent", "greedy"]
metric = "red_cleared_s"
seeds = 2000
expect = -12.835
tolerance = 0.8
```

**The tolerance is not a confidence interval.** The run computes its own standard error. The
tolerance is *your* statement of how far the number may move before the prose around it stops
being true, which is a different and more useful question.

**Add an entry whenever you write a number into a document.** A drift report lists every document
repeating the number, because fixing a stale finding is mostly a matter of finding all the places
it was copied to.

---
# Part 4 - Maintaining it

## 16. Testing, linting, releasing

```
cargo test --workspace                 # all of it
cargo test -p validation               # the V1-V77 gates
cargo test -p sim_core                 # engine unit tests (fast, headless)
cargo test -p experiments              # harness: parallel-equals-serial, patching, statistics
cargo test -p app                      # selection and picking
```

```
cargo run -p validation --release --bin validation_report
```

That last one is the one worth running. It prints every gate beside **the closed form it is
checked against**, because the useful question is not "are the tests green" but *is the maths
still right, and right against what*.

**If you change a model and a gate fails, understand why before re-baselining it.** That gate is
the only thing standing between a model and a plausible-looking number that is quietly wrong. See
[`docs/VALIDATION.md`](VALIDATION.md) for what each is checked against and how to add one.

### Lint and format

```
cargo clippy --workspace                 # take its advice seriously
cargo clippy --workspace --all-targets   # including tests and benches
cargo fmt                                # rustfmt; format-on-save is configured
```

### Building for release

```
cargo build -p app --release
```

**Remove the `dynamic_linking` feature from `crates/app/Cargo.toml` first.** It makes iterative
builds much faster and produces a binary that will not run anywhere else.

### A note on determinism

Everything above is reproducible from (scenario, seed). Nothing reads the wall clock, nothing
depends on thread scheduling, and the parallel rasters and parallel trials are checked to produce
bit-identical results to their serial equivalents. **If two runs of the same command disagree,
that is a bug, not variance.**

## 17. Extending the harness

### 17.1 Adding a metric

Four edits, all in [`crates/experiments/src/metrics.rs`](../crates/experiments/src/metrics.rs):

1. A field on `Outcome`.
2. A name in `COLUMNS`.
3. A line in `Outcome::values()`, in the same position.
4. Bump `N`.

`COLUMNS` and `values()` are tied together by the array length `N`, so forgetting one of them will
not compile. Fill the field in `run_one` **from the simulation's logs or final state** - that is
the rule that keeps the metrics honest.

Everything else picks it up automatically: both CSVs, the summary, and `--metric <name>`.

### 17.2 Adding an experiment

For a question `sweep` cannot phrase, add a binary in `crates/experiments/src/bin/`. **Use the
harness rather than reimplementing it:**

```rust
use experiments::{stats::paired, study::{column, run_study, StudyConfig}};

let cfg = StudyConfig { seeds: 1000, until_s: 600.0, progress: true };
let a = run_study(&scenario_a, &libs, cfg)?;
let b = run_study(&scenario_b, &libs, cfg)?;
let metric = experiments::Outcome::column("red_cleared_s").unwrap();
println!("{}", paired(&column(&a, metric), &column(&b, metric)).report());
```

Both arms ran seeds `0..1000`, so they are paired by construction. That is the point of
`StudyConfig::seeds` being a count rather than a range.

Every bespoke binary that predated the shared harness went stale without anyone noticing, and three
were eventually deleted once the harness subsumed them. **A probe that does not use the harness is
a probe that will quietly stop agreeing with everything else.**

### 17.3 Adding a validation gate

See [`docs/VALIDATION.md`](VALIDATION.md#adding-one). In outline: write the model *and its
reference* together, state the gate in its `docs/THEORY.md` section, add the test to
`crates/validation/tests/`, add the `Gate {…}` entry to `gates.rs`, and regenerate the catalogue.
A test that is not catalogued, or a catalogued gate with no test, is a build failure rather than a
silent gap.

### 17.4 What the harness cannot do yet

- **Two-way interactions only.** `factorial` reports every pair of factors; a three-way interaction
  is in the per-cell CSV but not in the report.
- **Sensitivity dials are continuous only.** A range is a pair of numbers, so a categorical dial
  like `sim.allocation` has no place in a study file - use `factorial` for those.
- **A sensitivity study may not vary terrain.** Terrain is built once for the whole design, so a
  terrain dial would ask for a map it does not get.
- **`--seeds N` always means seeds `0..N`**, so two studies at different `N` share a prefix rather
  than being independent. That is deliberate - it is what makes arms pairable - but it means "run
  1,000 more seeds" is `--seeds 2000`, not a second run.
- **No confidence intervals on quantiles**, only on means.

## 18. Troubleshooting

### Setup

| Symptom | Cause and fix |
|---|---|
| First build feels frozen | It is not. Compiling the engine takes minutes, once. |
| rust-analyzer shows no types, or errors everywhere | Still indexing on first open. It can take a few minutes and a lot of RAM. |
| Windows link errors | Missing MSVC C++ Build Tools. Reinstall them. |
| An ecosystem crate will not compile | Version mismatch with Bevy. Check that crate's README for the Bevy version it targets. |
| A release build will not run elsewhere | `dynamic_linking` was left on. Disable it for release. |
| A large download fails mid-stream | Known on this machine. Retry - resume loops work. |
| Tests fail on a scenario path that no longer exists | The project folder was moved. `env!("CARGO_MANIFEST_DIR")` is baked in at compile time, so a cached test binary still looks for the old location. `cargo clean -p sim_core` and re-run. |

### Scenarios

| Symptom | Cause and fix |
|---|---|
| Load error naming a key | A misspelt dial. Every schema sets `deny_unknown_fields` - this is the loader doing its job. |
| Load error naming a value | The input contract. A dial the model cannot run on: a zero where a divisor needs one, a probability outside $[0,1]$. |
| Load error listing valid names | A doctrine `priority` entry matching nothing on the field. |
| Load error about `route` and `objective` | A unit declared both. Pick one; they mean different things. |
| An artillery unit sits silent | Either the target is inside `min_range_m`, or **nobody is watching it**. Indirect fire needs a track, not a sightline. |
| A drone flies its route and hits nothing | It had no `target` and is not `autonomous`, so it released at its final waypoint. |
| A scenario opens flat when a ridge was intended | The key inside `[[terrain.source.layers.apply]]` is `apply`, not `features`. Now rejected rather than ignored. |

### Studies

| Symptom | Cause and fix |
|---|---|
| Every arm ties on every seed | The dial does not reach the metric in this scenario. Not "no effect" - **no mechanism**. Build a scenario where the choice matters. |
| Both arms read 0.00 | The mechanism never fired. The classic case is a drone shot down before it releases, so the seeker under test was never exercised. **Check the intermediate metrics, not just the headline one.** |
| An effect looks real but the standard error is huge | You may be reading two arms across a shared baseline. Re-run with one of them first (§11). |
| The study is far slower than expected | Either you left off `--release`, or `--until` is long past the point where anything happens. Check `*_cleared_s`. |
| A sweep hangs on its first arm | A clock dial swept from zero. This is what the input contract now refuses at load. |
| A runner exits immediately, naming a flag | A flag was given a missing or unparseable value - `--seeds abc`, `--until 60O`. The runners **exit with status 2** rather than falling back to the default, because a run that silently used 200 seeds would succeed and answer a different question. |
| Results changed after parallelising | They cannot have. That is a bug - a test pins parallel-equals-serial byte for byte. |

**The most expensive failure mode in this project has been a scenario that measures nothing**, and
it does not announce itself: both arms return a number, the standard errors look fine, and the
conclusion is that the dial does not matter. Release range and engagement envelope are a matched
pair; a top-priority target that is never detected is never shot at; a cap of 4 on three batteries
is a cap of 3. **A scenario that measures nothing is worse than no scenario, because it looks like
evidence.**

---

*Back to [README](../README.md) · [Model](MODEL.md) · [Reference](REFERENCE.md) · [Theory](THEORY.md) · [Validation](VALIDATION.md)*
