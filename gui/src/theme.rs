//! The viewer's look: colours, fonts and the one icon set.
//!
//! The accent is the teal of the signal line the mascot sits on, so the app
//! and its icon read as one thing; the mascot's orange is left to warnings,
//! which is what an orange line in a measurement tool already means. Both
//! themes are set here and nowhere else — a panel that needs a colour with a
//! meaning (an error, a warning) asks `ui.visuals()` for it rather than
//! naming an RGB value that is only right against one background.

use egui::{Color32, CornerRadius, FontDefinitions, FontFamily, RichText, Shadow, Stroke, Theme};

/// Every icon the viewer draws, from one face. A re-export rather than a
/// list of chosen constants so a new panel picks from the whole set, and
/// still cannot reach for an emoji by accident.
pub use egui_phosphor::regular as icon;

/// egui's default fonts plus the Phosphor icon font, as fallbacks for both
/// families: an icon in a monospace label (a sorted column header) has to
/// resolve too, not only one in body text.
pub fn font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
        mono.push("phosphor".into());
    }
    fonts
}

/// Fonts, both themes and the shared spacing, applied once at start-up.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
    ctx.set_visuals_of(Theme::Light, light());
    ctx.set_visuals_of(Theme::Dark, dark());
    // egui's defaults cram the toolbars: text buttons sit edge to edge.
    // A little more air keeps the strips of small buttons readable without
    // touching any panel's layout code.
    ctx.all_styles_mut(|style| {
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    });
}

/// A value the file holds rather than prose about it — an address, a block
/// ID, a count, a decoded sample — set in the monospace face so digits line
/// up down a column and `0x1f8` cannot be misread as a word.
pub fn mono(text: impl Into<String>) -> RichText {
    RichText::new(text).monospace()
}

/// Light theme. Text is near-black on an off-white panel rather than egui's
/// grey on grey; the weak text that carries hints and units stays above 4.5:1
/// against both the panel and the striped rows.
fn light() -> egui::Visuals {
    let text = Color32::from_rgb(0x1e, 0x23, 0x29);
    let strong = Color32::from_rgb(0x0b, 0x0e, 0x12);
    let panel = Color32::from_rgb(0xf6, 0xf7, 0xf9);
    let line = Color32::from_rgb(0xdc, 0xe0, 0xe5);
    let mut v = egui::Visuals::light();
    v.panel_fill = panel;
    v.window_fill = Color32::WHITE;
    v.extreme_bg_color = Color32::WHITE;
    v.faint_bg_color = Color32::from_rgb(0xee, 0xf1, 0xf4);
    v.code_bg_color = Color32::from_rgb(0xea, 0xee, 0xf2);
    v.weak_text_color = Some(Color32::from_rgb(0x58, 0x61, 0x6c));
    v.hyperlink_color = Color32::from_rgb(0x0a, 0x73, 0x67);
    v.warn_fg_color = Color32::from_rgb(0xa8, 0x4f, 0x08);
    v.error_fg_color = Color32::from_rgb(0xc0, 0x26, 0x26);
    v.selection.bg_fill = Color32::from_rgb(0xc9, 0xec, 0xe6);
    v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(0x08, 0x5c, 0x53));
    v.window_stroke = Stroke::new(1.0, line);
    v.window_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(28),
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(24),
    };
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = panel;
    w.noninteractive.weak_bg_fill = panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, line);
    w.noninteractive.fg_stroke = Stroke::new(1.0, text);
    w.inactive.bg_fill = Color32::from_rgb(0xe6, 0xe9, 0xed);
    w.inactive.weak_bg_fill = Color32::from_rgb(0xe9, 0xec, 0xf0);
    w.inactive.fg_stroke = Stroke::new(1.0, text);
    w.hovered.bg_fill = Color32::from_rgb(0xdb, 0xe0, 0xe6);
    w.hovered.weak_bg_fill = Color32::from_rgb(0xdf, 0xe3, 0xe8);
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xb9, 0xc1, 0xca));
    w.hovered.fg_stroke = Stroke::new(1.5, strong);
    w.active.bg_fill = Color32::from_rgb(0xcf, 0xd6, 0xde);
    w.active.weak_bg_fill = Color32::from_rgb(0xd3, 0xd9, 0xe0);
    w.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x0f, 0x8f, 0x80));
    w.active.fg_stroke = Stroke::new(2.0, strong);
    w.open.weak_bg_fill = Color32::from_rgb(0xe3, 0xe7, 0xeb);
    w.open.fg_stroke = Stroke::new(1.0, strong);
    shape(&mut v);
    v
}

