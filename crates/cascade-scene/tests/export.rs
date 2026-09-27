//! SVG and PNG export of scenes.

mod common;

use cascade_core::Severity;
use cascade_layout::{Point, Rect};
use cascade_scene::{
    Arrow, Badge, Border, Dash, EdgeKind, Emphasis, ExportError, FontWeight, HitTarget, Label, Lane, Layer, Overlay,
    Rgba, Scene, SceneEdge, SceneNode, Shape, Stroke, ViewKind, ViewState, to_png, to_svg,
};
use common::*;

fn parse(svg: &str) -> roxmltree::Document<'_> {
    match roxmltree::Document::parse(svg) {
        Ok(doc) => doc,
        Err(err) => panic!("invalid SVG: {err}\n{svg}"),
    }
}

fn with_class<'a, 'i>(doc: &'a roxmltree::Document<'i>, class: &str) -> Vec<roxmltree::Node<'a, 'i>> {
    doc.descendants().filter(|n| n.attribute("class") == Some(class)).collect()
}

fn label(text: &str, x: f32, y: f32) -> Label {
    Label {
        text: text.into(),
        origin: Point::new(x, y),
        font_size: 13.0,
        color: Rgba::hex(0x1F2328),
        weight: FontWeight::Normal,
    }
}

fn node(shape: Shape, rect: Rect, border: Border, dash: Dash) -> SceneNode {
    SceneNode {
        target: HitTarget::None,
        shape,
        rect,
        fill: Some(Rgba::hex(0xDDEEFF)),
        stroke: Stroke { color: Rgba::hex(0x0072B2), width: 1.5, dash },
        border,
        labels: vec![label("x", rect.left() + 2.0, rect.top() + 2.0)],
        badge: None,
        opacity: 1.0,
        emphasis: Emphasis::Normal,
        diff: None,
    }
}

/// A scene with every shape, border, dash, arrow, badge, overlay and lane.
fn kitchen_sink() -> Scene {
    let mut scene = Scene::empty(ViewKind::Causal, Rgba::hex(0xFFFFFF));
    scene.lanes.push(Lane {
        target: HitTarget::None,
        rect: Rect::new(0.0, 0.0, 900.0, 300.0),
        fill: Rgba::hex(0xF0F4FA),
        stroke: Stroke::solid(Rgba::hex(0x99AACC), 1.0),
        title: Label { weight: FontWeight::Bold, ..label("Order", 10.0, 4.0) },
        opacity: 1.0,
        collapsed: false,
    });
    let shapes = [
        (Shape::Pill, Border::Single, Dash::Solid),
        (Shape::Tag, Border::Single, Dash::Dashed { on: 6.0, off: 4.0 }),
        (Shape::Hexagon, Border::Single, Dash::Solid),
        (Shape::RoundedRect { radius: 8.0 }, Border::ThickLeft(5.0), Dash::Solid),
        (Shape::RoundedRect { radius: 8.0 }, Border::Double, Dash::Solid),
        (Shape::Rect, Border::Single, Dash::Dotted),
        (Shape::Stub, Border::Single, Dash::Dashed { on: 6.0, off: 4.0 }),
    ];
    for (i, (shape, border, dash)) in shapes.into_iter().enumerate() {
        scene.nodes.push(node(shape, Rect::new(20.0 + 120.0 * i as f32, 40.0, 100.0, 40.0), border, dash));
    }
    scene.nodes[0].labels.push(Label { weight: FontWeight::Bold, ..label("a < b & \"c\" → 'd'", 22.0, 60.0) });
    scene.nodes[1].badge =
        Some(Badge { count: 3, severity: Severity::Error, center: Point::new(240.0, 40.0), radius: 8.0 });
    scene.nodes[2].badge =
        Some(Badge { count: 1, severity: Severity::Warning, center: Point::new(360.0, 40.0), radius: 8.0 });
    scene.nodes[2].fill = None;
    scene.nodes[3].opacity = 0.15;
    scene.edges.push(SceneEdge {
        target: HitTarget::None,
        kind: EdgeKind::Fire,
        points: vec![
            Point::new(120.0, 60.0),
            Point::new(130.0, 60.0),
            Point::new(130.0, 150.0),
            Point::new(140.0, 150.0),
        ],
        stroke: Stroke::dashed(Rgba::hex(0x009E73), 1.5),
        arrow: Arrow::End,
        label: Some(label("[amount > 0]", 125.0, 100.0)),
        opacity: 0.5,
        emphasis: Emphasis::Dimmed,
        back_edge: false,
        diff: None,
    });
    scene.edges.push(SceneEdge {
        target: HitTarget::None,
        kind: EdgeKind::Transition,
        points: vec![Point::new(200.0, 200.0), Point::new(300.0, 200.0)],
        stroke: Stroke::solid(Rgba::hex(0xCF222E), 1.5),
        arrow: Arrow::None,
        label: None,
        opacity: 1.0,
        emphasis: Emphasis::Normal,
        back_edge: true,
        diff: None,
    });
    scene.overlays.push(Overlay::Line {
        from: Point::new(10.0, 250.0),
        to: Point::new(10.0, 290.0),
        stroke: Stroke { color: Rgba::hex(0xD0D7DE), width: 1.0, dash: Dash::Dashed { on: 4.0, off: 4.0 } },
        opacity: 1.0,
        layer: Layer::Under,
    });
    scene.overlays.push(Overlay::Rect {
        rect: Rect::new(400.0, 200.0, 10.0, 10.0),
        fill: Some(Rgba::hex(0xE69F00)),
        stroke: Some(Stroke::solid(Rgba::hex(0x000000), 1.0)),
        radius: 5.0,
        opacity: 0.5,
        layer: Layer::Over,
        target: HitTarget::None,
    });
    scene.overlays.push(Overlay::Text { label: label("title", 0.0, -20.0), opacity: 1.0, layer: Layer::Over });
    scene.bounds = Rect::new(0.0, -20.0, 900.0, 320.0);
    scene
}

