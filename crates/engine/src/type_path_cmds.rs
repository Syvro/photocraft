//! Type on a Path: attach a type layer to a work/saved path and control where the run starts,
//! how far it is slid, which subpath it follows and which side of the curve it sits on.
//!
//! The binding stores a resolved copy of the path in text space, so the text engine needs no
//! document lookup to lay out glyphs. A source (work/saved path) and the inverse text transform
//! are kept too: path edits re-read the source and update the copy, so the text re-flows live.
//! Photoshop's `Txt2` path data is deliberately out of scope (see `RESUME.md`); native `.pcraft`
//! saves keep the binding.

use photocraft_doc::path_measure::PathMeasure;
use photocraft_doc::text::{TextPathBinding, TextPathSource};
use photocraft_doc::{Document, LayerContent, Path, TextLayer};
use photocraft_geom::Point;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param};
use crate::type_cmds::refresh;
use crate::vector_cmds::{bad, is_work, resolve_path};
use crate::{EngineError, Result, Session};

const MAX_DISTANCE: f64 = 1_000_000.0;

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_text(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    match d.active_layer.and_then(|id| d.doc.layer(id)).map(|l| &l.content) {
        Some(LayerContent::Text(_)) => Ok(()),
        _ => Err("the active layer is not a type layer".into()),
    }
}

fn number(p: &Value, key: &str, default: Option<f64>) -> Result<Option<f64>> {
    match p.get(key) {
        Some(v) => {
            let n = v.as_f64().filter(|n| n.is_finite() && n.abs() <= MAX_DISTANCE).ok_or_else(|| bad("type.path", format!("`{key}` must be a finite number of magnitude at most {MAX_DISTANCE}")))?;
            Ok(Some(n))
        }
        None => Ok(default),
    }
}

fn source_from(name: &str) -> TextPathSource {
    if is_work(Some(name)) {
        TextPathSource::Work
    } else {
        TextPathSource::Saved(name.to_owned())
    }
}

fn source_name(source: &TextPathSource) -> String {
    match source {
        TextPathSource::Work => "work".to_owned(),
        TextPathSource::Saved(n) => n.clone(),
    }
}

/// The document path a binding is sourced from, if it still exists.
fn source_path(doc: &Document, source: &TextPathSource) -> Option<Path> {
    match source {
        TextPathSource::Work => doc.work_path.clone(),
        TextPathSource::Saved(n) => doc.paths.iter().find(|q| &q.name == n).map(|q| q.path.clone()),
    }
}

/// Re-reads every live source path and refreshes the type layers bound to it. Called by path edits
/// in the same history step, so editing a path makes its text re-flow with one undo.
pub(crate) fn reflow_bound_text(doc: &mut Document) {
    let mut updates = Vec::new();
    for (_, _, l) in doc.walk() {
        let LayerContent::Text(t) = &l.content else { continue };
        let Some(b) = t.path.as_ref() else { continue };
        let (Some(source), Some(xform)) = (b.source.as_ref(), b.source_xform) else { continue };
        let Some(new_path) = source_path(doc, source).map(|p| p.transform(&xform)) else { continue };
        if new_path != b.path {
            updates.push((l.id, new_path));
        }
    }
    if updates.is_empty() {
        return;
    }
    let snapshot = doc.clone();
    for (id, path) in updates {
        let Some(l) = doc.layer_mut(id) else { continue };
        let LayerContent::Text(t) = &mut l.content else { continue };
        if let Some(b) = t.path.as_mut() {
            b.path = path;
        }
        refresh(&snapshot, t);
    }
}

fn binding(t: &TextLayer) -> Result<&TextPathBinding> {
    t.path.as_ref().ok_or_else(|| EngineError::Other("the type layer is not attached to a path".into()))
}

