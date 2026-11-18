use super::*;
use photocraft_doc::LayerId;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 600, "height": 200, "background": "white"})).unwrap();
    s.execute("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[20, 60], [520, 60]]}]}})).unwrap();
    s
}

fn text_layer(s: &mut Session) -> LayerId {
    let r = s.execute("type.create", json!({"x": 20, "y": 60, "text": "Hello", "size": 30, "color": "#000000"})).unwrap();
    LayerId(r["layer"].as_u64().unwrap())
}

fn binding(s: &Session, id: LayerId) -> &TextPathBinding {
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.path.as_ref().unwrap(),
        _ => panic!("not a type layer"),
    }
}

fn ink(s: &Session, id: LayerId) -> photocraft_geom::Rect {
    s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds()
}

fn attach(s: &mut Session, id: LayerId) {
    s.execute("type.path.attach", json!({"layer": id.0, "path": "work", "x": 20, "y": 60})).unwrap();
}

#[test]
fn attach_align_offset_and_flip_move_the_ink_and_undo_restores_it() {
    let mut s = session();
    assert!(!s.is_enabled("type.path.attach"));
    let id = text_layer(&mut s);
    let flat = ink(&s, id);
    attach(&mut s, id);
    assert!(s.is_enabled("type.path.detach"));
    let b = binding(&s, id);
    assert_eq!(b.source, Some(TextPathSource::Work));
    assert!(b.start.abs() < 1e-3 && b.subpath == 0);
    let on_curve = ink(&s, id);
    assert!(on_curve.y1 <= 62 && on_curve.x0 >= 15, "text sits above the line: {on_curve:?}");

    s.execute("type.path.align", json!({"start": 100})).unwrap();
    let shifted = ink(&s, id);
    assert!((shifted.x0 - on_curve.x0 - 100).abs() <= 2, "align: {on_curve:?} → {shifted:?}");
    s.execute("type.path.offset", json!({"offset": 25})).unwrap();
    let offset = ink(&s, id);
    assert!((offset.x0 - shifted.x0 - 25).abs() <= 2, "offset: {shifted:?} → {offset:?}");
    s.execute("type.path.flip", json!({})).unwrap();
    assert!(binding(&s, id).flip);
    let flipped = ink(&s, id);
    assert!(flipped.y0 >= 55, "flip puts text below the line: {flipped:?}");

    for _ in 0..4 {
        assert!(s.undo());
    }
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => assert!(t.path.is_none(), "four undos remove the binding"),
        _ => panic!("not a type layer"),
    }
    assert_eq!(ink(&s, id), flat);
}

#[test]
fn path_edits_reflow_bound_text_in_the_same_history_step() {
    let mut s = session();
    let id = text_layer(&mut s);
    attach(&mut s, id);
    let before_path = binding(&s, id).path.clone();
    let before_ink = ink(&s, id);
    s.execute("path.moveAnchors", json!({"anchors": [[0, 1]], "move": [0, 80]})).unwrap();
    let after_path = binding(&s, id).path.clone();
    assert_ne!(after_path, before_path, "the bound copy follows the work path");
    let after_ink = ink(&s, id);
    assert_ne!(after_ink, before_ink, "the raster re-flows with the path");
    assert!(after_ink.y1 > before_ink.y1 + 5, "{before_ink:?} → {after_ink:?}");
    assert!(s.undo());
    assert_eq!(binding(&s, id).path, before_path);
    assert_eq!(ink(&s, id), before_ink);

    s.execute("path.transform", json!({"translateX": 30})).unwrap();
    let moved = binding(&s, id).path.clone();
    assert_ne!(moved, before_path);
    assert!((ink(&s, id).x0 - before_ink.x0 - 30).abs() <= 2);
}

#[test]
fn detach_restores_the_flat_baseline() {
    let mut s = session();
    let id = text_layer(&mut s);
    let flat = ink(&s, id);
    attach(&mut s, id);
    s.execute("type.path.detach", json!({})).unwrap();
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => assert!(t.path.is_none()),
        _ => panic!("not a type layer"),
    }
    assert_eq!(ink(&s, id), flat);
    assert_eq!(s.execute("type.path.info", json!({})).unwrap(), json!({"attached": false}));
}

#[test]
fn hostile_params_return_errors_instead_of_panicking() {
    let mut s = session();
    for p in [
        json!({}),
        json!({"path": "work", "x": "no", "y": 1}),
        json!({"path": "work", "x": 1e30, "y": 1}),
        json!({"path": "work", "subpath": -1}),
        json!({"path": "work", "subpath": "all"}),
        json!({"path": "no-such-path"}),
    ] {
        assert!(s.execute("type.path.attach", p.clone()).is_err(), "{p}");
    }
    assert!(s.execute("type.path.align", json!({})).is_err());
    assert!(s.execute("type.path.align", json!({"start": -5})).is_err());
    assert!(s.execute("type.path.offset", json!({"offset": [1, 2]})).is_err());
    assert!(s.execute("type.path.flip", json!({"layer": 999999})).is_err());
    assert!(s.execute("type.path.detach", json!({})).is_err());
    assert!(s.execute("type.path.info", json!({})).is_err());

    let id = text_layer(&mut s);
    assert!(s.execute("type.path.align", json!({"layer": id.0, "start": 10})).is_err());
    assert!(s.execute("type.path.detach", json!({"layer": id.0})).is_err());
    attach(&mut s, id);
    assert!(s.execute("type.path.align", json!({"layer": id.0, "start": -10})).is_err());
    assert!(s.execute("type.path.align", json!({"layer": id.0, "start": f64::NAN})).is_err());
    assert!(s.execute("type.path.attach", json!({"layer": id.0, "path": "work", "subpath": 999})).is_err());
    assert!(s.execute("type.path.info", json!({"layer": id.0})).unwrap()["attached"] == true);
}
