//! Type-mask commands: the Horizontal/Vertical Type Mask tools' commit.
//!
//! A type-mask tool types text exactly like the Type tool, but on commit the glyphs become a
//! **selection** (a coverage mask) instead of a text layer. `type.mask` renders a type layer to a
//! grayscale coverage surface, reads its alpha plane over the canvas, and combines it with the
//! current selection — the same [`photocraft_algo::selection::combine`] path every other selection
//! command uses, so the shape is anti-aliased exactly like Photoshop's.
//!
//! The UI keys off the toolbar tool: a type-mask tool creates a transient type layer (all the
//! inline editing, IME and transform machinery reused unchanged), and its commit runs `type.mask`
//! then deletes the layer inside one coalesced history step, so a whole typing session is one undo.

use photocraft_algo::selection::{self as sel, SelectionMode};
use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::LayerContent;
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// Pixel format of a coverage mask: grayscale at the document depth, no transparency. The text
/// rasterizer always appends an alpha channel, so the rendered surface is effectively GrayAlpha —
/// the alpha plane is the glyph coverage regardless of the text's colour.
fn mask_format(doc: &photocraft_doc::Document) -> PixelFormat {
    PixelFormat::new(ColorMode::Grayscale, doc.depth, false)
}

/// Reads channel `c` of a surface over `area` as one `f32` per pixel (missing samples are the
/// surface's default pixel, normally 0).
fn read_channel(s: &photocraft_raster::Surface, area: Rect, c: usize) -> Vec<f32> {
    let w = area.width() as usize;
    let h = area.height() as usize;
    let mut out = Vec::with_capacity(w * h);
    if w == 0 || h == 0 {
        return out;
    }
    // `sample_channel` looks up one tile per sample; a full-canvas mask is small enough that the
    // region read is simpler and stays correct for sparse (pruned) surfaces.
    let mut buf = Vec::new();
    s.read_region_into(area, &mut buf);
    let ch = s.channels();
    for px in buf.chunks_exact(ch) {
        out.push(px.get(c).copied().unwrap_or(0.0));
    }
    out
}

/// `type.mask`: render the type layer's glyphs to a selection.
///
/// Params: `{"layer":id?, "mode":"replace|add|subtract|intersect"="replace"}`.
pub fn create_selection(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "type.mask";
    let id = crate::commands::layer_param(s, p)?;
    let mode = SelectionMode::parse(p.get("mode").and_then(Value::as_str).unwrap_or("replace"));
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let l = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let LayerContent::Text(t) = &l.content else {
        return Err(EngineError::Other(format!("layer {} is not a type layer", id.0)));
    };
    let area = d.doc.bounds();
    if area.is_empty() {
        return Err(bad(cmd, "the document has no canvas"));
    }
    // Render (pure: never mutates the layer) and take the alpha plane as coverage.
    let mut eng = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
    let format = mask_format(&d.doc);
    let (_layout, rendered) = eng.render(t, d.doc.resolution_dpi, format);
    drop(eng);
    let alpha = rendered.surface.format().channels().saturating_sub(1);
    let mask = read_channel(&rendered.surface, area, alpha);
    let selected = s.edit("Type Mask", |doc, _| {
        doc.selection = sel::combine(doc.selection.as_ref(), &mask, area, mode);
        Ok(doc.selection.is_some())
    })?;
    Ok(json!({ "selected": selected }))
}

/// The Type menu gets a Photoshop-free entry point (`Type › Create Selection from Type`); the
/// toolbar tools drive the same command. Disabled unless the active layer is a type layer.
fn has_text(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    match d.active_layer.and_then(|id| d.doc.layer(id)).map(|l| &l.content) {
        Some(LayerContent::Text(_)) => Ok(()),
        _ => Err("the active layer is not a type layer".into()),
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "type.mask",
        label: "Create Selection from Type",
        menu: &["Type"],
        shortcut: None,
        params: r##"{"layer":id? (default: active type layer),"mode":"replace|add|subtract|intersect"="replace"} → renders the layer's glyphs to an anti-aliased selection"##,
        enabled: has_text,
        journal: true,
        run: create_selection,
    }]
}

#[cfg(test)]
#[path = "type_mask_cmds/tests.rs"]
mod tests;
