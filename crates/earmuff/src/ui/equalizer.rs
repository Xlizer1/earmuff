//! The equalizer: a slider per band, under a preview of the curve they make.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use earmuff_domain::Gain;

use super::view_model::CurveView;

pub struct EqualizerEditor {
    pub widget: gtk::Box,
    preview: gtk::DrawingArea,
    sliders: Vec<gtk::Scale>,
    values: Vec<gtk::Label>,
    gains: Rc<RefCell<Vec<f64>>>,
    /// Set while `show` moves the sliders, so that doesn't count as a change
    quiet: Rc<Cell<bool>>,
    baseline: f64,
}

impl EqualizerEditor {
    /// `changed` runs whenever the user moves a band
    pub fn new(view: &CurveView, changed: impl Fn() + 'static) -> Rc<Self> {
        let (min, max, baseline) = (view.min.db(), view.max.db(), view.baseline.db());
        let gains: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(view.gains.iter().map(|g| g.db()).collect()));

        let preview = gtk::DrawingArea::builder()
            .name("equalizer-preview")
            .content_height(90)
            .hexpand(true)
            .css_classes(["eq-preview"])
            .build();
        let drawn = gains.clone();
        preview.set_draw_func(move |area, cr, width, height| {
            draw(area, cr, f64::from(width), f64::from(height), &drawn.borrow(), (min, max, baseline));
        });

        let bands = gtk::Box::builder().homogeneous(true).hexpand(true).build();
        let changed = Rc::new(changed);
        let quiet = Rc::new(Cell::new(false));
        let (mut sliders, mut values) = (Vec::new(), Vec::new());
        for (band, label) in view.labels.iter().enumerate() {
            let slider = gtk::Scale::with_range(gtk::Orientation::Vertical, min, max, view.step.db().max(0.1));
            slider.set_widget_name(&format!("eq-band-{band}"));
            slider.set_inverted(true);
            slider.set_draw_value(false);
            slider.set_round_digits(1);
            slider.set_height_request(140);
            slider.set_halign(gtk::Align::Center);
            slider.add_mark(baseline, gtk::PositionType::Right, None);
            slider.set_tooltip_text(Some(&format!("{label} Hz")));
            slider.update_property(&[gtk::accessible::Property::Label(&format!("{label} Hz band"))]);
            let value = gtk::Label::builder().css_classes(["eq-value", "numeric"]).build();
            let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
            column.append(&value);
            column.append(&slider);
            column.append(&gtk::Label::builder().label(label.as_str()).css_classes(["eq-band", "dim-label"]).build());
            bands.append(&column);

            let (shown, gains, preview, changed, quiet) =
                (value.clone(), gains.clone(), preview.clone(), changed.clone(), quiet.clone());
            slider.connect_value_changed(move |slider| {
                shown.set_label(&gain_text(slider.value()));
                if let Some(gain) = gains.borrow_mut().get_mut(band) {
                    *gain = slider.value();
                }
                preview.queue_draw();
                if !quiet.get() {
                    changed();
                }
            });
            sliders.push(slider);
            values.push(value);
        }

        let flat = gtk::Button::builder()
            .label("Flat")
            .name("equalizer-flat")
            .halign(gtk::Align::End)
            .css_classes(["pill", "small"])
            .tooltip_text("Set every band to 0 dB")
            .build();
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .name("equalizer")
            .build();
        widget.append(&preview);
        widget.append(&bands);
        widget.append(&flat);

        let editor = Rc::new(EqualizerEditor { widget, preview, sliders, values, gains, quiet, baseline });
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
        *self.gains.borrow_mut() = self.sliders.iter().map(|slider| slider.value()).collect();
        self.quiet.set(false);
        self.preview.queue_draw();
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

/// A smooth line through the bands, each at the middle of its slider's column
fn draw(
    area: &gtk::DrawingArea,
    cr: &gtk::cairo::Context,
    width: f64,
    height: f64,
    gains: &[f64],
    range: (f64, f64, f64),
) {
    let (min, max, baseline) = range;
    if gains.is_empty() || max <= min {
        return;
    }
    let (top, bottom) = (6.0, height - 6.0);
    let y = |db: f64| top + (max - db.clamp(min, max)) / (max - min) * (bottom - top);
    let column = width / gains.len() as f64;
    let points: Vec<(f64, f64)> =
        gains.iter().enumerate().map(|(band, db)| ((band as f64 + 0.5) * column, y(*db))).collect();

    // the 0 dB line
    cr.set_line_width(1.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.12);
    cr.move_to(0.0, y(baseline).round() + 0.5);
    cr.line_to(width, y(baseline).round() + 0.5);
    let _ = cr.stroke();

    // Catmull-Rom through the points, flat out to both edges
    let mut padded = vec![(0.0, points[0].1)];
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
    trace();
    cr.line_to(width, y(baseline));
    cr.line_to(0.0, y(baseline));
    cr.close_path();
    cr.set_source_rgba(r, g, b, 0.16);
    let _ = cr.fill();
    trace();
    cr.set_source_rgba(r, g, b, 1.0);
    cr.set_line_width(2.5);
    let _ = cr.stroke();
    for (x, y) in points {
        cr.arc(x, y, 3.5, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }
}
