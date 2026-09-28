//! `cascade render --edit` draws the build canvas: the structure view with
//! its wiring band.

use std::path::PathBuf;
use std::process::Command;

fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/order-fulfillment/cascade.yaml")
}

fn render_svg(extra: &[&str], name: &str) -> String {
    let out = std::env::temp_dir().join(format!("cascade-render-edit-{name}-{}.svg", std::process::id()));
    let output = Command::new(env!("CARGO_BIN_EXE_cascade"))
        .arg("render")
        .arg(example())
        .args(["--view", "structure"])
        .args(extra)
        .arg("--out")
        .arg(&out)
        .output()
        .expect("cascade runs");
    assert!(output.status.success(), "render failed: {}", String::from_utf8_lossy(&output.stderr));
    let svg = std::fs::read_to_string(&out).expect("svg written");
    let _ = std::fs::remove_file(&out);
    svg
}

#[test]
fn edit_flag_adds_the_wiring_band() {
    let svg = render_svg(&["--edit"], "edit");
    assert!(svg.contains("Events and controllers"));
    assert!(svg.contains("External sources"));
    assert!(svg.contains("Fulfillment"));
}

#[test]
fn without_the_flag_the_structure_view_is_unchanged() {
    let svg = render_svg(&[], "view");
    assert!(!svg.contains("Events and controllers"));
}