#[test]
fn svg_is_well_formed_and_covers_every_item() {
    let scene = kitchen_sink();
    let svg = to_svg(&scene).expect("svg");
    let doc = parse(&svg);
    let root = doc.root_element();
    assert_eq!(root.tag_name().name(), "svg");
    assert_eq!(root.tag_name().namespace(), Some("http://www.w3.org/2000/svg"));
    assert!(root.attribute("viewBox").is_some());
    assert!(root.attribute("font-family").is_some_and(|f| f.contains("monospace")));
    assert_eq!(with_class(&doc, "node").len(), scene.nodes.len());
    assert_eq!(with_class(&doc, "edge").len(), scene.edges.len());
    assert_eq!(with_class(&doc, "lane").len(), scene.lanes.len());
    assert_eq!(with_class(&doc, "overlay").len(), scene.overlays.len());
    assert_eq!(with_class(&doc, "badge").len(), 2);

    let background = with_class(&doc, "background");
    assert_eq!(background.len(), 1);
    assert_eq!(background[0].attribute("fill"), Some("#ffffff"));

    // Text is escaped and round-trips.
    let texts: Vec<String> =
        doc.descendants().filter(|n| n.has_tag_name("text")).filter_map(|n| n.text().map(str::to_owned)).collect();
    assert!(texts.iter().any(|t| t == "a < b & \"c\" → 'd'"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "[amount > 0]"));
    assert!(texts.iter().any(|t| t == "Order"));
    assert!(texts.iter().any(|t| t == "3"), "badge count");
    assert!(doc.descendants().filter(|n| n.has_tag_name("text")).any(|n| n.attribute("font-weight") == Some("bold")));
}

