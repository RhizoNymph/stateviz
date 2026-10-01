//! `cascade render`: build a view's scene and write it as SVG or PNG.
//!
//! `--view` picks the view; `--state` applies a `cascade://` link
//! (selection, cone, filters, search, …) whose own view is overridden by
//! `--view`. The trace view simulates `--scenario`; with `race=N` in the
//! link it replays the N-th race candidate in both orders side by side.
//! With `diff=<base>,<head>` in the link it renders diff mode: the file at
//! `head` (default: the working tree) merged with the ghosts of what `base`
//! had, decorated added/removed/changed. Findings are not drawn in diff
//! mode, because the merged model's ghosts would distort them.
//! Scene notes (e.g. "Pick a scenario to trace.") go to standard error.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, bail};

use cascade_core::diff::{ModelDiff, merge_for_display};
use cascade_core::{CausalGraph, Check, Model, analyze, load_str};
use cascade_scene::{
    DiffRefs, MonoMeasure, SceneBuilder, SceneInput, Theme, ViewKind, ViewState, load_sidecar, sidecar_path,
};
use cascade_sim::Trace;

use crate::commands::load_or_report;

pub struct RenderArgs {
    pub file: PathBuf,
    pub view: String,
    pub state: Option<String>,
    pub scenario: Option<PathBuf>,
    pub out: PathBuf,
    pub dark: bool,
    /// Build mode: the structure view with its wiring band.
    pub edit: bool,
    pub scale: f32,
}

/// What to write, from the output file's extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Svg,
    Png,
}

pub fn run(args: &RenderArgs) -> anyhow::Result<ExitCode> {
    let format = match extension(&args.out).as_deref() {
        Some("svg") => Format::Svg,
        Some("png") => Format::Png,
        _ => bail!("--out must end in .svg or .png"),
    };
    let model = match load_or_report(&args.file)? {
        Ok(model) => model,
        Err(code) => return Ok(code),
    };
    let mut view = match &args.state {
        Some(link) => ViewState::from_link(link).context("invalid --state link")?,
        None => ViewState::default(),
    };
    view.view = args.view.parse::<ViewKind>().context("invalid --view")?;

    let (model, diff) = match &view.diff {
        Some(refs) => {
            let (merged, diff) = diff_mode(&args.file, refs)?;
            (merged, Some(diff))
        }
        None => (model, None),
    };
    let graph = CausalGraph::build(&model);
    let findings = if diff.is_some() { Vec::new() } else { analyze(&model, &graph) };
    let traces = match &args.scenario {
        Some(path) => traces(&model, &findings, path, view.race)?,
        None => Vec::new(),
    };
    let theme = if args.dark { Theme::dark() } else { Theme::light() };
    let sidecar = load_sidecar(&sidecar_path(&args.file))?;
    let input = SceneInput {
        model: &model,
        graph: &graph,
        findings: &findings,
        view: &view,
        theme: &theme,
        measure: &MonoMeasure::default(),
        sidecar: &sidecar,
        traces: &traces,
        mode: if args.edit { cascade_scene::SceneMode::Edit } else { cascade_scene::SceneMode::View },
        play: None,
        diff: diff.as_ref(),
    };
    let scene = SceneBuilder::new().build(&input)?;
    for note in &scene.notes {
        eprintln!("note: {note}");
    }

    let bytes = match format {
        Format::Svg => cascade_scene::to_svg(&scene)?.into_bytes(),
        Format::Png => cascade_scene::to_png(&scene, args.scale)?,
    };
    std::fs::write(&args.out, bytes).with_context(|| format!("cannot write {}", args.out.display()))?;
    Ok(ExitCode::SUCCESS)
}

/// The definition at `refs.head` (or the working tree) merged with the
/// ghosts of the definition at `refs.base`.
fn diff_mode(file: &Path, refs: &DiffRefs) -> anyhow::Result<(Model, ModelDiff)> {
    let old_text = cascade_interop::read_at_rev(file, &refs.base)?;
    let new_text = match &refs.head {
        Some(rev) => cascade_interop::read_at_rev(file, rev)?,
        None => std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?,
    };
    let old = load_str(&old_text).with_context(|| format!("the definition at {} is invalid", refs.base))?;
    let head = refs.head.as_deref().unwrap_or("the working tree");
    let new = load_str(&new_text).with_context(|| format!("the definition at {head} is invalid"))?;
    merge_for_display(&old, &new).context("cannot merge the two versions for display")
}

/// Simulate the scenario, or replay a race candidate in both orders.
fn traces(
    model: &Model,
    findings: &[cascade_core::Finding],
    path: &Path,
    race: Option<u32>,
) -> anyhow::Result<Vec<Trace>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let scenario = cascade_sim::parse_scenario(&text)?;
    match race {
        None => Ok(vec![cascade_sim::simulate(model, &scenario)?]),
        Some(index) => {
            let detail = findings
                .iter()
                .filter(|f| f.check() == Check::RaceCandidate)
                .nth(usize::try_from(index).unwrap_or(usize::MAX))
                .map(|f| &f.detail)
                .with_context(|| format!("no race candidate #{index}"))?;
            let both = cascade_sim::race_orderings(model, &scenario, detail)?;
            Ok(vec![both.as_queued, both.swapped])
        }
    }
}

fn extension(path: &Path) -> Option<String> {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase())
}
