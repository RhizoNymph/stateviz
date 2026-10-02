//! Everything that determines what a view shows, and its link encoding.
//!
//! The spec encodes every filter in the URL so a view can be shared. The
//! native app has no URL bar, so the same state round-trips through a
//! `cascade://` link string: the app copies it to the clipboard and accepts
//! it back via `cascade-app <file> --view <link>` or a paste.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use cascade_core::{Direction, ElementKey};
use cascade_layout::Point;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ViewKind {
    #[default]
    Causal,
    Structure,
    Trace,
    Matrix,
}

impl ViewKind {
    pub const ALL: [ViewKind; 4] = [ViewKind::Causal, ViewKind::Structure, ViewKind::Trace, ViewKind::Matrix];

    pub const fn name(self) -> &'static str {
        match self {
            ViewKind::Causal => "causal",
            ViewKind::Structure => "structure",
            ViewKind::Trace => "trace",
            ViewKind::Matrix => "matrix",
        }
    }
}

impl fmt::Display for ViewKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for ViewKind {
    type Err = ViewLinkError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ViewKind::ALL.into_iter().find(|v| v.name() == s).ok_or_else(|| ViewLinkError::UnknownView(s.to_owned()))
    }
}

/// Cone tracing from the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConeFocus {
    pub direction: Direction,
    /// Maximum causal hops; `None` for unlimited.
    pub depth: Option<u32>,
}

/// What happens to elements outside a cone or path query.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OutsideFocus {
    /// Fade to the theme's dim opacity (default: the layout never jumps).
    #[default]
    Dim,
    /// Remove, leaving stubs for cut links.
    Hide,
}

/// Compare the definition at two git refs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DiffRefs {
    pub base: String,
    /// `None` compares against the working tree.
    pub head: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// Scene point at the center of the canvas.
    pub center: Point,
    pub zoom: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewState {
    pub view: ViewKind,
    /// Up to two selected elements. Two transitions make a path query.
    pub selection: Vec<ElementKey>,
    pub cone: Option<ConeFocus>,
    pub outside: OutsideFocus,
    /// Machines toggled off in the legend; each collapses to a stub node.
    pub hidden_machines: BTreeSet<String>,
    /// Collapsed composite states and machines (structure view).
    pub collapsed: BTreeSet<ElementKey>,
    /// Causal view: one lane per machine on shared causal columns (link
    /// parameter `lanes=1`).
    pub group_by_machine: bool,
    /// Structure view: draw each transition as a pill between its states
    /// (on, the default) or as one labelled state → state arrow (off; link
    /// parameter `pills=0`). The causal view always draws pills, since
    /// transitions are its nodes.
    pub transition_pills: bool,
    pub search: Option<String>,
    /// Restrict the causal view to two machines (from a matrix cell click).
    pub machine_pair: Option<(String, String)>,
    /// Trace view: scenario name.
    pub scenario: Option<String>,
    /// Trace view: show both orderings of this race (index into findings
    /// that are race candidates, in finding order).
    pub race: Option<u32>,
    pub diff: Option<DiffRefs>,
    pub viewport: Option<Viewport>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            view: ViewKind::default(),
            selection: Vec::new(),
            cone: None,
            outside: OutsideFocus::default(),
            hidden_machines: BTreeSet::new(),
            collapsed: BTreeSet::new(),
            group_by_machine: false,
            transition_pills: true,
            search: None,
            machine_pair: None,
            scenario: None,
            race: None,
            diff: None,
            viewport: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ViewLinkError {
    #[error("not a cascade link: expected `cascade://<view>?…`")]
    NotALink,
    #[error("unknown view `{0}`")]
    UnknownView(String),
    #[error("unknown link parameter `{0}`")]
    UnknownParam(String),
    #[error("invalid value for `{param}`: `{value}`")]
    InvalidValue { param: &'static str, value: String },
}

const SCHEME: &str = "cascade://";

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b':' | b'>' | b'@' | b'/' | b'~' => {
                out.push(char::from(b));
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn decode(s: &str, param: &'static str) -> Result<String, ViewLinkError> {
    let invalid = || ViewLinkError::InvalidValue { param, value: s.to_owned() };
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3).ok_or_else(invalid)?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| invalid())?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| invalid())
}

fn join_encoded<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    items.into_iter().map(encode).collect::<Vec<_>>().join(",")
}

fn split_decoded(value: &str, param: &'static str) -> Result<Vec<String>, ViewLinkError> {
    value.split(',').filter(|s| !s.is_empty()).map(|s| decode(s, param)).collect()
}

