//! `cascade-app`: the native GPUI app.
//!
//! `cascade-app [--new] <file> [--view <cascade:// link>]` opens a
//! definition (creating a starter one with `--new`), paints its scenes,
//! reloads whenever the file, its pins sidecar or its scenarios change on
//! disk, and lets you build the system (Build mode) and play it (Play
//! mode).

mod args;
mod build;
mod canvas;
mod commands;
mod diffmode;
mod document;
mod editor;
mod gesture;
mod input;
mod link;
mod locate;
mod mode;
mod panels;
mod play;
mod settings;
mod theme;
mod trace;
mod viewport;
mod watch;
mod workspace;

use std::process::ExitCode;

use clap::Parser;
use gpui::{App, AppContext as _, Bounds, SharedString, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use gpui_platform::application;

use crate::args::Args;
use crate::workspace::Workspace;

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "cascade_app=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
}

fn main() -> ExitCode {
    init_tracing();
    let args = Args::parse();
    let mut view = match args.initial_view() {
        Ok(view) => view,
        Err(error) => {
            eprintln!("cascade-app: --view: {error}");
            return ExitCode::from(2);
        }
    };
    if args.new
        && let Err(error) = build::disk::create_new(&args.file)
    {
        eprintln!("cascade-app: --new: {error}");
        return ExitCode::from(2);
    }
    // Remembered settings seed the view; a `--view` link carries its own.
    let settings_path = settings::default_path();
    let remembered = settings::load_or_default(settings_path.as_deref());
    if args.view.is_none() {
        view.transition_pills = remembered.transition_pills;
    }
    let settings_path = settings_path.ok();
    let mode = args.initial_mode();
    let path = match std::fs::canonicalize(&args.file) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("cascade-app: cannot open {}: {error}", args.file.display());
            return ExitCode::from(2);
        }
    };
    let title = workspace::window_title(&path);

    application().run(move |cx: &mut App| {
        input::bind_keys(cx);
        workspace::actions::bind_keys(cx);
        let mono = SharedString::from(canvas::font::pick_mono(&cx.text_system().all_font_names()));
        tracing::debug!(font = %mono, "canvas font");

        let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions { title: Some(title.into()), ..Default::default() }),
                app_id: Some("cascade".to_owned()),
                focus: true,
                ..Default::default()
            },
            move |window, cx| cx.new(|cx| Workspace::new(path, view, mode, mono, settings_path, window, cx)),
        );
        if let Err(error) = opened {
            tracing::error!(%error, "cannot open the window");
            cx.quit();
            return;
        }
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        cx.activate(true);
    });
    ExitCode::SUCCESS
}
