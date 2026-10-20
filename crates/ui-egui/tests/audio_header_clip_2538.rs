// #2538: an audio track's M/S header buttons must not paint over the video half when the
// audio tracks are scrolled up.
//
// egui drains its graphic layers at the end of every frame, so painted shapes are captured
// at the end of the wrapped app's `update`, while they are still live.
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

type Shapes = Rc<RefCell<Vec<(egui::Rect, egui::Rect)>>>;

struct Wrapper {
    inner: FilmcraftApp,
    armed: Rc<RefCell<bool>>,
    captured: Shapes,
}

impl eframe::App for Wrapper {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.inner.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.inner.ui(ui, frame);
        if !*self.armed.borrow() {
            return;
        }
        let ctx = ui.ctx().clone();
        // Shapes live on whichever layer the panel painted to; sweep every visible one.
        let mut layers = vec![egui::LayerId::background()];
        ctx.memory(|m| layers.extend(m.areas().visible_layer_ids()));
        let mut out = self.captured.borrow_mut();
        out.clear();
        ctx.graphics(|g| {
            for li in layers {
                let Some(list) = g.get(li) else { continue };
                for cs in list.all_entries() {
                    if let egui::Shape::Rect(rs) = &cs.shape {
                        out.push((rs.rect, cs.clip_rect));
                    }
                }
            }
        });
        *self.armed.borrow_mut() = false;
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.inner.raw_input_hook(ctx, raw);
    }
}

struct Driver {
    harness: Harness<'static, Wrapper>,
    tx: Sender<ControlRequest>,
    armed: Rc<RefCell<bool>>,
    captured: Shapes,
}

impl Driver {
    fn demo() -> Self {
        let mut session = Session::default();
        session.execute("file.openDemoProject", json!({})).expect("demo project");
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let armed = Rc::new(RefCell::new(false));
        let captured = Rc::new(RefCell::new(Vec::new()));
        let w = Wrapper { inner: app, armed: armed.clone(), captured: captured.clone() };
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| w);
        let mut d = Driver { harness, tx, armed, captured };
        d.frames(4);
        d
    }
    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }
    fn ok(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                assert_eq!(v["ok"], json!(true), "{method} {params} failed: {v}");
                return v["result"].clone();
            }
        }
        panic!("no reply to {method} {params}");
    }
    fn app(&mut self) -> &mut FilmcraftApp {
        &mut self.harness.state_mut().inner
    }
    fn capture_next_frame(&mut self) {
        *self.armed.borrow_mut() = true;
    }
    fn element_rect(&mut self, id: &str) -> egui::Rect {
        let els = self.ok("ui.elements", json!({ "prefix": "timeline.track." }));
        for e in els.as_array().unwrap() {
            if e["id"].as_str().unwrap_or("") == id {
                // rects come as [x, y, w, h]
                let r = e["rect"].as_array().unwrap();
                let (x, y, w, h) = (r[0].as_f64().unwrap() as f32, r[1].as_f64().unwrap() as f32, r[2].as_f64().unwrap() as f32, r[3].as_f64().unwrap() as f32);
                return egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
            }
        }
        panic!("no element {id}");
    }
}

/// True if a shape matching button `r` was painted with a clip that lets any of it show.
/// Header buttons scrolled past the split paint as degenerate/inverted rects (the row rect
/// intersects the half to a zero-or-negative-height sliver), so compare by x-span and
/// vertical center and judge visibility by the clip, not the (possibly inverted) rect.
fn painted(shapes: &[(egui::Rect, egui::Rect)], r: egui::Rect) -> bool {
    shapes.iter().any(|(sr, clip)| {
        let (x0, x1) = (sr.min.x.min(sr.max.x), sr.min.x.max(sr.max.x));
        let ymid = (sr.min.y + sr.max.y) / 2.0;
        (x0 - r.min.x).abs() < 2.0 && (x1 - r.max.x).abs() < 2.0 && (ymid - r.center().y).abs() < 14.0 && clip.min.y < r.max.y && clip.max.y > r.min.y
    })
}

fn setup() -> Driver {
    let mut d = Driver::demo();
    // Tall audio tracks so the audio half scrolls.
    d.ok("ui.set", json!({ "timeline": { "audioTrackHeight": 120.0 } }));
    d.frames(4);
    d
}

#[test]
fn scrolled_out_audio_buttons_are_not_painted_over_the_video_half() {
    let mut d = setup();
    let split_y = d.app().tl.layout.clone().expect("layout").split_y;
    // Scroll the audio half up: A1 leaves entirely, A2's upper button row sits over the video half.
    d.app().ui.timeline.a_scroll = 150.0;
    d.frames(3);
    let m = d.element_rect("timeline.track.A2.muted");
    let s = d.element_rect("timeline.track.A2.solo");
    assert!(m.max.y < split_y, "A2's M button should be above the split for this repro (m={m:?} split={split_y})");
    d.capture_next_frame();
    d.frames(1);
    let shapes = d.captured.borrow().clone();
    assert!(!shapes.is_empty(), "capture found no painted rects at all");
    for (name, r) in [("M", m), ("S", s)] {
        assert!(!painted(&shapes, r), "the {name} button of a scrolled audio track is painted over the video half at {r:?}");
    }
}

#[test]
fn visible_audio_buttons_are_still_painted() {
    let mut d = setup();
    // No scroll: A1's M button sits inside the audio half and must be drawn.
    let split_y = d.app().tl.layout.clone().expect("layout").split_y;
    let m = d.element_rect("timeline.track.A1.muted");
    assert!(m.min.y > split_y, "unscrolled A1 header is below the split: {m:?} split {split_y}");
    d.capture_next_frame();
    d.frames(1);
    let shapes = d.captured.borrow().clone();
    assert!(!shapes.is_empty(), "capture found no painted rects at all");
    assert!(painted(&shapes, m), "the M button of a visible audio track must be painted");
}
