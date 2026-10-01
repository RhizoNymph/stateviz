//! Shareable view links through the clipboard.
//!
//! Copy writes [`ViewState::to_link`] with the current viewport baked in, so
//! the receiver sees exactly this view. Paste accepts a link anywhere in
//! the clipboard text (e.g. a pasted `cascade-app file --view '…'` command
//! line) and replaces the whole view state.

use cascade_core::{ElementKey, Model};
use cascade_scene::{ViewLinkError, ViewState, Viewport};

const SCHEME: &str = "cascade://";

/// The link for `state`, with `viewport` (the one on screen) recorded.
pub fn link_for(state: &ViewState, viewport: Option<Viewport>) -> String {
    let mut state = state.clone();
    if viewport.is_some() {
        state.viewport = viewport;
    }
    state.to_link()
}

/// Find and parse the first `cascade://` link in `text`.
pub fn parse_pasted(text: &str) -> Result<ViewState, ViewLinkError> {
    let start = text.find(SCHEME).ok_or(ViewLinkError::NotALink)?;
    let link: String =
        text[start..].chars().take_while(|c| !c.is_whitespace() && !matches!(c, '\'' | '"' | '<' | '>')).collect();
    ViewState::from_link(&link)
}

/// Keys in the link's selection or collapse set that this model does not
/// have (a link from another revision). They are kept, just reported.
pub fn unknown_keys<'a>(state: &'a ViewState, model: &Model) -> Vec<&'a ElementKey> {
    state.selection.iter().chain(&state.collapsed).filter(|k| model.resolve_key(k).is_none()).collect()
}

#[cfg(test)]
mod tests {
    use cascade_layout::Point;
    use cascade_scene::ViewKind;

    use super::*;

    #[test]
    fn copy_records_the_viewport() {
        let state = ViewState { view: ViewKind::Matrix, ..ViewState::default() };
        let vp = Viewport { center: Point::new(1.0, 2.0), zoom: 0.5 };
        let link = link_for(&state, Some(vp));
        assert_eq!(link, "cascade://matrix?at=1,2,0.5");
        assert_eq!(parse_pasted(&link).map(|s| s.viewport), Ok(Some(vp)));
        assert_eq!(link_for(&state, None), "cascade://matrix");
    }

    #[test]
    fn paste_finds_the_link_in_surrounding_text() {
        let text = "cascade-app shop.yaml --view 'cascade://structure?hide=Payment'\n";
        let state = parse_pasted(text).expect("parses");
        assert_eq!(state.view, ViewKind::Structure);
        assert!(state.hidden_machines.contains("Payment"));
        assert_eq!(parse_pasted("  cascade://trace  ").map(|s| s.view), Ok(ViewKind::Trace));
    }

    #[test]
    fn copy_and_paste_carry_lanes() {
        let state = ViewState { group_by_machine: true, ..ViewState::default() };
        let link = link_for(&state, None);
        assert_eq!(link, "cascade://causal?lanes=1");
        let pasted = parse_pasted(&format!("cascade-app shop.yaml --view '{link}'")).expect("parses");
        assert!(pasted.group_by_machine);
        assert!(!parse_pasted("cascade://causal").expect("parses").group_by_machine);
    }

    #[test]
    fn paste_rejects_non_links() {
        assert_eq!(parse_pasted("hello"), Err(ViewLinkError::NotALink));
        assert!(matches!(parse_pasted("cascade://pie"), Err(ViewLinkError::UnknownView(_))));
    }

    #[test]
    fn reports_keys_the_model_lacks() {
        let model =
            cascade_core::load_str(include_str!("../../../examples/order-fulfillment/cascade.yaml")).expect("loads");
        let known: ElementKey = "event:OrderPaid".parse().expect("key");
        let unknown: ElementKey = "event:Refunded".parse().expect("key");
        let state = ViewState { selection: vec![known, unknown.clone()], ..ViewState::default() };
        assert_eq!(unknown_keys(&state, &model), vec![&unknown]);
    }
}
