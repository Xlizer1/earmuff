//! A ring that fills with the battery level, easing between readings.

use std::cell::Cell;
use std::f64::consts::{FRAC_PI_2, TAU};
use std::rc::Rc;

use adw::prelude::*;

const LINE_WIDTH: f64 = 11.0;

pub struct BatteryRing {
    pub widget: gtk::Overlay,
    area: gtk::DrawingArea,
    percent: gtk::Label,
    state: gtk::Label,
    /// The fraction currently drawn, which the animation moves toward the real level
    shown: Rc<Cell<f64>>,
    animation: adw::TimedAnimation,
}

impl BatteryRing {
    pub fn new() -> Self {
        let area = gtk::DrawingArea::builder()
            .name("battery-ring")
            .content_width(156)
            .content_height(156)
            .css_classes(["battery-ring"])
            .build();
        let shown = Rc::new(Cell::new(0.0));
        let drawn = shown.clone();
        area.set_draw_func(move |area, cr, width, height| draw(area, cr, width, height, drawn.get()));

        let percent = gtk::Label::builder().name("battery-label").css_classes(["ring-percent"]).build();
        let state = gtk::Label::builder().name("battery-state").css_classes(["ring-state"]).build();
        let labels = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();
        labels.append(&percent);
        labels.append(&state);
        let widget = gtk::Overlay::builder().child(&area).valign(gtk::Align::Center).build();
        widget.add_overlay(&labels);

        let (target_shown, target_area) = (shown.clone(), area.clone());
        let target = adw::CallbackAnimationTarget::new(move |value| {
            target_shown.set(value);
            target_area.queue_draw();
        });
        let animation = adw::TimedAnimation::builder()
            .widget(&area)
            .value_from(0.0)
            .value_to(0.0)
            .duration(700)
            .easing(adw::Easing::EaseOutCubic)
            .target(&target)
            .build();
        BatteryRing { widget, area, percent, state, shown, animation }
    }

    /// Shows a battery reading; the ring eases to the new level
    pub fn show(&self, level: Option<u8>, text: &str, state: &str, low: bool, charging: bool) {
        self.percent.set_label(if text.is_empty() { "–" } else { text });
        self.state.set_label(state);
        for (class, on) in [("low", low), ("charging", charging)] {
            if on {
                self.area.add_css_class(class);
            } else {
                self.area.remove_css_class(class);
            }
        }
        let target = level.map_or(0.0, |level| f64::from(level) / 100.0);
        if (target - self.animation.value_to()).abs() > f64::EPSILON {
            self.animation.set_value_from(self.shown.get());
            self.animation.set_value_to(target);
            self.animation.play();
        }
    }
}

/// A faint full circle, and over it an arc in the widget's CSS color, clockwise from the top
fn draw(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, width: i32, height: i32, fraction: f64) {
    let (cx, cy) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
    let radius = (f64::from(width.min(height)) / 2.0 - LINE_WIDTH).max(1.0);
    cr.set_line_width(LINE_WIDTH);
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.08);
    cr.arc(cx, cy, radius, 0.0, TAU);
    let _ = cr.stroke();
    if fraction > 0.0 {
        let color = area.color();
        cr.set_source_rgba(color.red().into(), color.green().into(), color.blue().into(), color.alpha().into());
        cr.arc(cx, cy, radius, -FRAC_PI_2, -FRAC_PI_2 + TAU * fraction.min(1.0));
        let _ = cr.stroke();
    }
}