fn attach(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "type.path.attach";
    let name = p.get("path").and_then(Value::as_str).unwrap_or("work").to_owned();
    if name.is_empty() {
        return Err(bad(CMD, "`path` cannot be empty"));
    }
    let source_path = resolve_path(s, Some(&name))?;
    let x = number(p, "x", None)?;
    let y = number(p, "y", None)?;
    if x.is_some() != y.is_some() {
        return Err(bad(CMD, "`x` and `y` must be given together"));
    }
    let flip = p.get("flip").and_then(Value::as_bool).unwrap_or(false);
    let subpath = match p.get("subpath") {
        Some(v) => Some(v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad(CMD, "`subpath` must be a non-negative integer"))?),
        None => None,
    };
    let (r, damage) = {
        let id = layer_param(s, p)?;
        s.edit("Attach Text to Path", |doc, _| {
            let snapshot = doc.clone();
            let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
            let LayerContent::Text(t) = &mut l.content else {
                return Err(EngineError::Other(format!("layer {} is not a type layer", id.0)));
            };
            let before = t.cache.as_ref().map(|c| c.tile_bounds());
            let inverse = t.transform.inverse().ok_or_else(|| bad(CMD, "the text layer transform is not invertible"))?;
            let local = source_path.transform(&inverse);
            let measure = PathMeasure::new(&local);
            if measure.is_empty() {
                return Err(bad(CMD, "the path has no measurable geometry"));
            }
            let click = x.zip(y).map(|(x, y)| inverse.apply(Point::new(x, y)));
            let found = match subpath {
                Some(i) => measure.nearest(i, click.unwrap_or_else(|| Point::new(0.0, 0.0))).map(|v| (i, v)),
                None => (0..measure.subpath_count()).filter_map(|i| measure.nearest(i, click.unwrap_or_else(|| Point::new(0.0, 0.0))).map(|v| (i, v))).min_by(|a, b| a.1.1.total_cmp(&b.1.1)),
            };
            let (chosen, (start, distance)) = found.ok_or_else(|| bad(CMD, "the path has no usable subpath"))?;
            t.path = Some(TextPathBinding {
                path: local,
                start: start.clamp(0.0, MAX_DISTANCE) as f32,
                offset: 0.0,
                flip,
                subpath: chosen,
                source: Some(source_from(&name)),
                source_xform: Some(inverse),
            });
            refresh(&snapshot, t);
            let damage = before.zip(t.cache.as_ref().map(|c| c.tile_bounds())).map(|(a, b)| a.union(&b));
            Ok((json!({ "path": name, "subpath": chosen, "start": start, "distance": distance, "flip": flip }), damage))
        })?
    };
    if let (Some(st), Some(rect)) = (s.active_mut(), damage) {
        st.last_damage = Some(rect);
    }
    Ok(r)
}

/// One-field edits share this shape: check the binding, change it, re-render.
fn change(s: &mut Session, p: &Value, label: &str, f: impl FnOnce(&mut TextLayer) -> Result<Value>) -> Result<Value> {
    let id = layer_param(s, p)?;
    let (r, damage) = s.edit(label, |doc, _| {
        let snapshot = doc.clone();
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let LayerContent::Text(t) = &mut l.content else {
            return Err(EngineError::Other(format!("layer {} is not a type layer", id.0)));
        };
        binding(t)?;
        let before = t.cache.as_ref().map(|c| c.tile_bounds());
        let r = f(t)?;
        refresh(&snapshot, t);
        let damage = before.zip(t.cache.as_ref().map(|c| c.tile_bounds())).map(|(a, b)| a.union(&b));
        Ok((r, damage))
    })?;
    if let (Some(st), Some(rect)) = (s.active_mut(), damage) {
        st.last_damage = Some(rect);
    }
    Ok(r)
}

fn detach(s: &mut Session, p: &Value) -> Result<Value> {
    change(s, p, "Detach Text from Path", |t| {
        t.path = None;
        Ok(json!({ "attached": false }))
    })
}

