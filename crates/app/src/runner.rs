//! The experiment runner: browse the dials, build a study, queue a batch, watch it run.
//!
//! # What it is for
//!
//! Setting up a batch of related experiments. One sweep is a command worth typing; six
//! related sweeps is an afternoon of remembering dotted paths, and that is the job this does.
//!
//! # The rule it is built to
//!
//! It **shows the command it is about to run**, and a queue can be saved as a shell script.
//! Nothing here is a second way of doing experiments - it is a way of writing down the
//! existing one. An experiment that exists only as clicks is not reproducible, and every
//! finding this project keeps depends on being able to re-run it.
//!
//! So [`experiments::runner::RunSpec`] is the real object, and it lives in `experiments`
//! where it can be tested without a window. This file renders it.

use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future::poll_once, AsyncComputeTaskPool, Task};
use bevy_egui::egui;
use experiments::dials::{self, Dial, Range};
use experiments::outcome::COLUMNS;
use experiments::runner::{Batch, Kind, RunSpec};
use sim_core::scenario::Libraries;

use crate::state::SimRes;

/// Everything the runner window holds between frames.
#[derive(Resource)]
pub struct Runner {
    /// Is the window open?
    pub open: bool,
    /// Every dial the loaded libraries expose, listed once at startup.
    dials: Vec<Dial>,
    /// Filter text for the dial browser.
    filter: String,
    /// The study being composed.
    draft: RunSpec,
    /// Values being typed for a sweep, before they are split on commas.
    values_text: String,
    /// Factors being typed for a factorial, one per line.
    factors_text: String,
    /// Dials pinned for every arm, one `path=value` per line.
    fixed_text: String,
    /// The queue, in the order it will run.
    queue: Batch,
    /// The run in flight, if any.
    running: Option<Running>,
    /// Finished runs, newest last.
    log: Vec<Finished>,
    /// Scenario names, for the picker.
    scenarios: Vec<String>,
}

/// A run currently executing on the task pool.
struct Running {
    label: String,
    command: String,
    task: Task<Result<String, String>>,
}

/// What a finished run left behind.
struct Finished {
    label: String,
    command: String,
    outcome: Result<String, String>,
}

impl Runner {
    /// Build the runner's state from the libraries already loaded for the map.
    #[must_use]
    pub fn new(libs: &Libraries, scenarios: Vec<String>, current: &str) -> Self {
        Self {
            open: false,
            dials: dials::enumerate(libs),
            filter: String::new(),
            // Start on the scenario already open, which is nearly always the one being
            // asked about - the map and the study are two views of the same question.
            draft: RunSpec {
                scenario: current.to_owned(),
                ..RunSpec::default()
            },
            values_text: String::new(),
            factors_text: String::new(),
            fixed_text: String::new(),
            queue: Batch::default(),
            running: None,
            log: Vec::new(),
            scenarios,
        }
    }