#[test]
fn svg_encodes_shapes_borders_dashes_arrows_and_opacity() {
    let scene = kitchen_sink();
    let svg = to_svg(&scene).expect("svg");
    let doc = parse(&svg);
    let nodes = with_class(&doc, "node");
    let shape_of = |i: usize| nodes[i].children().find(|c| c.is_element()).expect("shape");

    // Pill: a rect with fully rounded ends.
    let pill = shape_of(0);
    assert_eq!(pill.tag_name().name(), "rect");
    assert_eq!(pill.attribute("rx"), Some("20"));
    // Tag and hexagon are paths.
    assert_eq!(shape_of(1).tag_name().name(), "path");
    assert_eq!(shape_of(2).tag_name().name(), "path");
    assert_eq!(shape_of(2).attribute("fill"), Some("none"));
    // Dashes.
    assert_eq!(shape_of(1).attribute("stroke-dasharray"), Some("6 4"));
    assert!(shape_of(5).attribute("stroke-dasharray").is_some(), "dotted");
    assert!(shape_of(0).attribute("stroke-dasharray").is_none(), "solid");
    // Initial state: a clipped bar on the left.
    let bar = nodes[3].children().find(|c| c.attribute("class") == Some("initial-bar")).expect("bar");
    let clip = bar.attribute("clip-path").expect("clip");
    let clip_id = clip.trim_start_matches("url(#").trim_end_matches(')');
    assert!(doc.descendants().any(|n| n.has_tag_name("clipPath") && n.attribute("id") == Some(clip_id)));
    // Final state: two outlines.
    let outlines = nodes[4].children().filter(|c| c.has_tag_name("rect")).count();
    assert_eq!(outlines, 2, "double border");
    // Opacity is carried by the item's group.
    assert_eq!(nodes[3].attribute("opacity"), Some("0.15"));
    assert!(nodes[0].attribute("opacity").is_none());

    // Arrowheads are markers in the stroke's color.
    let edges = with_class(&doc, "edge");
    let line = edges[0].children().find(|c| c.has_tag_name("polyline")).expect("polyline");
    let marker = line.attribute("marker-end").expect("marker-end");
    let marker_id = marker.trim_start_matches("url(#").trim_end_matches(')');
    let def = doc
        .descendants()
        .find(|n| n.has_tag_name("marker") && n.attribute("id") == Some(marker_id))
        .expect("marker def");
    assert!(def.descendants().any(|n| n.attribute("fill") == Some("#009e73")));
    assert_eq!(line.attribute("fill"), Some("none"));
    assert_eq!(edges[0].attribute("opacity"), Some("0.5"));
    let plain = edges[1].children().find(|c| c.has_tag_name("polyline")).expect("polyline");
    assert!(plain.attribute("marker-end").is_none(), "Arrow::None");
    assert_eq!(plain.attribute("stroke"), Some("#cf222e"));
}

#[test]
fn every_view_exports_valid_svg() {
    let mut fx = Fixture::new(SPEC_EXAMPLE);
    fx.findings = Vec::new();
    for view in cascade_scene::ViewKind::ALL {
        let scene = fx.scene(&ViewState { view, ..ViewState::default() });
        let svg = to_svg(&scene).expect("svg");
        let doc = parse(&svg);
        assert_eq!(with_class(&doc, "node").len(), scene.nodes.len(), "{view}");
    }
    let causal = fx.scene(&ViewState::default());
    let svg = to_svg(&causal).expect("svg");
    assert!(svg.contains("Order: pending → paid"));
}

#[test]
fn png_is_a_rasterised_svg_at_the_requested_scale() {
    let scene = kitchen_sink();
    let one = to_png(&scene, 1.0).expect("png");
    assert!(one.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]));
    let dims = |png: &[u8]| {
        let be = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        (be(&png[16..20]), be(&png[20..24]))
    };
    let (w1, h1) = dims(&one);
    let two = to_png(&scene, 2.0).expect("png");
    let (w2, h2) = dims(&two);
    assert!(w1 > 900 && h1 > 320, "the scene plus a margin: {w1}×{h1}");
    assert!((w2 as i64 - 2 * w1 as i64).abs() <= 2 && (h2 as i64 - 2 * h1 as i64).abs() <= 2);
}

#[test]
fn png_rejects_bad_scales_and_empty_scenes_still_export() {
    let scene = kitchen_sink();
    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(matches!(to_png(&scene, bad), Err(ExportError::InvalidScale(_))), "{bad}");
    }
    assert!(matches!(to_png(&scene, 10_000.0), Err(ExportError::TooLarge { .. })));
    let empty = Scene::empty(ViewKind::Trace, Rgba::hex(0x0D1117));
    let svg = to_svg(&empty).expect("svg");
    parse(&svg);
    let png = to_png(&empty, 1.0).expect("png");
    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
}

#[test]
fn non_finite_geometry_is_an_error_not_a_broken_file() {
    let mut scene = kitchen_sink();
    scene.nodes[0].rect.origin.x = f32::NAN;
    assert!(matches!(to_svg(&scene), Err(ExportError::NonFinite)));
}
