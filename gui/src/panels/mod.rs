//! UI panels. One file per panel.

pub mod batch;
pub mod blocks;
pub mod bus;
pub mod channel_list;
pub mod details;
pub mod gps;
pub mod hexdump;
pub mod metadata;
pub mod numeric;
pub mod plot;
pub mod stats;
pub mod table;
pub mod tree;
pub mod xy;

/// A plotted channel's colour marker. Painted rather than typed: the bundled
/// fonts have no glyph for U+25CF, so `colored_label(color, "●")` drew an
/// empty box instead of the channel's colour.
pub fn color_dot(ui: &mut egui::Ui, color: egui::Color32) -> egui::Response {
    let height = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(height * 0.8, height), egui::Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), height * 0.3, color);
    response
}