    /// Fold the free-text fields into the draft, so the command shown is always current.
    fn sync_draft(&mut self) {
        self.draft.fixed = self
            .fixed_text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect();
        match &mut self.draft.kind {
            Kind::Sweep { values, .. } => {
                *values = experiments::patch::split_values(&self.values_text)
                    .into_iter()
                    .filter(|v| !v.trim().is_empty())
                    .collect();
            }
            Kind::Factorial { factors } => {
                *factors = self
                    .factors_text
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            Kind::Batch { .. } => {}
        }
    }
}

/// Draw the runner window and drive its queue.
pub fn runner_window(
    mut contexts: bevy_egui::EguiContexts,
    mut runner: ResMut<Runner>,
    keys: Res<ButtonInput<KeyCode>>,
    sim: Res<SimRes>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    drive_queue(&mut runner);

    // `E` toggles the window. Not while egui wants the keyboard, or typing a dial path into
    // the runner's own filter would close it under the cursor.
    if keys.just_pressed(KeyCode::KeyE) && !ctx.egui_wants_keyboard_input() {
        runner.open = !runner.open;
    }

    // The screenshot rig captures a frame a few seconds in; open the window there so a
    // capture shows it rather than an empty map.
    if std::env::var_os("FIRES_SIM_SHOW_RUNNER").is_some() {
        runner.open = true;
    }
    if !runner.open {
        return;
    }

    let mut open = runner.open;
    egui::Window::new("Experiments")
        .open(&mut open)
        .default_size([560.0, 700.0])
        .default_pos([300.0, 20.0])
        .vscroll(true)
        .show(ctx, |ui| {
            runner.sync_draft();
            build_section(ui, &mut runner);
            ui.separator();
            // Above the queue on purpose: finding the dial is the first thing anyone does,
            // and it is the reason to open this window rather than type the command.
            dial_browser(ui, &mut runner);
            ui.separator();
            queue_section(ui, &mut runner);
            ui.separator();
            log_section(ui, &runner);
        });
    runner.open = open;
    let _ = &sim; // the window reads libraries at construction, not per frame
}

/// Compose one study, and show exactly what it will run.
fn build_section(ui: &mut egui::Ui, runner: &mut Runner) {
    ui.label(egui::RichText::new("Build a study").strong());

    ui.horizontal(|ui| {
        ui.label("label");
        ui.text_edit_singleline(&mut runner.draft.label);
    });

    ui.horizontal(|ui| {
        egui::ComboBox::from_label("scenario")
            .selected_text(runner.draft.scenario.clone())
            .show_ui(ui, |ui| {
                for s in &runner.scenarios {
                    ui.selectable_value(&mut runner.draft.scenario, s.clone(), s);
                }
            });
        egui::ComboBox::from_label("metric")
            .selected_text(runner.draft.metric.clone())
            .show_ui(ui, |ui| {
                for m in COLUMNS {
                    ui.selectable_value(&mut runner.draft.metric, (*m).to_owned(), m.to_string());
                }
            });
    });

    ui.horizontal(|ui| {
        let is = |k: &Kind, name: &str| k.binary() == name;
        if ui
            .selectable_label(is(&runner.draft.kind, "sweep"), "sweep")
            .clicked()
        {
            runner.draft.kind = Kind::Sweep {
                param: String::new(),
                values: Vec::new(),
            };
        }
        if ui
            .selectable_label(is(&runner.draft.kind, "factorial"), "factorial")
            .clicked()
        {
            runner.draft.kind = Kind::Factorial {
                factors: Vec::new(),
            };
        }
        if ui
            .selectable_label(is(&runner.draft.kind, "batch"), "batch")
            .clicked()
        {
            runner.draft.kind = Kind::Batch { only: None };
        }
    });

    match &mut runner.draft.kind {
        Kind::Sweep { param, .. } => {
            ui.horizontal(|ui| {
                ui.label("dial");
                ui.text_edit_singleline(param);
            });
            ui.horizontal(|ui| {
                ui.label("values");
                ui.text_edit_singleline(&mut runner.values_text);
            });
            ui.label(
                egui::RichText::new("comma separated; pick a dial from the browser below")
                    .small()
                    .weak(),
            );
        }
        Kind::Factorial { .. } => {
            ui.label("factors, one per line as path=v1,v2");
            ui.text_edit_multiline(&mut runner.factors_text);
        }
        Kind::Batch { only } => {
            let mut text = only.clone().unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label("only scenarios containing");
                ui.text_edit_singleline(&mut text);
            });
            *only = (!text.trim().is_empty()).then_some(text);
        }
    }

    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut runner.draft.seeds).range(1..=20000));
        ui.label("seeds");
        ui.add(egui::DragValue::new(&mut runner.draft.until_s).range(1.0..=3600.0));
        ui.label("until (s)");
    });

    ui.label("pinned dials, one per line as path=value");
    ui.text_edit_multiline(&mut runner.fixed_text);

    runner.sync_draft();
    let problems = runner.draft.problems();
    let command = runner.draft.command();

    // The command is shown whether or not it is valid, because seeing it change as the form
    // is filled in is how the form teaches the tool.
    ui.add_space(4.0);
    ui.label(egui::RichText::new("will run").small().weak());
    ui.add(
        egui::TextEdit::multiline(&mut command.clone())
            .font(egui::TextStyle::Monospace)
            .desired_rows(2)
            .desired_width(f32::INFINITY),
    );

    ui.horizontal(|ui| {
        let ok = problems.is_empty();
        if ui
            .add_enabled(ok, egui::Button::new("Add to queue"))
            .clicked()
        {
            runner.queue.runs.push(runner.draft.clone());
        }
        if ui.button("Copy command").clicked() {
            ui.ctx().copy_text(command.clone());
        }
        if let Some(n) = runner.draft.trials() {
            ui.label(egui::RichText::new(format!("{n} trials")).small().weak());
        }
    });
    for p in &problems {
        ui.colored_label(egui::Color32::from_rgb(200, 140, 60), format!("- {p}"));
    }
}

