//! The test driver and the test declaration.
//!
//! `TestDriver` creates one instance of every machine and controller into a
//! registry keyed by P machine name, sends the registry to each (`eWire`),
//! then, for a bounded number of steps (10), lets every external source fire
//! each of its triggers nondeterministically.

use cascade_core::Model;

use super::names::{DRIVER, Names, REGISTRY_TYPE, TEST, WIRE_EVENT};
use super::writer::Writer;

/// How many rounds of external triggers the driver runs.
const STEPS: u32 = 10;

pub(super) fn write_driver(w: &mut Writer, model: &Model, names: &Names) {
    w.comment("Creates one instance of every machine and controller, wires them together,");
    w.comment("then lets the external sources fire their triggers nondeterministically.");
    w.open(&format!("machine {DRIVER} {{"));
    w.line(&format!("var registry: {REGISTRY_TYPE};"));
    w.blank();

    w.open("start state Init {");
    w.open("entry {");
    w.line("var m: machine;");
    for name in instances(model, names) {
        w.line(&format!("registry[\"{name}\"] = new {name}();"));
    }
    w.open("foreach (m in values(registry)) {");
    w.line(&format!("send m, {WIRE_EVENT}, registry;"));
    w.close();
    w.line("goto Driving;");
    w.close();
    w.close();
    w.blank();

    w.open("state Driving {");
    w.open("entry {");
    let drives = model.externals().any(|(_, x)| !x.triggers.is_empty());
    if drives {
        w.line("var i: int;");
        w.line("i = 0;");
        w.open(&format!("while (i < {STEPS}) {{"));
        for (_, source) in model.externals() {
            if source.triggers.is_empty() {
                continue;
            }
            w.comment(&source.name);
            for &trigger in &source.triggers {
                let machine = model.trigger(trigger).machine;
                w.open("if ($) {");
                w.line(&format!("send registry[\"{}\"], {};", names.machine(machine), names.trigger(trigger)));
                w.close();
            }
        }
        w.line("i = i + 1;");
        w.close();
    } else {
        w.comment("No external source fires a trigger: nothing drives the system.");
    }
    w.close();
    w.close();
    w.close();
}

pub(super) fn write_test(w: &mut Writer, model: &Model, names: &Names) {
    let mut members = vec![DRIVER.to_owned()];
    members.extend(instances(model, names));
    w.line(&format!("test {TEST} [main = {DRIVER}]: {{ {} }};", members.join(", ")));
}

/// Every machine and controller, in definition order.
fn instances(model: &Model, names: &Names) -> Vec<String> {
    let machines = model.machine_ids().map(|m| names.machine(m).to_owned());
    let controllers = model.controller_ids().map(|c| names.controller(c).to_owned());
    machines.chain(controllers).collect()
}