fn keys(value: &str, param: &'static str) -> Result<Vec<ElementKey>, ViewLinkError> {
    split_decoded(value, param)?
        .into_iter()
        .map(|k| k.parse().map_err(|_| ViewLinkError::InvalidValue { param, value: k.clone() }))
        .collect()
}

impl ViewState {
    /// Encode as `cascade://<view>?<params>`. Default-valued fields are
    /// omitted, so the default state is just `cascade://causal`.
    pub fn to_link(&self) -> String {
        let mut params: Vec<String> = Vec::new();
        if !self.selection.is_empty() {
            let sel: Vec<String> = self.selection.iter().map(ToString::to_string).collect();
            params.push(format!("sel={}", join_encoded(sel.iter().map(String::as_str))));
        }
        if let Some(cone) = self.cone {
            let dir = match cone.direction {
                Direction::Forward => 'f',
                Direction::Backward => 'b',
            };
            let depth = cone.depth.map_or("*".to_owned(), |d| d.to_string());
            params.push(format!("cone={dir}{depth}"));
        }
        if self.outside == OutsideFocus::Hide {
            params.push("outside=hide".to_owned());
        }
        if !self.hidden_machines.is_empty() {
            params.push(format!("hide={}", join_encoded(self.hidden_machines.iter().map(String::as_str))));
        }
        if !self.collapsed.is_empty() {
            let keys: Vec<String> = self.collapsed.iter().map(ToString::to_string).collect();
            params.push(format!("collapse={}", join_encoded(keys.iter().map(String::as_str))));
        }
        if self.group_by_machine {
            params.push("lanes=1".to_owned());
        }
        if !self.transition_pills {
            params.push("pills=0".to_owned());
        }
        if let Some(q) = &self.search {
            params.push(format!("q={}", encode(q)));
        }
        if let Some((a, b)) = &self.machine_pair {
            params.push(format!("pair={},{}", encode(a), encode(b)));
        }
        if let Some(s) = &self.scenario {
            params.push(format!("scenario={}", encode(s)));
        }
        if let Some(r) = self.race {
            params.push(format!("race={r}"));
        }
        if let Some(diff) = &self.diff {
            let head = diff.head.as_deref().unwrap_or("");
            params.push(format!("diff={},{}", encode(&diff.base), encode(head)));
        }
        if let Some(vp) = self.viewport {
            params.push(format!("at={},{},{}", vp.center.x, vp.center.y, vp.zoom));
        }
        let mut link = format!("{SCHEME}{}", self.view);
        if !params.is_empty() {
            link.push('?');
            link.push_str(&params.join("&"));
        }
        link
    }

