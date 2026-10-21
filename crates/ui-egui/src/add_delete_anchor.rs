//! Add Anchor Point and Delete Anchor Point tools (P group). Each click edits the path the canvas
//! shows — the targeted vector mask, the active shape layer's path, or the work path — through the
//! engine (`path.addAnchor`, `path.deleteAnchor`), so the change is one undoable history step and
//! the same commands drive it over MCP and the control channel.

use egui::Modifiers;
use photocraft_geom::Point;
use photocraft_vector::edit::Hit;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

/// Canvas pointer events for the Add / Delete Anchor Point tools; false for other tools.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: Modifiers) -> bool {
    let tool = app.ui.tool;
    if !matches!(tool, Tool::AddAnchorPoint | Tool::DeleteAnchorPoint) {
        return false;
    }
    let _ = mods;
    match ev {
        ToolEvent::Down { x, y, .. } => {
            click(app, tool, [x, y]);
            true
        }
        // These tools act on the press; the Move and Up of the same gesture are swallowed so no
        // marquee or paint drag starts.
        ToolEvent::Move { .. } | ToolEvent::Up { .. } => true,
    }
}

/// The path and hit under the pointer, front first (the vector mask or shape path, then the work
/// path), at the editing tolerance.
fn hit(app: &PhotocraftApp, p: [f64; 2]) -> Option<(crate::direct_select::PathRef, Hit)> {
    let tol = 6.0 / f64::from(app.current_zoom().max(0.01));
    crate::direct_select::candidates(app).into_iter().find_map(|(r, path)| photocraft_vector::edit::hit(&path, Point::new(p[0], p[1]), tol, &[]).map(|h| (r, h)))
}

fn click(app: &mut PhotocraftApp, tool: Tool, p: [f64; 2]) {
    let Some((target, hit)) = hit(app, p) else { return };
    let mut params = match (tool, hit) {
        (Tool::AddAnchorPoint, Hit::Segment([s, k], t)) => serde_json::json!({"subpath": s, "knot": k, "t": t}),
        (Tool::DeleteAnchorPoint, Hit::Anchor([s, k])) => serde_json::json!({"subpath": s, "knot": k}),
        // Adding on an anchor (rather than a segment), or deleting off an anchor, does nothing.
        _ => return,
    };
    if let (Some(p), Some(t)) = (params.as_object_mut(), target.params().as_object()) {
        p.extend(t.clone());
    }
    let id = if tool == Tool::AddAnchorPoint { "path.addAnchor" } else { "path.deleteAnchor" };
    if let Err(e) = app.run(id, params) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
#[path = "add_delete_anchor_tests.rs"]
mod tests;
