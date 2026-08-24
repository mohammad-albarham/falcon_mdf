//! A hex dump widget: offsets, bytes and ASCII for a slice of the file.
//!
//! Rows are virtualized through `ScrollArea::show_rows`, so the widget can
//! be pointed at a whole data block without formatting the parts of it that
//! are not on screen. Nothing here allocates per byte of the buffer — only
//! per byte of the rows actually drawn.

/// How many bytes of a row are drawn before an extra space is inserted, so
/// the eye can count columns without following them one at a time.
const GROUP: usize = 8;

/// Display state for one hex view: the row width, and the go-to box.
pub struct HexView {
    bytes_per_row: usize,
    goto_text: String,
    /// Set when the go-to box resolves to a row, applied to the next frame's
    /// scroll area (an offset can only be given when the area is built).
    scroll_to_row: Option<usize>,
    /// Row the go-to box last landed on, highlighted so the answer is
    /// visible after the scroll rather than merely arrived at.
    marked_row: Option<usize>,
    /// What the go-to box has to say about its last input, if anything.
    message: Option<String>,
}

impl Default for HexView {
    fn default() -> Self {
        Self {
            bytes_per_row: 16,
            goto_text: String::new(),
            scroll_to_row: None,
            marked_row: None,
            message: None,
        }
    }
}

impl HexView {
    /// Draws `bytes`, labelling offsets from `base_address` — the file offset
    /// `bytes[0]` sits at, so the offset column reads as file addresses
    /// rather than as positions in a buffer the user never asked about.
    pub fn show(&mut self, ui: &mut egui::Ui, bytes: &[u8], base_address: u64) {
        self.toolbar(ui, bytes, base_address);

        if bytes.is_empty() {
            ui.weak("(no bytes)");
            return;
        }

        let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
        let n_rows = bytes.len().div_ceil(self.bytes_per_row);
        let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if let Some(row) = self.scroll_to_row.take() {
            area = area.vertical_scroll_offset(row as f32 * row_height);
        }
        area.show_rows(ui, row_height, n_rows, |ui, range| {
            // A dump row is as wide as it is; wrapping one would double its
            // height and put every row below it out of place.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            // Rows are drawn back to back: `show_rows` places them on the
            // assumption that each is exactly `row_height` tall, and any
            // spacing between them accumulates into drift.
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in range {
                self.row(ui, bytes, base_address, row);
            }
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, bytes: &[u8], base_address: u64) {
        ui.horizontal(|ui| {
            ui.label("Bytes per row:");
            for width in [8usize, 16, 32] {
                ui.selectable_value(&mut self.bytes_per_row, width, width.to_string());
            }
            ui.separator();
            ui.label("Go to offset:");
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.goto_text)
                    .desired_width(120.0)
                    .hint_text("0x1f40"),
            );
            let go = ui.button("Go").clicked()
                || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if go {
                self.go_to(bytes.len(), base_address);
            }
        });
        if let Some(message) = &self.message {
            ui.weak(message);
        }
    }

    /// Resolves the go-to box against the *file* offsets this view shows, so
    /// an address copied out of the block list can be pasted in as it stands.
    fn go_to(&mut self, len: usize, base_address: u64) {
        let text = self.goto_text.trim();
        if text.is_empty() {
            self.message = None;
            self.marked_row = None;
            return;
        }
        let parsed = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            Some(hex) => u64::from_str_radix(hex, 16).ok(),
            // Bare digits are ambiguous, and a hex dump is a place where hex
            // is the house language: `1f40` is read as hex, and so is `40`.
            None => u64::from_str_radix(text, 16)
                .ok()
                .or_else(|| text.parse::<u64>().ok()),
        };
        let Some(address) = parsed else {
            self.message = Some(format!("{text:?} is not an address"));
            self.marked_row = None;
            return;
        };
        let end = base_address + len as u64;
        if address < base_address || address >= end {
            self.message = Some(format!(
                "{address:#x} is outside this block, which covers {base_address:#x}..{end:#x}"
            ));
            self.marked_row = None;
            return;
        }
        let row = ((address - base_address) as usize) / self.bytes_per_row;
        self.scroll_to_row = Some(row);
        self.marked_row = Some(row);
        self.message = None;
    }

    fn row(&self, ui: &mut egui::Ui, bytes: &[u8], base_address: u64, row: usize) {
        let start = row * self.bytes_per_row;
        let end = (start + self.bytes_per_row).min(bytes.len());
        let chunk = &bytes[start..end];

        let mut hex = String::with_capacity(self.bytes_per_row * 3 + self.bytes_per_row / GROUP);
        let mut ascii = String::with_capacity(self.bytes_per_row);
        for i in 0..self.bytes_per_row {
            if i > 0 && i % GROUP == 0 {
                hex.push(' ');
            }
            match chunk.get(i) {
                // A short last row is padded rather than left ragged, so the
                // ASCII gutter stays in the same column on every row.
                Some(byte) => {
                    hex.push_str(&format!("{byte:02x} "));
                    ascii.push(if (0x20..=0x7e).contains(byte) {
                        *byte as char
                    } else {
                        '.'
                    });
                }
                None => {
                    hex.push_str("   ");
                    ascii.push(' ');
                }
            }
        }

        let text = format!("{:#010x}  {hex} |{ascii}|", base_address + start as u64);
        let mut rich = egui::RichText::new(text).monospace();
        if self.marked_row == Some(row) {
            rich = rich.background_color(ui.visuals().selection.bg_fill);
        }
        // The hover band has to be painted *behind* the row, and the row's
        // rectangle is only known once it has been laid out. Reserving a
        // shape slot first and filling it afterwards is how egui puts
        // something under a widget it has already drawn.
        let band = ui.painter().add(egui::Shape::Noop);
        let response = ui.add(egui::Label::new(rich).sense(egui::Sense::hover()));
        if response.hovered() {
            ui.painter().set(
                band,
                egui::epaint::RectShape::filled(
                    response.rect,
                    0.0,
                    ui.visuals().widgets.hovered.bg_fill.linear_multiply(0.3),
                ),
            );
        }
    }
}
