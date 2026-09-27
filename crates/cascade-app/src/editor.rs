//! Click-to-source: open a definition line in the user's editor.
//!
//! Resolution order:
//! 1. `CASCADE_EDITOR`, a command template with `{file}`, `{line}` and
//!    `{col}` placeholders (`{file}` is appended when absent). Words split
//!    on whitespace; single or double quotes group words.
//! 2. `zed file:line` when `zed` is on `PATH`.
//! 3. `code -g file:line` when `code` is on `PATH`.
//! 4. The platform opener (`xdg-open file`, `open file` on macOS).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// The environment variable holding the editor template.
pub const EDITOR_ENV: &str = "CASCADE_EDITOR";

/// A place in a file. `line` and `col` are 1-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location<'a> {
    pub file: &'a Path,
    pub line: u32,
    pub col: u32,
}

impl<'a> Location<'a> {
    /// A location from a source span; unknown positions open line 1.
    pub fn from_span(file: &'a Path, span: cascade_core::SourceSpan) -> Self {
        let line = span.line().unwrap_or(1).max(1);
        let col = if span.is_known() { span.start.col.max(1) } else { 1 };
        Self { file, line, col }
    }
}

/// A program and its arguments, ready to spawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl EditorCommand {
    /// The command as one line, for status messages.
    pub fn display(&self) -> String {
        std::iter::once(self.program.as_str()).chain(self.args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EditorError {
    #[error("{EDITOR_ENV} has an unterminated quote: `{0}`")]
    UnterminatedQuote(String),
    #[error("{EDITOR_ENV} names no program")]
    NoProgram,
}

/// Build the command that opens `loc`. `template` is the value of
/// [`EDITOR_ENV`] (blank counts as unset); `on_path` says whether a program
/// name is installed.
pub fn build_command(
    template: Option<&str>,
    on_path: impl Fn(&str) -> bool,
    loc: Location<'_>,
) -> Result<EditorCommand, EditorError> {
    let file = loc.file.to_string_lossy().into_owned();
    if let Some(template) = template.filter(|t| !t.trim().is_empty()) {
        return from_template(template, loc);
    }
    if on_path("zed") {
        return Ok(EditorCommand { program: "zed".into(), args: vec![format!("{file}:{}", loc.line)] });
    }
    if on_path("code") {
        return Ok(EditorCommand { program: "code".into(), args: vec!["-g".into(), format!("{file}:{}", loc.line)] });
    }
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    Ok(EditorCommand { program: opener.into(), args: vec![file] })
}

fn from_template(template: &str, loc: Location<'_>) -> Result<EditorCommand, EditorError> {
    let words = split_words(template)?;
    let mentions_file = words.iter().any(|w| w.contains("{file}"));
    let file = loc.file.to_string_lossy();
    let mut words: Vec<String> = words
        .into_iter()
        .map(|w| {
            w.replace("{file}", &file).replace("{line}", &loc.line.to_string()).replace("{col}", &loc.col.to_string())
        })
        .collect();
    if !mentions_file {
        words.push(file.into_owned());
    }
    let mut words = words.into_iter();
    let program = words.next().filter(|p| !p.is_empty()).ok_or(EditorError::NoProgram)?;
    Ok(EditorCommand { program, args: words.collect() })
}

/// Split on whitespace; `'…'` and `"…"` group, and a backslash escapes the
/// next character outside single quotes.
pub fn split_words(text: &str) -> Result<Vec<String>, EditorError> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('"') | None, '\\') => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
                in_word = true;
            }
            (Some(_), c) => current.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            (None, c) => {
                current.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err(EditorError::UnterminatedQuote(text.to_owned()));
    }
    if in_word {
        words.push(current);
    }
    Ok(words)
}

