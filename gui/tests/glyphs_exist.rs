//! Every symbol the viewer writes into its UI must exist in egui's bundled
//! fonts. A missing one does not fail anywhere else: it renders as an empty
//! box, which is how the plotted-channel colour dot (U+25CF) and the channel
//! hierarchy icon went unnoticed.

use std::collections::BTreeMap;
use std::path::Path;

use egui::epaint::text::{Fonts, TextOptions};
use egui::{FontDefinitions, FontId};

/// Symbols the sources name as `\u{…}` escapes or write literally, with the
/// first file that uses each. Letters and punctuation below U+2000 are covered
/// by the text font and would only add noise.
fn symbols_in_sources() -> BTreeMap<char, String> {
    let mut found = BTreeMap::new();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            for line in text.lines() {
                // Comments may use any character; only code reaches the UI.
                let code = line.split("//").next().unwrap_or("");
                let mut rest = code;
                while let Some(at) = rest.find("\\u{") {
                    let tail = &rest[at + 3..];
                    let end = tail.find('}').unwrap();
                    let c = char::from_u32(u32::from_str_radix(&tail[..end], 16).unwrap()).unwrap();
                    found.entry(c).or_insert_with(|| name.clone());
                    rest = &tail[end..];
                }
                for c in code.chars().filter(|&c| c as u32 >= 0x2000) {
                    found.entry(c).or_insert_with(|| name.clone());
                }
            }
        }
    }
    found
}

#[test]
fn every_ui_symbol_has_a_glyph() {
    let mut fonts = Fonts::new(TextOptions::default(), FontDefinitions::default());
    let mut view = fonts.with_pixels_per_point(1.0);
    let font = FontId::proportional(14.0);
    let missing: Vec<String> = symbols_in_sources()
        .into_iter()
        // Variation selectors and joiners are invisible by design.
        .filter(|(c, _)| !matches!(*c as u32, 0x200B..=0x200F | 0xFE00..=0xFE0F))
        // Not `has_glyph`: it reports false for any symbol that lives in the
        // same face as the replacement glyph (the emoji icon font), even
        // though it draws. A width of zero is the face really lacking it.
        .filter(|(c, _)| view.glyph_width(&font, *c) <= 0.0)
        .map(|(c, file)| format!("U+{:04X} {c} (first used in {file})", c as u32))
        .collect();
    assert!(
        missing.is_empty(),
        "symbols with no glyph:\n{}",
        missing.join("\n")
    );
}
