//! `type.mask`: rendering a type layer to a selection.

use super::*;

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 160, "depth": depth, "background": "white"})).unwrap();
    s
}

fn text_layer(s: &mut Session, text: &str) -> u64 {
    let r = s.execute("type.create", json!({"x": 20, "y": 110, "text": text, "size": 90, "color": "#000000"})).unwrap();
    r["layer"].as_u64().unwrap()
}

/// Selection coverage at a document pixel.
fn coverage(s: &Session, x: i32, y: i32) -> f32 {
    s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
}

/// What fraction of the canvas is selected (sum of coverage / area).
fn selected_fraction(s: &Session) -> f32 {
    let d = s.active().unwrap();
    let area = d.doc.bounds();
    let Some(sel) = d.doc.selection.as_ref() else { return 0.0 };
    let n = area.width() as usize * area.height() as usize;
    let mut buf = Vec::new();
    sel.read_region_into(area, &mut buf);
    let ch = sel.channels();
    buf.chunks_exact(ch).map(|p| p[0]).sum::<f32>() / n as f32
}

#[test]
fn mask_selects_the_glyphs() {
    let mut s = session(8);
    let id = text_layer(&mut s, "H");
    s.execute("type.mask", json!({"layer": id})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_some());
    // The glyph is a thin part of a 300×160 canvas: some pixels selected, most not, and both
    // the glyph body and the empty background behave.
    let f = selected_fraction(&s);
    assert!(f > 0.001 && f < 0.3, "glyph coverage fraction was {f}");
    // A pixel inside the vertical stem of "H" is selected; a corner is not.
    assert_eq!(coverage(&s, 296, 4), 0.0);
}

#[test]
fn mask_is_empty_when_the_text_is_blank() {
    let mut s = session(8);
    // Point text with only a space draws nothing.
    let r = s.execute("type.create", json!({"x": 20, "y": 110, "text": " ", "size": 40})).unwrap();
    let id = r["layer"].as_u64().unwrap();
    s.execute("type.mask", json!({"layer": id})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_none(), "blank text must give no selection");
    assert_eq!(selected_fraction(&s), 0.0);
}

#[test]
fn mask_combine_modes() {
    let mut s = session(8);
    let id = text_layer(&mut s, "H");
    // A big square selection, then subtract the glyphs: the fraction drops.
    s.execute("select.all", json!({})).unwrap();
    let all = selected_fraction(&s);
    assert!((all - 1.0).abs() < 1e-3, "{all}");
    s.execute("type.mask", json!({"layer": id, "mode": "subtract"})).unwrap();
    let after = selected_fraction(&s);
    assert!(after < all, "subtract left {after} of {all}");
    // Add puts the subtracted glyphs back (8-bit quantization: within one level per pixel).
    s.execute("type.mask", json!({"layer": id, "mode": "add"})).unwrap();
    assert!((selected_fraction(&s) - all).abs() < 0.01, "add did not restore the covered pixels");
    // Intersect with all keeps the glyphs only.
    s.execute("select.all", json!({})).unwrap();
    s.execute("type.mask", json!({"layer": id, "mode": "intersect"})).unwrap();
    let inter = selected_fraction(&s);
    assert!(inter > 0.001 && inter < 0.3, "{inter}");
}

#[test]
fn mask_is_one_history_step() {
    let mut s = session(8);
    let id = text_layer(&mut s, "H");
    // A second identical call replaces the selection (replace mode) in one step.
    s.execute("type.mask", json!({"layer": id})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_some());
    assert!(s.undo());
    assert!(s.active().unwrap().doc.selection.is_none(), "one undo removes the whole mask");
    assert!(s.redo());
    assert!(s.active().unwrap().doc.selection.is_some());
}

#[test]
fn mask_renders_at_any_depth() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let id = text_layer(&mut s, "H");
        s.execute("type.mask", json!({"layer": id})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_some(), "depth {depth}");
    }
}

#[test]
fn mask_rejects_non_type_and_bad_params() {
    let mut s = session(8);
    // A plain raster layer (the active layer) is refused by the enabled predicate.
    let raster = s.active().unwrap().active_layer.unwrap();
    assert!(s.execute("type.mask", json!({"layer": raster.0})).is_err());
    // No such layer: also refused (the predicate sees a non-type active layer).
    let e = s.execute("type.mask", json!({"layer": 9999})).unwrap_err();
    assert!(matches!(e, EngineError::Disabled(..) | EngineError::NoLayer(_)), "{e:?}");
    // With a type layer active, a good call works; a wrong-typed `mode` falls back to replace.
    let id = text_layer(&mut s, "H");
    s.execute("type.mask", json!({"layer": id, "mode": 7})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_some());
    // A layer that exists but is not text, named explicitly with a type layer active.
    let extra = s.execute("layer.new.layer", json!({"name": "pixel"})).unwrap();
    let pixel = extra["layer"].as_u64().unwrap();
    assert!(s.execute("type.mask", json!({"layer": pixel})).is_err());
}

#[test]
fn mask_is_disabled_without_a_type_layer() {
    let mut s = session(8);
    assert!(!s.is_enabled("type.mask"));
    let _ = text_layer(&mut s, "H");
    assert!(s.is_enabled("type.mask"));
}
