//! `cascade render --view causal --state 'cascade://causal?lanes=1'`: the
//! causal view grouped by machine.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/order-fulfillment/cascade.yaml");
const SHOP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/shop/cascade.yaml");
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const LANES: &str = "cascade://causal?lanes=1";

fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cascade-render-lanes-{}-{test}", std::process::id()));
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
fn lanes_link_renders_one_lane_per_machine() {
    let dir = scratch("svg");
    let lanes = dir.join("lanes.svg");
    let output = render(EXAMPLE, &lanes, &["--view", "causal", "--state", LANES]);
    assert!(output.status.success(), "{}", stderr(&output));
    let svg = std::fs::read_to_string(&lanes).expect("written");
    assert_eq!(svg.matches("class=\"lane\"").count(), 2, "Order and Shipment lanes");
    assert!(svg.contains(">Order<") && svg.contains(">Shipment<"));
    assert!(svg.contains("pending → paid") && !svg.contains("Order: pending → paid"), "the lane names the machine");

    let flat = dir.join("flat.svg");
    let output = render(EXAMPLE, &flat, &["--view", "causal"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let svg = std::fs::read_to_string(&flat).expect("written");
    assert!(!svg.contains("class=\"lane\""));
    assert!(svg.contains("Order: pending → paid"));
}

#[test]
fn lanes_render_to_png() {
    let dir = scratch("png");
    let out = dir.join("shop.png");
    let output = render(SHOP, &out, &["--view", "causal", "--state", LANES]);
    assert!(output.status.success(), "{}", stderr(&output));
    let bytes = std::fs::read(&out).expect("written");
    assert_eq!(bytes[..8], PNG_SIGNATURE);
}

#[test]
fn a_bad_lanes_value_is_rejected() {
    let dir = scratch("bad");
    let output = render(EXAMPLE, &dir.join("x.svg"), &["--view", "causal", "--state", "cascade://causal?lanes=yes"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("lanes"), "{}", stderr(&output));
}