fn align(s: &mut Session, p: &Value) -> Result<Value> {
    let start = number(p, "start", None)?.ok_or_else(|| bad("type.path.align", "missing `start` (distance along the path)"))?;
    if start < 0.0 {
        return Err(bad("type.path.align", "`start` cannot be negative"));
    }
    let subpath = p.get("subpath").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok());
    change(s, p, "Align Text on Path", |t| {
        let b = t.path.as_mut().ok_or_else(|| EngineError::Other("no path binding".into()))?;
        b.start = start as f32;
        if let Some(i) = subpath {
            b.subpath = i;
        }
        Ok(json!({ "start": start, "subpath": b.subpath }))
    })
}

fn set_offset(s: &mut Session, p: &Value) -> Result<Value> {
    let offset = number(p, "offset", None)?.ok_or_else(|| bad("type.path.offset", "missing `offset`"))?;
    change(s, p, "Offset Text on Path", |t| {
        let b = t.path.as_mut().ok_or_else(|| EngineError::Other("no path binding".into()))?;
        b.offset = offset as f32;
        Ok(json!({ "offset": offset }))
    })
}

fn flip(s: &mut Session, p: &Value) -> Result<Value> {
    let on = p.get("on").and_then(Value::as_bool);
    change(s, p, "Flip Text on Path", |t| {
        let b = t.path.as_mut().ok_or_else(|| EngineError::Other("no path binding".into()))?;
        b.flip = on.unwrap_or(!b.flip);
        Ok(json!({ "flip": b.flip }))
    })
}

fn info(s: &Session, p: &Value) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = layer_param(s, p)?;
    let l = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let LayerContent::Text(t) = &l.content else {
        return Err(EngineError::Other(format!("layer {} is not a type layer", id.0)));
    };
    let Some(b) = t.path.as_ref() else {
        return Ok(json!({ "attached": false }));
    };
    let map = photocraft_text::path::PathMap::new(b);
    Ok(json!({
        "attached": true,
        "path": b.source.as_ref().map(source_name),
        "start": b.start,
        "offset": b.offset,
        "flip": b.flip,
        "subpath": b.subpath,
        "length": map.as_ref().map(|m| m.length()),
    }))
}

fn reflow(s: &mut Session, _p: &Value) -> Result<Value> {
    s.edit("Re-flow Text on Path", |doc, _| {
        reflow_bound_text(doc);
        Ok(())
    })?;
    Ok(json!({ "reflowed": true }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "type.path.attach",
            label: "Attach to Path",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?,"path":"work"|saved-name="work","x":number?,"y":number?,"subpath":int?,"flip":bool=false}"##,
            enabled: has_text,
            run: attach,
            journal: true,
        },
        CommandSpec {
            id: "type.path.detach",
            label: "Detach from Path",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?}"##,
            enabled: has_text,
            run: detach,
            journal: true,
        },
        CommandSpec {
            id: "type.path.align",
            label: "Align Text on Path",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?,"start":number>=0,"subpath":int?}"##,
            enabled: has_text,
            run: align,
            journal: true,
        },
        CommandSpec {
            id: "type.path.offset",
            label: "Offset Text on Path",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?,"offset":number}"##,
            enabled: has_text,
            run: set_offset,
            journal: true,
        },
        CommandSpec {
            id: "type.path.flip",
            label: "Flip Text on Path",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?,"on":bool? (default: toggle)}"##,
            enabled: has_text,
            run: flip,
            journal: true,
        },
        CommandSpec {
            id: "type.path.info",
            label: "Type on Path Info",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?}"##,
            enabled: has_text,
            run: |s, p| info(s, p),
            journal: false,
        },
        CommandSpec {
            id: "type.path.reflow",
            label: "Re-flow Text on Path",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: has_doc,
            run: reflow,
            journal: true,
        },
    ]
}

#[cfg(test)]
#[path = "type_path_cmds/tests.rs"]
mod tests;
