//! `cascade render`: every view to SVG and PNG, view links, themes, pins
//! and errors.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/order-fulfillment/cascade.yaml");
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// A fresh directory for one test's outputs.
fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cascade-render-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn cascade(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cascade")).args(args).output().expect("runs")
}

fn render(file: &str, out: &Path, extra: &[&str]) -> Output {
    let out = out.to_str().expect("utf-8 path");
    let mut args = vec!["render", file, "--out", out];
    args.extend_from_slice(extra);
    cascade(&args)
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn svg_width(svg: &str) -> f32 {
    let start = svg.find("width=\"").expect("width") + 7;
    let end = svg[start..].find('"').expect("quote") + start;
    svg[start..end].parse().expect("number")
}

#[test]
fn renders_every_view_as_svg() {
    let dir = scratch("views");
    for view in ["causal", "structure", "trace", "matrix"] {
        let out = dir.join(format!("{view}.svg"));
        let output = render(EXAMPLE, &out, &["--view", view]);
        assert!(output.status.success(), "{view}: {}", stderr(&output));
        let svg = std::fs::read_to_string(&out).expect("written");
        assert!(svg.starts_with("<?xml") && svg.contains("<svg"), "{view}");
    }
    let causal = std::fs::read_to_string(dir.join("causal.svg")).expect("causal");
    assert!(causal.contains("Order: pending → paid"));
    let structure = std::fs::read_to_string(dir.join("structure.svg")).expect("structure");
    assert!(structure.contains("OrderPaid › Fulfillment"));
    let matrix = std::fs::read_to_string(dir.join("matrix.svg")).expect("matrix");
    assert!(matrix.contains("Shipment"));
}

#[test]
fn trace_without_a_scenario_says_what_to_do() {
    let dir = scratch("trace");
    let output = render(EXAMPLE, &dir.join("t.svg"), &["--view", "trace"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("Pick a scenario to trace"));
}

#[test]
fn renders_png_at_a_scale() {
    let dir = scratch("png");
    let one = dir.join("one.png");
    let two = dir.join("two.png");
    assert!(render(EXAMPLE, &one, &["--scale", "1"]).status.success());
    assert!(render(EXAMPLE, &two, &["--scale", "2"]).status.success());
    let (a, b) = (std::fs::read(&one).expect("one"), std::fs::read(&two).expect("two"));
    assert!(a.starts_with(&PNG_SIGNATURE) && b.starts_with(&PNG_SIGNATURE));
    let width = |png: &[u8]| u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    assert!(width(&b) > width(&a) * 3 / 2);
    let bad = render(EXAMPLE, &dir.join("bad.png"), &["--scale", "0"]);
    assert_eq!(bad.status.code(), Some(2));
}

#[test]
fn applies_view_links_and_the_dark_theme() {
    let dir = scratch("links");
    let out = dir.join("hidden.svg");
    let output = render(EXAMPLE, &out, &["--state", "cascade://causal?hide=Shipment"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(std::fs::read_to_string(&out).expect("svg").contains("Shipment, 2 links"));

    let dark = dir.join("dark.svg");
    assert!(render(EXAMPLE, &dark, &["--dark", "--view", "matrix"]).status.success());
    assert!(std::fs::read_to_string(&dark).expect("svg").contains("fill=\"#0d1117\""));
}

#[test]
fn honours_pins_from_the_sidecar() {
    let dir = scratch("pins");
    let file = dir.join("cascade.yaml");
    std::fs::copy(EXAMPLE, &file).expect("copy");
    let plain = dir.join("plain.svg");
    assert!(render(file.to_str().expect("utf-8"), &plain, &[]).status.success());
    std::fs::write(
        dir.join("cascade.layout.json"),
        r#"{"version":1,"pins":{"causal":{"event:OrderPaid":{"x":5000.0,"y":40.0}}}}"#,
    )
    .expect("sidecar");
    let pinned = dir.join("pinned.svg");
    let output = render(file.to_str().expect("utf-8"), &pinned, &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    let (plain, pinned) =
        (std::fs::read_to_string(&plain).expect("plain"), std::fs::read_to_string(&pinned).expect("pinned"));
    assert!(svg_width(&pinned) > 5000.0 && svg_width(&plain) < 5000.0);
}

#[test]
fn reports_bad_arguments_with_exit_code_two() {
    let dir = scratch("errors");
    let cases: [(&[&str], &str, &str); 4] = [
        (&[], "out.txt", ".svg or .png"),
        (&["--view", "pie"], "out.svg", "--view"),
        (&["--state", "https://example.com"], "out.svg", "--state"),
        (&["--view", "trace", "--scenario", "/definitely/missing.yaml"], "out.svg", "cannot read"),
    ];
    for (extra, name, message) in cases {
        let output = render(EXAMPLE, &dir.join(name), extra);
        assert_eq!(output.status.code(), Some(2), "{extra:?}");
        assert!(stderr(&output).contains(message), "{extra:?}: {}", stderr(&output));
    }
}

#[test]
fn invalid_definitions_report_diagnostics() {
    let dir = scratch("invalid");
    let file = dir.join("broken.yaml");
    std::fs::write(&file, "machines:\n  A:\n    states: [x]\n    transitions: [{ from: x, to: nowhere, on: go }]\n")
        .expect("write");
    let out = dir.join("out.svg");
    let output = render(file.to_str().expect("utf-8"), &out, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("broken.yaml"));
    assert!(!out.exists());
}
