//! GPUI actions for the workspace's commands.
//!
//! One unit action per [`Command`]; [`bind_keys`] installs
//! [`commands::KEYMAP`], and [`register`] routes each action to
//! [`Workspace::run_command`].

use gpui::{App, Context, InteractiveElement, KeyBinding, actions};

use super::Workspace;
use crate::commands::{self, Command};
use crate::mode::AppMode;
use cascade_scene::ViewKind;

macro_rules! command_actions {
    ($($action:ident => $command:expr),* $(,)?) => {
        actions!(cascade, [$($action),*]);

        /// The key binding for `keys` → `command` in `context`.
        fn key_binding(keys: &str, command: Command, context: &str) -> Option<KeyBinding> {
            $(
                if command == $command {
                    return Some(KeyBinding::new(keys, $action, Some(context)));
                }
            )*
            None
        }

        /// Route every command action to the workspace.
        pub fn register<E: InteractiveElement>(el: E, cx: &mut Context<Workspace>) -> E {
            el $(
                .on_action(cx.listener(|ws: &mut Workspace, _: &$action, window, cx| {
                    ws.run_command($command, window, cx)
                }))
            )*
        }
    };
}

command_actions! {
    ClearSelection => Command::ClearSelection,
    ConeForward => Command::ConeForward,
    ConeBackward => Command::ConeBackward,
    ToggleOutside => Command::ToggleOutside,
    DepthLess => Command::DepthLess,
    DepthMore => Command::DepthMore,
    ShowCausal => Command::ShowView(ViewKind::Causal),
    ShowStructure => Command::ShowView(ViewKind::Structure),
    ShowTrace => Command::ShowView(ViewKind::Trace),
    ShowMatrix => Command::ShowView(ViewKind::Matrix),
    ZoomIn => Command::ZoomIn,
    ZoomOut => Command::ZoomOut,
    FitView => Command::FitView,
    PanLeft => Command::PanLeft,
    PanRight => Command::PanRight,
    PanUp => Command::PanUp,
    PanDown => Command::PanDown,
    FocusSearch => Command::FocusSearch,
    CopyLink => Command::CopyLink,
    PasteLink => Command::PasteLink,
    ToggleTheme => Command::ToggleTheme,
    OpenSource => Command::OpenSource,
    UnpinSelection => Command::UnpinSelection,
    Reload => Command::Reload,
    Quit => Command::Quit,
    ModeView => Command::SetMode(AppMode::View),
    ModeBuild => Command::SetMode(AppMode::Build),
    ModePlay => Command::SetMode(AppMode::Play),
    Undo => Command::Undo,
    Redo => Command::Redo,
    DeleteSelection => Command::DeleteSelection,
    NewFile => Command::NewFile,
    PlayStep => Command::PlayStep,
    PlayRun => Command::PlayRun,
}

/// Install the workspace key map.
pub fn bind_keys(cx: &mut App) {
    let bindings: Vec<KeyBinding> = commands::KEYMAP
        .iter()
        .filter_map(|b| {
            let binding = key_binding(b.keys, b.command, b.scope.context());
            if binding.is_none() {
                tracing::error!(keys = b.keys, command = ?b.command, "command has no action");
            }
            binding
        })
        .collect();
    cx.bind_keys(bindings);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_an_action() {
        for command in commands::ALL_COMMANDS {
            assert!(key_binding("x", *command, "Workspace").is_some(), "{command:?}");
        }
    }

    #[test]
    fn every_binding_parses() {
        for b in commands::KEYMAP {
            assert!(gpui::Keystroke::parse(b.keys).is_ok(), "bad keystroke {}", b.keys);
            assert!(gpui::KeyBindingContextPredicate::parse(b.scope.context()).is_ok(), "bad context");
        }
    }
}
