//! The two themes keep text readable. Contrast is the one property of a
//! palette a test can hold it to: a colour tweak that drops the weak text
//! below WCAG AA on the striped rows fails here instead of in a screenshot.

use egui::{Color32, Theme};
use falcon_mdf_gui::theme::visuals_of;

fn luminance(c: Color32) -> f64 {
    let channel = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
}

fn contrast(a: Color32, b: Color32) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[test]
fn text_meets_wcag_aa_on_every_background() {
    for theme in [Theme::Light, Theme::Dark] {
        let v = visuals_of(theme);
        let backgrounds = [
            ("panel", v.panel_fill),
            ("striped row", v.faint_bg_color),
            ("window", v.window_fill),
            ("text field", v.extreme_bg_color),
        ];
        let foregrounds = [
            ("text", v.text_color(), 7.0),
            ("weak text", v.weak_text_color(), 4.5),
            ("warning", v.warn_fg_color, 4.5),
            ("error", v.error_fg_color, 4.5),
            ("link", v.hyperlink_color, 4.5),
        ];
        for (bg_name, bg) in backgrounds {
            for (fg_name, fg, min) in foregrounds {
                let ratio = contrast(fg, bg);
                assert!(
                    ratio >= min,
                    "{theme:?}: {fg_name} on {bg_name} is {ratio:.2}:1, needs {min}:1"
                );
            }
        }
        let selected = contrast(v.selection.stroke.color, v.selection.bg_fill);
        assert!(
            selected >= 4.5,
            "{theme:?}: selected-tab text is {selected:.2}:1"
        );
    }
}
