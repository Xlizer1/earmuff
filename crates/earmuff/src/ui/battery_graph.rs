//! A line graph of the battery over the last day or week.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use earmuff_application::Sample;

pub const DAY: u64 = 24 * 60 * 60;
pub const WEEK: u64 = 7 * DAY;
/// A longer gap between readings means the headset was off: the line breaks there
const GAP: u64 = 30 * 60;

#[derive(Default)]
struct Data {
    samples: Vec<Sample>,
    now: u64,
}

pub struct BatteryGraph {
    pub widget: gtk::DrawingArea,
    data: Rc<RefCell<Data>>,
    span: Rc<Cell<u64>>,
}

impl BatteryGraph {
    pub fn new() -> Self {
        let widget = gtk::DrawingArea::builder()
            .name("battery-graph")
            .content_height(150)
            .hexpand(true)
            .css_classes(["battery-graph"])
            .build();
        let data: Rc<RefCell<Data>> = Rc::default();
        let span = Rc::new(Cell::new(DAY));
        let (drawn, spanned) = (data.clone(), span.clone());
        widget.set_draw_func(move |area, cr, width, height| {
            let data = drawn.borrow();
            draw(area, cr, f64::from(width), f64::from(height), &data.samples, data.now, spanned.get());
        });
        BatteryGraph { widget, data, span }
    }

    pub fn show(&self, samples: &[Sample], now: u64) {
        *self.data.borrow_mut() = Data { samples: samples.to_vec(), now };
        self.widget.queue_draw();
    }

    /// How far back the graph goes, in seconds
    pub fn set_span(&self, seconds: u64) {
        self.span.set(seconds);
        self.widget.queue_draw();
    }
}

fn draw(
    area: &gtk::DrawingArea,
    cr: &gtk::cairo::Context,
    width: f64,
    height: f64,
    samples: &[Sample],
    now: u64,
    span: u64,
) {
    let (left, right, top, bottom) = (4.0, width - 4.0, 8.0, height - 18.0);
    let x = |at: u64| left + (at.saturating_sub(now.saturating_sub(span)) as f64 / span as f64) * (right - left);
    let y = |level: u8| top + (100.0 - f64::from(level)) / 100.0 * (bottom - top);

    // faint lines at 0, 25, 50, 75 and 100%
    cr.set_line_width(1.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.07);
    for level in [0, 25, 50, 75, 100] {
        cr.move_to(left, y(level).round() + 0.5);
        cr.line_to(right, y(level).round() + 0.5);
    }
    let _ = cr.stroke();

    // labels for the time axis
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.45);
    cr.set_font_size(10.0);
    cr.move_to(left, height - 4.0);
    let _ = cr.show_text(if span >= WEEK { "7 days ago" } else { "24 hours ago" });
    if let Ok(extents) = cr.text_extents("now") {
        cr.move_to(right - extents.width(), height - 4.0);
        let _ = cr.show_text("now");
    }

    let shown: Vec<&Sample> = samples.iter().filter(|s| s.at + span >= now && s.at <= now).collect();
    if shown.len() < 2 {
        cr.set_font_size(12.0);
        let note = "Not enough readings yet";
        if let Ok(extents) = cr.text_extents(note) {
            cr.move_to((width - extents.width()) / 2.0, (top + bottom) / 2.0);
            let _ = cr.show_text(note);
        }
        return;
    }

    // one stretch per run of readings without a long gap
    let color = area.color();
    let (r, g, b) = (f64::from(color.red()), f64::from(color.green()), f64::from(color.blue()));
    let mut runs: Vec<Vec<&Sample>> = Vec::new();
    for sample in shown {
        match runs.last_mut() {
            Some(run) if sample.at.saturating_sub(run.last().map_or(0, |s| s.at)) <= GAP => run.push(sample),
            _ => runs.push(vec![sample]),
        }
    }
    for run in runs.iter().filter(|run| run.len() > 1) {
        let trace = || {
            cr.move_to(x(run[0].at), y(run[0].level));
            for sample in &run[1..] {
                cr.line_to(x(sample.at), y(sample.level));
            }
        };
        trace();
        cr.line_to(x(run[run.len() - 1].at), bottom);
        cr.line_to(x(run[0].at), bottom);
        cr.close_path();
        cr.set_source_rgba(r, g, b, 0.18);
        let _ = cr.fill();
        trace();
        cr.set_source_rgba(r, g, b, 1.0);
        cr.set_line_width(2.5);
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        let _ = cr.stroke();
    }
}