/// The queue: what is waiting, what is running, and how to keep a record of it.
fn queue_section(ui: &mut egui::Ui, runner: &mut Runner) {
    let (total, uncounted) = runner.queue.size();
    ui.label(
        egui::RichText::new(format!(
            "Queue - {} run(s), {total} trials{}",
            runner.queue.runs.len(),
            if uncounted > 0 {
                format!(" plus {uncounted} of unknown size")
            } else {
                String::new()
            }
        ))
        .strong(),
    );

    let mut remove = None;
    for (i, r) in runner.queue.runs.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui.small_button("x").clicked() {
                remove = Some(i);
            }
            ui.label(format!("{}. {}", i + 1, r.label));
            ui.label(
                egui::RichText::new(format!("{} on {}", r.kind.binary(), r.scenario))
                    .small()
                    .weak(),
            );
        });
    }
    if let Some(i) = remove {
        runner.queue.runs.remove(i);
    }

    if let Some(active) = &runner.running {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(format!("running: {}", active.label));
        });
    }

    ui.horizontal(|ui| {
        let can_start = runner.running.is_none() && !runner.queue.runs.is_empty();
        if ui
            .add_enabled(can_start, egui::Button::new("Run queue"))
            .clicked()
        {
            start_next(runner);
        }
        if ui.button("Save as script").clicked() {
            save(&runner.queue.to_script(), "experiments.sh");
        }
        if ui.button("Save as TOML").clicked() {
            save(&runner.queue.to_toml(), "experiments.toml");
        }
        if ui.button("Clear").clicked() {
            runner.queue.runs.clear();
        }
    });
    ui.label(
        egui::RichText::new(
            "the script is the record: whatever was set up here, that file repeats it",
        )
        .small()
        .weak(),
    );
}

/// Every dial there is, filtered, with what it means and what it currently holds.
fn dial_browser(ui: &mut egui::Ui, runner: &mut Runner) {
    ui.label(egui::RichText::new(format!("Dials ({} available)", runner.dials.len())).strong());
    ui.horizontal(|ui| {
        ui.label("filter");
        ui.text_edit_singleline(&mut runner.filter);
    });

    let needle = runner.filter.to_lowercase();
    let matches: Vec<Dial> = runner
        .dials
        .iter()
        .filter(|d| needle.is_empty() || d.path.to_lowercase().contains(&needle))
        .take(40)
        .cloned()
        .collect();

    egui::ScrollArea::vertical()
        .max_height(220.0)
        .show(ui, |ui| {
            for d in matches {
                ui.horizontal(|ui| {
                    if ui.small_button("use").clicked() {
                        if let Kind::Sweep { param, .. } = &mut runner.draft.kind {
                            param.clone_from(&d.path);
                            // Offer the dial's own range as a starting pair of arms, so a
                            // sweep is one click from being runnable rather than a blank box.
                            runner.values_text = match d.range {
                                Range::Absolute(lo, hi) => format!("{lo},{hi}"),
                                Range::Integer(lo, hi) => format!("{lo},{hi}"),
                                Range::Choice(opts) => opts.join(","),
                                Range::Flag => "false,true".to_owned(),
                                Range::Relative(..) => String::new(),
                            };
                        }
                    }
                    ui.label(egui::RichText::new(&d.path).monospace());
                });
                ui.label(
                    egui::RichText::new(format!(
                        "    {} ({}{})",
                        d.doc,
                        d.section,
                        if d.unit.is_empty() {
                            String::new()
                        } else {
                            format!(", {}", d.unit)
                        }
                    ))
                    .small()
                    .weak(),
                );
            }
        });
}

