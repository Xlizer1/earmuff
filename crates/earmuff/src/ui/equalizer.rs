//! The equalizer: a fader per band, with the curve they make drawn behind them through
//! each fader's knob, so dragging a knob drags the curve. The faders are real sliders:
//! keyboard and screen readers work on them as on any other.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use earmuff_domain::Gain;

use super::view_model::CurveView;

/// Room on the left for the dB labels
const SCALE_WIDTH: i32 = 44;

pub struct EqualizerEditor {
    pub widget: gtk::Box,
    curve: gtk::DrawingArea,
    sliders: Vec<gtk::Scale>,
    values: Vec<gtk::Label>,
    /// Set while `show` moves the sliders, so that doesn't count as a change
    quiet: Rc<Cell<bool>>,
    baseline: f64,
}

impl EqualizerEditor {
    /// `changed` runs whenever the user moves a band
    pub fn new(view: &CurveView, changed: impl Fn() + 'static) -> Rc<Self> {
        let (min, max, baseline) = (view.min.db(), view.max.db(), view.baseline.db());
        let faders = gtk::Box::builder().homogeneous(true).hexpand(true).margin_start(SCALE_WIDTH).build();
        let curve = gtk::DrawingArea::builder().name("equalizer-curve").css_classes(["eq-preview"]).build();
        let changed = Rc::new(changed);
        let quiet = Rc::new(Cell::new(false));
        let (mut sliders, mut values) = (Vec::new(), Vec::new());
        for (band, label) in view.labels.iter().enumerate() {
            let slider = gtk::Scale::with_range(gtk::Orientation::Vertical, min, max, view.step.db().max(0.1));
            slider.set_widget_name(&format!("eq-band-{band}"));
            slider.set_inverted(true);
            // no bar from the bottom: a cut would look like a small boost
            slider.set_has_origin(false);
            slider.set_draw_value(false);
            slider.set_round_digits(1);
            slider.set_height_request(200);
            slider.set_halign(gtk::Align::Center);
            slider.set_tooltip_text(Some(&format!("{label} Hz")));
            slider.update_property(&[gtk::accessible::Property::Label(&format!("{label} Hz band"))]);
            let value = gtk::Label::builder().css_classes(["eq-value"]).build();
            let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
            column.append(&value);
            column.append(&slider);
            column.append(&gtk::Label::builder().label(label.as_str()).css_classes(["eq-band", "dim-label"]).build());
            faders.append(&column);

            let (shown, curve, changed, quiet) = (value.clone(), curve.clone(), changed.clone(), quiet.clone());
            slider.connect_value_changed(move |slider| {
                shown.set_label(&gain_text(slider.value()));
                curve.queue_draw();
                if !quiet.get() {
                    changed();
                }
            });
            sliders.push(slider);
            values.push(value);
        }

        let drawn = sliders.clone();
        curve.set_draw_func(move |area, cr, width, _| draw(area, cr, f64::from(width), &drawn, (min, max, baseline)));
        // the faders size the editor; the curve fills the space behind them
        let stage = gtk::Overlay::builder().child(&curve).build();
        stage.add_overlay(&faders);
        stage.set_measure_overlay(&faders, true);

        let flat = gtk::Button::builder()
            .label("Reset")
            .name("equalizer-flat")
            .halign(gtk::Align::End)
            .css_classes(["flat"])
            .tooltip_text("Set every band to 0 dB")
            .build();
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .name("equalizer")
            .css_classes(["eq-editor"])
            .build();
        widget.append(&stage);
        widget.append(&flat);

        let editor = Rc::new(EqualizerEditor { widget, curve, sliders, values, quiet, baseline });
        let weak = Rc::downgrade(&editor);
        flat.connect_clicked(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.flatten();
            }
        });
        editor.show(view);
        editor
    }

    /// The curve as the sliders have it
    pub fn gains(&self) -> Vec<Gain> {
        self.sliders.iter().map(|slider| Gain::from_db(slider.value())).collect()
    }

    pub fn show(&self, view: &CurveView) {
        self.quiet.set(true);
        for ((slider, value), gain) in self.sliders.iter().zip(&self.values).zip(&view.gains) {
            slider.set_value(gain.db());
            value.set_label(&gain_text(slider.value()));
        }
        self.quiet.set(false);
        self.curve.queue_draw();
    }

    fn flatten(&self) {
        for slider in &self.sliders {
            slider.set_value(self.baseline);
        }
    }
}