/// The full path of `program` if it is an executable file on `path_var`
/// (a `PATH`-style list).
pub fn find_on_path(program: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    let path_var = path_var?;
    std::env::split_paths(path_var).map(|dir| dir.join(program)).find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Resolve the command for `loc` from the real environment.
pub fn command_for(loc: Location<'_>) -> Result<EditorCommand, EditorError> {
    let template = std::env::var(EDITOR_ENV).ok();
    let path_var = std::env::var_os("PATH");
    build_command(template.as_deref(), |program| find_on_path(program, path_var.as_deref()).is_some(), loc)
}

/// Start the editor detached from the app's stdio. The caller reaps the
/// child.
pub fn spawn(command: &EditorCommand) -> std::io::Result<Child> {
    Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(file: &Path) -> Location<'_> {
        Location { file, line: 12, col: 5 }
    }

    fn cmd(program: &str, args: &[&str]) -> EditorCommand {
        EditorCommand { program: program.into(), args: args.iter().map(|s| (*s).to_owned()).collect() }
    }

    #[test]
    fn template_substitutes_placeholders() {
        let file = Path::new("/w/cascade.yaml");
        let c = build_command(Some("nvim +{line} {file}"), |_| true, loc(file)).expect("builds");
        assert_eq!(c, cmd("nvim", &["+12", "/w/cascade.yaml"]));
        let c = build_command(Some("code -g {file}:{line}:{col}"), |_| false, loc(file)).expect("builds");
        assert_eq!(c, cmd("code", &["-g", "/w/cascade.yaml:12:5"]));
    }

    #[test]
    fn template_without_file_appends_it() {
        let c = build_command(Some("subl"), |_| false, loc(Path::new("a.yaml"))).expect("builds");
        assert_eq!(c, cmd("subl", &["a.yaml"]));
    }

    #[test]
    fn paths_with_spaces_stay_one_argument() {
        let file = Path::new("/my docs/cascade.yaml");
        let c = build_command(Some("'my editor' --line={line} {file}"), |_| false, loc(file)).expect("builds");
        assert_eq!(c, cmd("my editor", &["--line=12", "/my docs/cascade.yaml"]));
    }

    #[test]
    fn blank_template_falls_back_to_detection() {
        let file = Path::new("x.yaml");
        assert_eq!(build_command(Some("   "), |p| p == "zed", loc(file)), Ok(cmd("zed", &["x.yaml:12"])));
        assert_eq!(build_command(None, |p| p == "zed", loc(file)), Ok(cmd("zed", &["x.yaml:12"])));
        assert_eq!(build_command(None, |p| p == "code", loc(file)), Ok(cmd("code", &["-g", "x.yaml:12"])));
    }

    #[test]
    fn zed_is_preferred_over_code() {
        let c = build_command(None, |_| true, loc(Path::new("x.yaml"))).expect("builds");
        assert_eq!(c.program, "zed");
    }

    #[test]
    fn nothing_installed_uses_the_platform_opener() {
        let c = build_command(None, |_| false, loc(Path::new("x.yaml"))).expect("builds");
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        assert_eq!(c, cmd(opener, &["x.yaml"]));
    }

    #[test]
    fn bad_templates_are_errors() {
        let file = Path::new("x.yaml");
        assert!(matches!(
            build_command(Some("vim 'oops"), |_| false, loc(file)),
            Err(EditorError::UnterminatedQuote(_))
        ));
        assert_eq!(build_command(Some("'' {file}"), |_| false, loc(file)), Err(EditorError::NoProgram));
    }

    #[test]
    fn word_splitting() {
        assert_eq!(split_words("a  b\tc").expect("splits"), ["a", "b", "c"]);
        assert_eq!(split_words(r#"a "b c" 'd "e"'"#).expect("splits"), ["a", "b c", r#"d "e""#]);
        assert_eq!(split_words(r"a\ b c").expect("splits"), ["a b", "c"]);
        assert_eq!(split_words(r#""""#).expect("splits"), [""]);
        assert!(split_words("").expect("splits").is_empty());
    }

    #[test]
    fn location_from_unknown_span_is_line_one() {
        let file = Path::new("x.yaml");
        let l = Location::from_span(file, cascade_core::SourceSpan::unknown());
        assert_eq!((l.line, l.col), (1, 1));
        let span = cascade_core::SourceSpan::new(cascade_core::Pos::new(7, 3), cascade_core::Pos::new(7, 9));
        let l = Location::from_span(file, span);
        assert_eq!((l.line, l.col), (7, 3));
    }

    #[test]
    fn path_lookup_finds_executables_only() {
        let dir = std::env::temp_dir().join(format!("cascade-editor-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let exe = dir.join("fake-editor");
        let plain = dir.join("not-executable");
        std::fs::write(&exe, "#!/bin/sh\n").expect("write");
        std::fs::write(&plain, "").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).expect("chmod");
            std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        }
        let path_var = std::env::join_paths([Path::new("/nonexistent"), dir.as_path()]).expect("joins");
        assert_eq!(find_on_path("fake-editor", Some(&path_var)), Some(exe.clone()));
        #[cfg(unix)]
        assert_eq!(find_on_path("not-executable", Some(&path_var)), None);
        assert_eq!(find_on_path("missing", Some(&path_var)), None);
        assert_eq!(find_on_path("fake-editor", None), None);
        let _ = std::fs::remove_file(&exe);
        let _ = std::fs::remove_file(&plain);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn display_joins_program_and_args() {
        assert_eq!(cmd("zed", &["x:1"]).display(), "zed x:1");
    }
}
