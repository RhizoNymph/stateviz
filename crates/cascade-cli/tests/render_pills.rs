//! `cascade render --view structure --state 'cascade://structure?pills=0'`:
//! the structure view (and, with `--edit`, the build canvas) with
//! transitions drawn as arrows instead of pills.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/order-fulfillment/cascade.yaml");
const SHOP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/shop/cascade.yaml");
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const ARROWS: &str = "cascade://structure?pills=0";

fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cascade-render-pills-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn render(file: &str, out: &Path, extra: &[&str]) -> Output {
    let out = out.to_str().expect("utf-8 path");
    let mut args = vec!["render", file, "--out", out];
    args.extend_from_slice(extra);
    Command::new(env!("CARGO_BIN_EXE_cascade")).args(args).output().expect("runs")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn pills_off_draws_arrows_labelled_with_their_triggers() {
    let dir = scratch("svg");
    for edit in [false, true] {
        let arrows = dir.join(format!("arrows-{edit}.svg"));
        let mut extra = vec!["--view", "structure", "--state", ARROWS];
        if edit {
            extra.push("--edit");
        }
        let output = render(EXAMPLE, &arrows, &extra);
        assert!(output.status.success(), "{}", stderr(&output));
        let svg = std::fs::read_to_string(&arrows).expect("written");
        assert!(!svg.contains("pending → paid"), "no pill labels");
        assert!(svg.contains(">capture_ok<"), "the arrow carries its trigger");

        let pills = dir.join(format!("pills-{edit}.svg"));
        let mut extra = vec!["--view", "structure"];
        if edit {
            extra.push("--edit");
        }
        let output = render(EXAMPLE, &pills, &extra);
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(std::fs::read_to_string(&pills).expect("written").contains("pending → paid"));
    }
}

#[test]
fn arrows_render_to_png() {
    let dir = scratch("png");
    let out = dir.join("shop.png");
    let output = render(SHOP, &out, &["--view", "structure", "--state", ARROWS, "--edit"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let bytes = std::fs::read(&out).expect("written");
    assert_eq!(bytes[..8], PNG_SIGNATURE);
}

#[test]
fn a_bad_pills_value_is_rejected() {
    let dir = scratch("bad");
    let output =
        render(EXAMPLE, &dir.join("x.svg"), &["--view", "structure", "--state", "cascade://structure?pills=off"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("pills"), "{}", stderr(&output));
}
