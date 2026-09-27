//! `cascade render`: build a view's scene and write it as SVG or PNG.
//!
//! Owner: `feat/view-scenes`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, bail};

use cascade_core::{CausalGraph, analyze};
use cascade_scene::{MonoMeasure, SceneBuilder, SceneInput, Theme, ViewKind, ViewState, load_sidecar, sidecar_path};

use crate::commands::load_or_report;

pub struct RenderArgs {
    pub file: PathBuf,
    pub view: String,
    pub state: Option<String>,
    pub scenario: Option<PathBuf>,
    pub out: PathBuf,
    pub dark: bool,
    pub scale: f32,
}

pub fn run(args: &RenderArgs) -> anyhow::Result<ExitCode> {
    let model = match load_or_report(&args.file)? {
        Ok(model) => model,
        Err(code) => return Ok(code),
    };
    let mut view = match &args.state {
        Some(link) => ViewState::from_link(link).context("invalid --state link")?,
        None => ViewState::default(),
    };
    view.view = args.view.parse::<ViewKind>().context("invalid --view")?;

    let traces = match &args.scenario {
        Some(path) => {
            let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
            let scenario = cascade_sim::parse_scenario(&text)?;
            vec![cascade_sim::simulate(&model, &scenario)?]
        }
        None => Vec::new(),
    };

    let graph = CausalGraph::build(&model);
    let findings = analyze(&model, &graph);
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
        diff: None,
    };
    let scene = SceneBuilder::new().build(&input)?;

    match extension(&args.out).as_deref() {
        Some("svg") => {
            let svg = cascade_scene::to_svg(&scene)?;
            std::fs::write(&args.out, svg).with_context(|| format!("cannot write {}", args.out.display()))?;
        }
        Some("png") => {
            let png = cascade_scene::to_png(&scene, args.scale)?;
            std::fs::write(&args.out, png).with_context(|| format!("cannot write {}", args.out.display()))?;
        }
        _ => bail!("--out must end in .svg or .png"),
    }
    Ok(ExitCode::SUCCESS)
}

fn extension(path: &Path) -> Option<String> {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase())
}