/// Dark theme: a blue-grey ramp rather than egui's neutral greys, with the
/// accent lifted far enough to read on it.
fn dark() -> egui::Visuals {
    let text = Color32::from_rgb(0xe3, 0xe6, 0xea);
    let strong = Color32::WHITE;
    let panel = Color32::from_rgb(0x17, 0x19, 0x1d);
    let line = Color32::from_rgb(0x2e, 0x33, 0x3a);
    let mut v = egui::Visuals::dark();
    v.panel_fill = panel;
    v.window_fill = Color32::from_rgb(0x1f, 0x22, 0x27);
    v.extreme_bg_color = Color32::from_rgb(0x10, 0x12, 0x15);
    v.faint_bg_color = Color32::from_rgb(0x1f, 0x23, 0x28);
    v.code_bg_color = Color32::from_rgb(0x25, 0x29, 0x2f);
    v.weak_text_color = Some(Color32::from_rgb(0x9c, 0xa4, 0xae));
    v.hyperlink_color = Color32::from_rgb(0x4f, 0xd1, 0xc0);
    v.warn_fg_color = Color32::from_rgb(0xf2, 0xa1, 0x4a);
    v.error_fg_color = Color32::from_rgb(0xf3, 0x76, 0x76);
    v.selection.bg_fill = Color32::from_rgb(0x12, 0x4d, 0x47);
    v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(0x8c, 0xe8, 0xda));
    v.window_stroke = Stroke::new(1.0, line);
    v.window_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(90),
    };
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = panel;
    w.noninteractive.weak_bg_fill = panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, line);
    w.noninteractive.fg_stroke = Stroke::new(1.0, text);
    w.inactive.bg_fill = Color32::from_rgb(0x2a, 0x2e, 0x35);
    w.inactive.weak_bg_fill = Color32::from_rgb(0x27, 0x2b, 0x31);
    w.inactive.fg_stroke = Stroke::new(1.0, text);
    w.hovered.bg_fill = Color32::from_rgb(0x34, 0x39, 0x41);
    w.hovered.weak_bg_fill = Color32::from_rgb(0x31, 0x36, 0x3d);
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x4b, 0x52, 0x5c));
    w.hovered.fg_stroke = Stroke::new(1.5, strong);
    w.active.bg_fill = Color32::from_rgb(0x3c, 0x42, 0x4b);
    w.active.weak_bg_fill = Color32::from_rgb(0x3a, 0x40, 0x48);
    w.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x4f, 0xd1, 0xc0));
    w.active.fg_stroke = Stroke::new(2.0, strong);
    w.open.weak_bg_fill = Color32::from_rgb(0x2d, 0x32, 0x39);
    w.open.fg_stroke = Stroke::new(1.0, strong);
    shape(&mut v);
    v
}

/// Rounding shared by both themes: one radius across every widget kind a
/// toolbar mixes, and a slightly larger one for the surfaces that float.
fn shape(v: &mut egui::Visuals) {
    for widget in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::same(6);
    }
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
}

/// The light/dark switch, drawn with the icon set rather than egui's own
/// toggle, whose moon and sun come from the emoji font.
pub fn theme_switch(ui: &mut egui::Ui) {
    let dark = ui.visuals().dark_mode;
    let (glyph, tip, next) = if dark {
        (icon::SUN, "Switch to the light theme", Theme::Light)
    } else {
        (icon::MOON, "Switch to the dark theme", Theme::Dark)
    };
    if ui.button(glyph).named(tip).on_hover_text(tip).clicked() {
        ui.ctx().set_theme(next);
    }
}

/// Light and dark visuals as installed, for tests that check contrast
/// without a window.
pub fn visuals_of(theme: Theme) -> egui::Visuals {
    match theme {
        Theme::Light => light(),
        Theme::Dark => dark(),
    }
}

/// A name for an icon-only widget. Its glyph sits in Phosphor's private-use
/// codepoints, so without this a screen reader — and the accessibility tree
/// the UI tests drive the app through — sees an unlabelled button.
pub trait Named {
    fn named(self, name: &str) -> Self;
}

impl Named for egui::Response {
    fn named(self, name: &str) -> Self {
        let enabled = self.enabled();
        self.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
        self
    }
}