/// What has finished, and what it said.
fn log_section(ui: &mut egui::Ui, runner: &Runner) {
    if runner.log.is_empty() {
        return;
    }
    ui.label(egui::RichText::new("Finished").strong());
    for f in runner.log.iter().rev().take(8) {
        match &f.outcome {
            Ok(summary) => {
                ui.label(format!("{} - {summary}", f.label));
            }
            Err(e) => {
                ui.colored_label(
                    egui::Color32::from_rgb(200, 100, 90),
                    format!("{} - {e}", f.label),
                );
            }
        }
        ui.label(egui::RichText::new(&f.command).monospace().small().weak());
    }
}

/// Start the next queued run on the task pool.
///
/// Shelling out to the binary rather than calling `run_study` in-process, deliberately: the
/// command shown is then the command run, with no second execution path that could drift
/// from it. It costs a process spawn per run, which against thousands of trials is nothing.
fn start_next(runner: &mut Runner) {
    if runner.queue.runs.is_empty() {
        return;
    }
    let spec = runner.queue.runs.remove(0);
    let command = spec.command();
    let label = spec.label.clone();
    let pool = AsyncComputeTaskPool::get();
    let cmd = command.clone();
    let task = pool.spawn(async move { run_command(&cmd) });
    runner.running = Some(Running {
        label,
        command,
        task,
    });
}

/// Poll the running task, record what it produced, and start the next one.
fn drive_queue(runner: &mut Runner) {
    let Some(active) = runner.running.as_mut() else {
        return;
    };
    let Some(outcome) = block_on(poll_once(&mut active.task)) else {
        return;
    };
    let done = runner.running.take().expect("checked above");
    runner.log.push(Finished {
        label: done.label,
        command: done.command,
        outcome,
    });
    // A batch runs to the end: one failure does not abandon the rest, because the point of
    // queueing six is to come back to six answers.
    if !runner.queue.runs.is_empty() {
        start_next(runner);
    }
}

/// Run one command, returning its last meaningful line or the error.
fn run_command(command: &str) -> Result<String, String> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut parts = shell_split(command);
    if parts.is_empty() {
        return Err("empty command".to_owned());
    }
    let program = parts.remove(0);
    let out = std::process::Command::new(program)
        .args(parts)
        .current_dir(root)
        .output()
        .map_err(|e| format!("could not start: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().last().unwrap_or("failed").to_owned());
    }
    Ok(text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("done")
        .to_owned())
}

/// Split a command into arguments, respecting the single quotes the spec uses to protect
/// values containing commas or brackets.
fn shell_split(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in command.chars() {
        match c {
            '\'' => quoted = !quoted,
            ' ' if !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Write a file next to the repository, reporting where it went.
fn save(contents: &str, name: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("out")
        .join(name);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, contents) {
        Ok(()) => info!("wrote {}", path.display()),
        Err(e) => warn!("could not write {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The quoting the spec emits and the splitting the runner does have to be inverses, or
    /// a value containing a comma would be passed as several arguments and the run would
    /// silently answer a different question.
    #[test]
    fn quoting_and_splitting_round_trip() {
        let spec = RunSpec {
            kind: Kind::Sweep {
                param: "sim.allocation".to_owned(),
                values: vec!["greedy".to_owned(), "optimal".to_owned()],
            },
            fixed: vec![r#"blue.doctrine.priority=["c2","armour"]"#.to_owned()],
            ..RunSpec::default()
        };
        let parts = shell_split(&spec.command());
        assert!(
            parts.contains(&"greedy,optimal".to_owned()),
            "the value list must survive as ONE argument: {parts:?}"
        );
        assert!(
            parts.contains(&r#"blue.doctrine.priority=["c2","armour"]"#.to_owned()),
            "a list-valued --set must survive as one argument: {parts:?}"
        );
    }

    #[test]
    fn splitting_handles_plain_arguments() {
        assert_eq!(
            shell_split("cargo run --bin sweep -- default"),
            vec!["cargo", "run", "--bin", "sweep", "--", "default"]
        );
    }
}
