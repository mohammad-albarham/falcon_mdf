//! What the viewer remembers about a file, pinned against the format it is
//! remembered in.
//!
//! The session store is written to disk by one run of the viewer and read by
//! the next, which makes its text format a compatibility surface: a line this
//! version cannot read must be skipped rather than taken as an excuse to
//! discard every other file's session, and a line it writes must come back as
//! what was written. Neither of those is visible from inside a running
//! window, so they are pinned here instead.

use std::path::{Path, PathBuf};

use falcon_mdf_gui::model::ChannelLoc;
use falcon_mdf_gui::session::{format_line, parse_line, Session};

fn loc(dg: usize, cg: usize, ch: usize) -> ChannelLoc {
    ChannelLoc {
        data_group_index: dg,
        channel_group_index: cg,
        channel_index: ch,
    }
}

fn session(plotted: Vec<ChannelLoc>) -> Session {
    Session {
        plotted,
        nav: "Blocks".to_string(),
        tab: "Samples".to_string(),
        cursor_a: None,
        cursor_b: None,
    }
}

#[test]
fn a_written_line_reads_back_as_what_was_written() {
    let path = PathBuf::from("/measurements/drive.mf4");
    let original = session(vec![loc(0, 0, 1), loc(2, 1, 7)]);

    let (read_path, read_session) =
        parse_line(&format_line(&path, &original)).expect("the line it wrote should parse");

    assert_eq!(read_path, path);
    assert_eq!(read_session, original);
}

#[test]
fn session_with_cursors_round_trips() {
    let path = PathBuf::from("/measurements/drive.mf4");
    let mut original = session(vec![loc(0, 0, 1), loc(2, 1, 7)]);
    original.cursor_a = Some(1.234567);
    original.cursor_b = Some(8.901234);

    let (read_path, read_session) =
        parse_line(&format_line(&path, &original)).expect("the line it wrote should parse");

    assert_eq!(read_path, path);
    assert_eq!(read_session, original);
    assert_eq!(read_session.cursor_a, Some(1.234567));
    assert_eq!(read_session.cursor_b, Some(8.901234));
}

#[test]
fn session_with_single_cursor_round_trips() {
    let path = PathBuf::from("/measurements/drive.mf4");
    let mut original = session(vec![loc(0, 0, 1)]);
    original.cursor_a = Some(4.5);
    original.cursor_b = None;

    let (read_path, read_session) =
        parse_line(&format_line(&path, &original)).expect("the line it wrote should parse");

    assert_eq!(read_path, path);
    assert_eq!(read_session, original);
    assert_eq!(read_session.cursor_a, Some(4.5));
    assert_eq!(read_session.cursor_b, None);
}

#[test]
fn a_file_with_nothing_plotted_round_trips() {
    // The empty plotted list writes an empty field, which is the one case a
    // naive split would turn into a single empty entry rather than none.
    let path = PathBuf::from("/measurements/empty.mf4");
    let original = session(Vec::new());

    let (_, read_session) = parse_line(&format_line(&path, &original)).expect("should parse");

    assert!(read_session.plotted.is_empty());
    assert_eq!(read_session.nav, "Blocks");
}

#[test]
fn a_path_with_spaces_survives() {
    let path = PathBuf::from("/Users/someone/My Measurements/run 3.mf4");
    let original = session(vec![loc(1, 0, 0)]);

    let (read_path, _) = parse_line(&format_line(&path, &original)).expect("should parse");

    assert_eq!(read_path, path);
}

#[test]
fn lines_from_a_version_that_wrote_more_are_refused_not_misread() {
    // Four numbers where three are expected means the entry was written by
    // something with a different idea of what identifies a channel. Guessing
    // which three were meant would restore the wrong channel silently.
    assert!(parse_line("/f.mf4\t0:0:1:9\tStructure\tPlot").is_none());
}

#[test]
fn malformed_lines_are_refused() {
    for line in [
        "",                               // nothing at all
        "\t0:0:1\tStructure\tPlot",       // no path
        "/f.mf4",                         // no plotted field
        "/f.mf4\t0:0\tStructure\tPlot",   // a location missing its channel
        "/f.mf4\tx:0:1\tStructure\tPlot", // a location that is not a number
    ] {
        assert!(
            parse_line(line).is_none(),
            "{line:?} should not parse as a session"
        );
    }
}

#[test]
fn a_line_missing_its_tabs_still_yields_the_file() {
    // Tab labels were added after the plotted list; a line written before
    // them should still restore the channels rather than being thrown away.
    let (path, session) = parse_line("/f.mf4\t0:0:1").expect("should parse");

    assert_eq!(path, Path::new("/f.mf4"));
    assert_eq!(session.plotted, vec![loc(0, 0, 1)]);
    assert_eq!(session.nav, "");
    assert_eq!(session.tab, "");
}

#[test]
fn the_plotted_list_keeps_its_order() {
    // Order is not decoration: it decides which colour each channel is given
    // when the session is restored.
    let path = PathBuf::from("/f.mf4");
    let original = session(vec![loc(3, 0, 0), loc(0, 0, 0), loc(1, 2, 3)]);

    let (_, read_session) = parse_line(&format_line(&path, &original)).expect("should parse");

    assert_eq!(read_session.plotted, original.plotted);
}

#[test]
fn a_very_long_plotted_list_is_capped_on_the_way_in() {
    // A stored line naming a thousand channels would otherwise restore a
    // thousand decodes on open.
    let many: Vec<ChannelLoc> = (0..200).map(|i| loc(0, 0, i)).collect();
    let line = format_line(Path::new("/f.mf4"), &session(many));

    let (_, read_session) = parse_line(&line).expect("should parse");

    assert!(
        read_session.plotted.len() <= 32,
        "restored {} channels, which is more than the cap",
        read_session.plotted.len()
    );
}
