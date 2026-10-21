//! Add Anchor Point and Delete Anchor Point (P group): a click adds a smooth knot on a segment, or
//! deletes the anchor under the pointer, through `path.addAnchor` / `path.deleteAnchor`.

use egui::Modifiers;
use photocraft_doc::vector::Path;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

/// A closed square work path, all corners.
fn with_path(app: &mut PhotocraftApp) {
    let path = json!({"subpaths": [{"closed": true, "knots": [[40, 40], [160, 40], [160, 160], [40, 160]]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
}

fn work(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().unwrap()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn click(app: &mut PhotocraftApp, at: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: at[0], y: at[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: at[0], y: at[1] }, Modifiers::NONE);
}

#[test]
fn clicking_a_segment_adds_an_anchor_in_one_undoable_step() {
    let mut app = app(Tool::AddAnchorPoint);
    with_path(&mut app);
    let (before, n) = (work(&app), steps(&app));
    // The top edge (knot 0 → 1, y = 40) at its midpoint.
    click(&mut app, [100.0, 40.0]);
    let wp = work(&app);
    assert_eq!(wp.subpaths[0].knots.len(), 5, "a knot was inserted");
    assert_eq!((wp.subpaths[0].knots[1].anchor.x, wp.subpaths[0].knots[1].anchor.y), (100.0, 40.0));
    assert_eq!(steps(&app), n + 1, "one history step");
    assert!(app.session.undo());
    assert_eq!(work(&app), before);
}

#[test]
fn clicking_an_anchor_with_add_does_nothing() {
    let mut app = app(Tool::AddAnchorPoint);
    with_path(&mut app);
    let n = steps(&app);
    click(&mut app, [40.0, 40.0]);
    assert_eq!(work(&app).subpaths[0].knots.len(), 4, "adding on an anchor is a no-op");
    assert_eq!(steps(&app), n);
}

#[test]
fn clicking_an_anchor_deletes_it_in_one_undoable_step() {
    let mut app = app(Tool::DeleteAnchorPoint);
    with_path(&mut app);
    let (before, n) = (work(&app), steps(&app));
    click(&mut app, [160.0, 40.0]);
    let wp = work(&app);
    assert_eq!(wp.subpaths[0].knots.len(), 3, "the anchor was removed");
    assert!(!wp.subpaths.iter().flat_map(|s| &s.knots).any(|k| k.anchor.x == 160.0 && k.anchor.y == 40.0));
    assert_eq!(steps(&app), n + 1, "one history step");
    assert!(app.session.undo());
    assert_eq!(work(&app), before);
}

#[test]
fn clicking_a_segment_with_delete_does_nothing() {
    let mut app = app(Tool::DeleteAnchorPoint);
    with_path(&mut app);
    let n = steps(&app);
    click(&mut app, [100.0, 40.0]);
    assert_eq!(work(&app).subpaths[0].knots.len(), 4, "deleting off an anchor is a no-op");
    assert_eq!(steps(&app), n);
}

#[test]
fn the_tools_do_not_start_a_marquee_or_paint_drag() {
    let mut app = app(Tool::AddAnchorPoint);
    with_path(&mut app);
    // A press on empty canvas with these tools is swallowed, not turned into a drag.
    for at in [[10.0, 10.0], [100.0, 40.0]] {
        tool_event(&mut app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, Modifiers::NONE);
        assert!(app.drag.is_none(), "no drag started at {at:?}");
        assert!(app.ui.direct_selection.drag.is_none(), "no Direct Selection drag either");
        tool_event(&mut app, ToolEvent::Up { x: at[0], y: at[1] }, Modifiers::NONE);
    }
}
