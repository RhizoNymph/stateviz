//! The workbench mode: viewing, building or playing.
//!
//! Pure: which views each mode shows, how the scene is drawn in it, and
//! labels for the segmented control. The workspace keeps one [`AppMode`]
//! and routes view switches through [`view_for`].

use cascade_scene::{SceneMode, ViewKind};

/// What the window is for right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AppMode {
    /// Reading and analysing: the four views as before.
    #[default]
    View,
    /// Editing the definition in the structure view.
    Build,
    /// Driving an interactive simulator session over the causal or
    /// structure view.
    Play,
}

impl AppMode {
    pub const ALL: [AppMode; 3] = [AppMode::View, AppMode::Build, AppMode::Play];

    pub const fn label(self) -> &'static str {
        match self {
            AppMode::View => "View",
            AppMode::Build => "Build",
            AppMode::Play => "Play",
        }
    }

    /// How the scene builder draws in this mode.
    pub const fn scene_mode(self) -> SceneMode {
        match self {
            AppMode::Build => SceneMode::Edit,
            AppMode::View | AppMode::Play => SceneMode::View,
        }
    }

    /// Whether `view` can be shown in this mode.
    pub const fn allows(self, view: ViewKind) -> bool {
        match self {
            AppMode::View => true,
            AppMode::Build => matches!(view, ViewKind::Structure),
            AppMode::Play => matches!(view, ViewKind::Structure | ViewKind::Causal),
        }
    }

    /// The view to show on entering this mode from `current`.
    pub const fn entry_view(self, current: ViewKind) -> ViewKind {
        if self.allows(current) { current } else { ViewKind::Structure }
    }
}

/// Asking for `view` while in `mode`: the mode to be in and whether the
/// view is shown. A view the mode cannot show drops back to View mode, so
/// pressing `3` in Build mode opens the trace view.
pub const fn view_for(mode: AppMode, view: ViewKind) -> AppMode {
    if mode.allows(view) { mode } else { AppMode::View }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_shows_only_the_structure_view_in_edit_mode() {
        assert_eq!(AppMode::Build.scene_mode(), SceneMode::Edit);
        assert!(AppMode::Build.allows(ViewKind::Structure));
        for view in [ViewKind::Causal, ViewKind::Trace, ViewKind::Matrix] {
            assert!(!AppMode::Build.allows(view), "{view:?}");
        }
        assert_eq!(AppMode::Build.entry_view(ViewKind::Causal), ViewKind::Structure);
    }

    #[test]
    fn play_shows_structure_and_causal() {
        assert_eq!(AppMode::Play.scene_mode(), SceneMode::View);
        assert!(AppMode::Play.allows(ViewKind::Causal));
        assert!(AppMode::Play.allows(ViewKind::Structure));
        assert!(!AppMode::Play.allows(ViewKind::Trace));
        assert_eq!(AppMode::Play.entry_view(ViewKind::Causal), ViewKind::Causal);
        assert_eq!(AppMode::Play.entry_view(ViewKind::Matrix), ViewKind::Structure);
    }

    #[test]
    fn view_mode_shows_everything() {
        for view in ViewKind::ALL {
            assert!(AppMode::View.allows(view));
            assert_eq!(AppMode::View.entry_view(view), view);
        }
        assert_eq!(AppMode::View.scene_mode(), SceneMode::View);
    }

    #[test]
    fn asking_for_an_unsupported_view_leaves_the_mode() {
        assert_eq!(view_for(AppMode::Build, ViewKind::Trace), AppMode::View);
        assert_eq!(view_for(AppMode::Build, ViewKind::Structure), AppMode::Build);
        assert_eq!(view_for(AppMode::Play, ViewKind::Causal), AppMode::Play);
        assert_eq!(view_for(AppMode::Play, ViewKind::Matrix), AppMode::View);
        assert_eq!(view_for(AppMode::View, ViewKind::Matrix), AppMode::View);
    }

    #[test]
    fn labels() {
        let labels: Vec<_> = AppMode::ALL.iter().map(|m| m.label()).collect();
        assert_eq!(labels, ["View", "Build", "Play"]);
    }
}