/// "+6", "-2.5", "0"
fn gain_text(db: f64) -> String {
    if db.abs() < 0.05 { "0".into() } else { format!("{:+}", (db * 10.0).round() / 10.0) }
}

/// Where a fader's knob is, in the curve's coordinates. `value` picks a gain other than
/// the fader's own, for the guide lines.
fn knob(slider: &gtk::Scale, area: &gtk::DrawingArea, value: Option<f64>, range: (f64, f64)) -> Option<(f64, f64)> {
    let (start, end) = slider.slider_range();
    let center = match value {
        None => f64::from(start + end) / 2.0,
        Some(value) => {
            // the knob travels the trough, half its length short of either end
            let trough = slider.range_rect();
            let half = f64::from(end - start) / 2.0;
            let travel = f64::from(trough.height()) - 2.0 * half;
            f64::from(trough.y()) + half + (range.1 - value) / (range.1 - range.0) * travel
        }
    };
    let point = gtk::graphene::Point::new(slider.width() as f32 / 2.0, center as f32);
    slider.compute_point(area, &point).map(|p| (f64::from(p.x()), f64::from(p.y())))
}

/// The guide lines, then a smooth line through the knobs, flat out to both edges
fn draw(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, width: f64, sliders: &[gtk::Scale], range: (f64, f64, f64)) {
    let (min, max, baseline) = range;
    let Some(first) = sliders.first() else { return };
    let points: Vec<(f64, f64)> = sliders.iter().filter_map(|slider| knob(slider, area, None, (min, max))).collect();
    if points.len() != sliders.len() || max <= min {
        return;
    }
    let level = |db: f64| knob(first, area, Some(db), (min, max)).map_or(0.0, |(_, y)| y);
    let left = f64::from(SCALE_WIDTH);

    // a line every 6 dB, the 0 dB one brighter, labelled on the left
    cr.set_line_width(1.0);
    cr.set_font_size(11.0);
    let mut db = (min / 6.0).ceil() * 6.0;
    while db <= max {
        let y = level(db).round() + 0.5;
        let strength = if (db - baseline).abs() < 0.05 { 0.14 } else { 0.06 };
        cr.set_source_rgba(1.0, 1.0, 1.0, strength);
        cr.move_to(left, y);
        cr.line_to(width, y);
        let _ = cr.stroke();
        if db == min || db == max || (db - baseline).abs() < 0.05 {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.5);
            cr.move_to(0.0, y + 4.0);
            let _ = cr.show_text(&if db > 0.0 { format!("+{db} dB") } else { format!("{db} dB") });
        }
        db += 6.0;
    }

    let mut padded = vec![(left, points[0].1)];
    padded.extend(&points);
    padded.push((width, points[points.len() - 1].1));
    let trace = || {
        cr.move_to(padded[0].0, padded[0].1);
        for i in 0..padded.len() - 1 {
            let p0 = padded[i.saturating_sub(1)];
            let (p1, p2) = (padded[i], padded[i + 1]);
            let p3 = padded[(i + 2).min(padded.len() - 1)];
            cr.curve_to(
                p1.0 + (p2.0 - p0.0) / 6.0,
                p1.1 + (p2.1 - p0.1) / 6.0,
                p2.0 - (p3.0 - p1.0) / 6.0,
                p2.1 - (p3.1 - p1.1) / 6.0,
                p2.0,
                p2.1,
            );
        }
    };
    let color = area.color();
    let (r, g, b) = (f64::from(color.red()), f64::from(color.green()), f64::from(color.blue()));
    let zero = level(baseline);
    trace();
    cr.line_to(width, zero);
    cr.line_to(left, zero);
    cr.close_path();
    cr.set_source_rgba(r, g, b, 0.10);
    let _ = cr.fill();
    trace();
    cr.set_source_rgba(r, g, b, 1.0);
    cr.set_line_width(2.5);
    let _ = cr.stroke();
}