    pub fn from_link(link: &str) -> Result<Self, ViewLinkError> {
        let rest = link.trim().strip_prefix(SCHEME).ok_or(ViewLinkError::NotALink)?;
        let (view, query) = rest.split_once('?').unwrap_or((rest, ""));
        let mut state = ViewState { view: view.parse()?, ..ViewState::default() };
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (param, value) = pair.split_once('=').unwrap_or((pair, ""));
            match param {
                "sel" => state.selection = keys(value, "sel")?,
                "cone" => {
                    let invalid = || ViewLinkError::InvalidValue { param: "cone", value: value.to_owned() };
                    let mut chars = value.chars();
                    let direction = match chars.next() {
                        Some('f') => Direction::Forward,
                        Some('b') => Direction::Backward,
                        _ => return Err(invalid()),
                    };
                    let depth = match chars.as_str() {
                        "*" => None,
                        n => Some(n.parse().map_err(|_| invalid())?),
                    };
                    state.cone = Some(ConeFocus { direction, depth });
                }
                "outside" => {
                    state.outside = match value {
                        "hide" => OutsideFocus::Hide,
                        "dim" => OutsideFocus::Dim,
                        _ => {
                            return Err(ViewLinkError::InvalidValue { param: "outside", value: value.to_owned() });
                        }
                    }
                }
                "hide" => state.hidden_machines = split_decoded(value, "hide")?.into_iter().collect(),
                "collapse" => state.collapsed = keys(value, "collapse")?.into_iter().collect(),
                "lanes" => {
                    state.group_by_machine = match value {
                        "1" => true,
                        "0" => false,
                        _ => return Err(ViewLinkError::InvalidValue { param: "lanes", value: value.to_owned() }),
                    }
                }
                "pills" => {
                    state.transition_pills = match value {
                        "1" => true,
                        "0" => false,
                        _ => return Err(ViewLinkError::InvalidValue { param: "pills", value: value.to_owned() }),
                    }
                }
                "q" => state.search = Some(decode(value, "q")?),
                "pair" => {
                    let parts = split_decoded(value, "pair")?;
                    match parts.as_slice() {
                        [a, b] => state.machine_pair = Some((a.clone(), b.clone())),
                        _ => {
                            return Err(ViewLinkError::InvalidValue { param: "pair", value: value.to_owned() });
                        }
                    }
                }
                "scenario" => state.scenario = Some(decode(value, "scenario")?),
                "race" => {
                    state.race = Some(
                        value
                            .parse()
                            .map_err(|_| ViewLinkError::InvalidValue { param: "race", value: value.to_owned() })?,
                    );
                }
                "diff" => {
                    let (base, head) = value.split_once(',').unwrap_or((value, ""));
                    let base = decode(base, "diff")?;
                    if base.is_empty() {
                        return Err(ViewLinkError::InvalidValue { param: "diff", value: value.to_owned() });
                    }
                    let head = decode(head, "diff")?;
                    state.diff = Some(DiffRefs { base, head: (!head.is_empty()).then_some(head) });
                }
                "at" => {
                    let invalid = || ViewLinkError::InvalidValue { param: "at", value: value.to_owned() };
                    let nums: Vec<f32> =
                        value.split(',').map(|n| n.parse::<f32>().map_err(|_| invalid())).collect::<Result<_, _>>()?;
                    match nums.as_slice() {
                        [x, y, zoom] if zoom.is_finite() && *zoom > 0.0 => {
                            state.viewport = Some(Viewport { center: Point::new(*x, *y), zoom: *zoom });
                        }
                        _ => return Err(invalid()),
                    }
                }
                other => return Err(ViewLinkError::UnknownParam(other.to_owned())),
            }
        }
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_a_bare_link() {
        assert_eq!(ViewState::default().to_link(), "cascade://causal");
        assert_eq!(ViewState::from_link("cascade://causal"), Ok(ViewState::default()));
    }

    #[test]
    fn full_state_round_trips() {
        let state = ViewState {
            view: ViewKind::Structure,
            selection: vec![
                "transition:Order:pending->paid@capture_ok#1".parse().expect("key"),
                "event:OrderPaid".parse().expect("key"),
            ],
            cone: Some(ConeFocus { direction: Direction::Backward, depth: Some(3) }),
            outside: OutsideFocus::Hide,
            hidden_machines: ["Payment".to_owned(), "Shipment".to_owned()].into_iter().collect(),
            collapsed: ["state:Job:running".parse().expect("key")].into_iter().collect(),
            group_by_machine: true,
            transition_pills: true,
            search: Some("paid & co, 100%".to_owned()),
            machine_pair: Some(("Order".into(), "Shipment".into())),
            scenario: Some("happy path".into()),
            race: Some(2),
            diff: Some(DiffRefs { base: "main".into(), head: Some("feature/x".into()) }),
            viewport: Some(Viewport { center: Point::new(12.5, -4.0), zoom: 1.25 }),
        };
        let link = state.to_link();
        assert!(link.starts_with("cascade://structure?"));
        assert_eq!(ViewState::from_link(&link), Ok(state));
    }

    #[test]
    fn unlimited_cone_and_working_tree_diff() {
        let state = ViewState {
            cone: Some(ConeFocus { direction: Direction::Forward, depth: None }),
            diff: Some(DiffRefs { base: "HEAD~1".into(), head: None }),
            ..ViewState::default()
        };
        let link = state.to_link();
        assert_eq!(link, "cascade://causal?cone=f*&diff=HEAD~1,");
        assert_eq!(ViewState::from_link(&link), Ok(state));
    }

    #[test]
    fn rejects_bad_links() {
        assert_eq!(ViewState::from_link("http://x"), Err(ViewLinkError::NotALink));
        assert!(matches!(ViewState::from_link("cascade://pie"), Err(ViewLinkError::UnknownView(_))));
        assert!(matches!(ViewState::from_link("cascade://causal?zoom=3"), Err(ViewLinkError::UnknownParam(_))));
        for bad in [
            "cascade://causal?cone=x1",
            "cascade://causal?cone=fx",
            "cascade://causal?sel=nonsense",
            "cascade://causal?pair=A",
            "cascade://causal?at=1,2",
            "cascade://causal?at=1,2,0",
            "cascade://causal?q=%zz",
            "cascade://causal?diff=,x",
        ] {
            assert!(ViewState::from_link(bad).is_err(), "{bad}");
        }
    }
}
