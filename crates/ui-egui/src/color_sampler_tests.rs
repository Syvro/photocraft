//! The Color Sampler tool through the real canvas (#1649): a click drops a numbered marker at the
//! exact pixel, dragging a marker moves it, ⌥-click removes it (Photoshop: Alt-click to delete),
//! markers persist across tool switches, and at most `MAX_SAMPLERS` can be placed.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::analysis_ui::MAX_SAMPLERS;
use crate::canvas::ViewXform;
use crate::state::Tool;

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = Tool::ColorSampler;
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::shortcuts::handle(app, &ctx);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    // 100 %: one document pixel per point, so clicks land on whole pixels.
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.fit_pending = false;
    h.run_steps(2);
    h
}

fn screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
    let app = h.state();
    let v = &app.ui.views[0];
    let xf = ViewXform {
        rect: crate::rulers::content_rect(app, app.last_canvas_rect),
        zoom: v.zoom,
        center: v.center,
        flip: app.ui.view.flip_horizontal,
        rotation: v.rotation,
    };
    xf.to_screen(x, y)
}

fn click(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32, m: Modifiers) {
    let p = screen(h, x, y);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: m });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: m });
    h.run_steps(2);
}

fn samplers(h: &Harness<'static, PhotocraftApp>) -> Vec<[i32; 2]> {
    h.state().ui.analysis.color_samplers.clone()
}

#[test]
fn a_click_drops_a_marker_at_the_exact_pixel_and_alt_click_removes_it() {
    let mut h = harness();
    click(&mut h, 250.0, 120.0, Modifiers::NONE);
    assert_eq!(samplers(&h), vec![[250, 120]], "one marker at the clicked pixel");
    // ⌥-click on the marker deletes it (Photoshop: Alt-click to remove).
    click(&mut h, 250.0, 120.0, Modifiers::ALT);
    assert!(samplers(&h).is_empty(), "⌥-click removes");
    // ⌥-click on empty canvas does not add.
    click(&mut h, 60.0, 60.0, Modifiers::ALT);
    assert!(samplers(&h).is_empty(), "⌥-click never adds");
}

#[test]
fn clicking_an_existing_marker_starts_a_drag_that_moves_it() {
    let mut h = harness();
    click(&mut h, 60.0, 60.0, Modifiers::NONE);
    assert_eq!(samplers(&h), vec![[60, 60]]);
    // Press on the marker, drag to (100, 90), release.
    let p = screen(&h, 60.0, 60.0);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    let p2 = screen(&h, 100.0, 90.0);
    h.event(egui::Event::PointerMoved(p2));
    h.run_steps(2);
    h.event(egui::Event::PointerButton { pos: p2, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(samplers(&h), vec![[100, 90]], "the marker moved with the drag");
    assert_eq!(h.state().ui.analysis.color_samplers.len(), 1, "no marker was added");
}

#[test]
fn at_most_max_samplers_markers_and_they_persist_across_tools() {
    let mut h = harness();
    for i in 0..MAX_SAMPLERS {
        click(&mut h, 20.0 + 30.0 * i as f32, 20.0, Modifiers::NONE);
    }
    assert_eq!(samplers(&h).len(), MAX_SAMPLERS);
    // An 11th click is ignored.
    click(&mut h, 350.0, 250.0, Modifiers::NONE);
    assert_eq!(samplers(&h).len(), MAX_SAMPLERS, "Photoshop caps the samplers at 10");
    // They stay after switching tools and come back (they draw on top of the canvas regardless).
    h.state_mut().ui.tool = Tool::Move;
    h.run_steps(2);
    assert_eq!(samplers(&h).len(), MAX_SAMPLERS);
    h.state_mut().ui.tool = Tool::ColorSampler;
    h.run_steps(2);
    assert_eq!(samplers(&h).len(), MAX_SAMPLERS);
    // A click on an existing marker still removes, not adds beyond the cap.
    click(&mut h, 20.0, 20.0, Modifiers::ALT);
    assert_eq!(samplers(&h).len(), MAX_SAMPLERS - 1);
}

/// `cargo test --release -p photocraft-ui-egui sampler_overlay_bench -- --ignored --nocapture`
#[test]
#[ignore]
fn sampler_overlay_bench() {
    let mut h = harness();
    for i in 0..MAX_SAMPLERS {
        let p = screen(&h, 40.0 + 30.0 * i as f32, 40.0);
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    }
    h.run_steps(3);
    let n = 60;
    let t0 = std::time::Instant::now();
    for _ in 0..n {
        h.step();
    }
    let ms = t0.elapsed().as_secs_f64() * 1e3 / f64::from(n);
    eprintln!("color sampler overlay with 10 markers: {ms:.2} ms per frame");
    assert_eq!(h.state().ui.analysis.color_samplers.len(), MAX_SAMPLERS);
}