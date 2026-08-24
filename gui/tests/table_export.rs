//! The table's sort, filter and CSV-export logic lives in free functions in
//! `gui/src/panels/table.rs` precisely so it can be pinned down here with
//! plain data — no `Ui`, no file, no decoded signals.

use falcon_mdf_gui::panels::table::{csv_row, matching_indices, sorted_indices, SortKey};

fn text(s: &str) -> SortKey {
    SortKey::Text(s.to_string())
}

fn row(cells: &[Option<&str>]) -> Vec<Option<String>> {
    cells.iter().map(|c| c.map(|s| s.to_string())).collect()
}

// --- csv_row -------------------------------------------------------------

#[test]
fn a_cell_with_a_comma_is_quoted() {
    assert_eq!(csv_row(&[Some("a,b".to_string())]), "\"a,b\"");
}

#[test]
fn a_cell_with_a_double_quote_has_it_doubled_and_is_quoted() {
    assert_eq!(
        csv_row(&[Some("say \"hi\"".to_string())]),
        "\"say \"\"hi\"\"\""
    );
}

#[test]
fn a_cell_with_a_newline_is_quoted() {
    assert_eq!(
        csv_row(&[Some("line one\nline two".to_string())]),
        "\"line one\nline two\""
    );
    // A bare carriage return breaks a naive line-based parser just the same.
    assert_eq!(csv_row(&[Some("a\rb".to_string())]), "\"a\rb\"");
}

#[test]
fn plain_cells_pass_through_unquoted_and_join_with_commas() {
    assert_eq!(
        csv_row(&[Some("1.5".to_string()), Some("hello".to_string())]),
        "1.5,hello"
    );
}

#[test]
fn an_empty_cell_and_a_none_cell_are_distinguishable() {
    let empty = csv_row(&[Some(String::new())]);
    let invalid = csv_row(&[None]);
    assert_eq!(empty, "");
    // None is an invalid sample; the export writes the em dash the table
    // shows for it, not an empty field.
    assert_eq!(invalid, "\u{2014}");
    assert_ne!(empty, invalid);
}

// --- sorted_indices --------------------------------------------------------

#[test]
fn numbers_sort_numerically_not_lexically() {
    let keys = vec![SortKey::Number(10.0), SortKey::Number(9.0)];
    // Lexical order would put "10" before "9".
    assert_eq!(sorted_indices(&keys, false), vec![1, 0]);
    assert_eq!(sorted_indices(&keys, true), vec![0, 1]);
}

#[test]
fn sorting_is_stable_for_equal_keys() {
    let keys = vec![
        SortKey::Number(1.0),
        SortKey::Number(1.0),
        SortKey::Number(1.0),
    ];
    assert_eq!(sorted_indices(&keys, false), vec![0, 1, 2]);
    assert_eq!(sorted_indices(&keys, true), vec![0, 1, 2]);
}

#[test]
fn equal_text_keys_keep_their_input_order() {
    // All three fold to the same lowercase key, so stability decides.
    let keys = vec![text("AB"), text("ab"), text("Ab")];
    assert_eq!(sorted_indices(&keys, false), vec![0, 1, 2]);
    assert_eq!(sorted_indices(&keys, true), vec![0, 1, 2]);
}

#[test]
fn invalid_samples_sort_last_ascending() {
    let keys = vec![SortKey::Invalid, SortKey::Number(1.0), SortKey::Number(2.0)];
    assert_eq!(sorted_indices(&keys, false), vec![1, 2, 0]);
}

#[test]
fn invalid_samples_sort_last_descending() {
    let keys = vec![SortKey::Invalid, SortKey::Number(1.0), SortKey::Number(2.0)];
    assert_eq!(sorted_indices(&keys, true), vec![2, 1, 0]);
}

#[test]
fn text_keys_sort_case_insensitively() {
    // Case-sensitive comparison would order "Banana" (capital B) before
    // "apple"; case-insensitive comparison orders by the words.
    let keys = vec![text("apple"), text("Banana"), text("cherry")];
    assert_eq!(sorted_indices(&keys, false), vec![0, 1, 2]);

    let keys = vec![text("cherry"), text("apple"), text("Banana")];
    assert_eq!(sorted_indices(&keys, false), vec![1, 2, 0]);
}

// --- matching_indices --------------------------------------------------------

#[test]
fn the_filter_is_case_insensitive() {
    let rows = vec![row(&[Some("Hello")]), row(&[Some("world")])];
    assert_eq!(matching_indices(&rows, "HELLO"), vec![0]);
    assert_eq!(matching_indices(&rows, "wOrLd"), vec![1]);
}

#[test]
fn the_filter_matches_any_column() {
    let rows = vec![
        row(&[Some("foo"), Some("bar")]),
        row(&[Some("baz"), Some("qux")]),
    ];
    assert_eq!(matching_indices(&rows, "qux"), vec![1]);
    assert_eq!(matching_indices(&rows, "FOO"), vec![0]);
}

#[test]
fn an_empty_query_keeps_every_row() {
    let rows = vec![row(&[Some("a")]), row(&[None]), row(&[Some("b")])];
    assert_eq!(matching_indices(&rows, ""), vec![0, 1, 2]);
}

#[test]
fn invalid_cells_carry_no_text_to_match() {
    // None is an invalid sample; its placeholder em dash is not searchable
    // text, so it matches nothing — only real values do.
    let rows = vec![row(&[None]), row(&[Some("x")])];
    assert_eq!(matching_indices(&rows, "x"), vec![1]);
    assert_eq!(matching_indices(&rows, "\u{2014}"), Vec::<usize>::new());
}

#[test]
fn a_substring_of_a_cell_matches() {
    let rows = vec![row(&[Some("speed 12.5")]), row(&[Some("other")])];
    assert_eq!(matching_indices(&rows, "12"), vec![0]);
}
