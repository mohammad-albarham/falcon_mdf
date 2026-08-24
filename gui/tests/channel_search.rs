//! Tests for the channel search matcher: substring, wildcard globs, and the
//! small supported regex subset.

use falcon_mdf_gui::search::{compile, matches, MatchMode, Pattern};

fn compiled(text: &str, mode: MatchMode) -> Pattern {
    compile(text, mode).unwrap_or_else(|err| panic!("`{text}` must compile as {mode:?}: {err}"))
}

#[test]
fn substring_ignores_case_and_finds_the_middle() {
    let pattern = compiled("vEhIcLe", MatchMode::Substring);
    assert!(matches(&pattern, "VehicleSpeed"), "case must not matter");
    assert!(
        matches(&pattern, "the vehicle speed sensor"),
        "a hit in the middle counts"
    );
    assert!(
        !matches(&pattern, "wheel speed"),
        "unrelated text must not match"
    );
}

#[test]
fn wildcard_star_matches_a_run_and_also_nothing() {
    let pattern = compiled("*speed*", MatchMode::Wildcard);
    assert!(
        matches(&pattern, "VehicleSpeedKmh"),
        "`*` swallows runs on both sides"
    );
    assert!(matches(&pattern, "speed"), "`*` also matches nothing");

    let upper = compiled("VEH*", MatchMode::Wildcard);
    assert!(
        matches(&upper, "vehicle_speed"),
        "wildcard matching is case-insensitive too"
    );
}

#[test]
fn wildcard_question_matches_exactly_one_character() {
    let pattern = compiled("a?c", MatchMode::Wildcard);
    assert!(matches(&pattern, "abc"), "`?` matches one character");
    assert!(!matches(&pattern, "ac"), "`?` cannot match zero characters");
    assert!(
        !matches(&pattern, "abbc"),
        "`?` cannot match two characters"
    );
}

#[test]
fn wildcard_must_match_the_whole_string() {
    let pattern = compiled("ab*", MatchMode::Wildcard);
    assert!(
        !matches(&pattern, "xab"),
        "matching only part of the haystack is not enough"
    );
    assert!(
        matches(&pattern, "abcdef"),
        "matching from the first character to the end is fine"
    );
}

#[test]
fn regex_dot_and_character_classes() {
    let dot = compiled("t.mp", MatchMode::Regex);
    assert!(matches(&dot, "temp"), "`.` stands in for one character");
    assert!(!matches(&dot, "tmp"), "`.` still needs a character there");

    let class = compiled("ch[123]", MatchMode::Regex);
    assert!(
        matches(&class, "channel ch2 data"),
        "a listed member matches"
    );
    assert!(
        !matches(&class, "channel ch4 data"),
        "an unlisted member does not"
    );

    let negated = compiled("ab[^c]", MatchMode::Regex);
    assert!(
        matches(&negated, "abd"),
        "`[^c]` accepts any other character"
    );
    assert!(!matches(&negated, "abc"), "`[^c]` rejects `c`");
}

#[test]
fn regex_anchors_pin_the_start_and_end() {
    let at_start = compiled("^VEH", MatchMode::Regex);
    assert!(
        matches(&at_start, "VehicleSpeed"),
        "`^` pins the pattern to the start"
    );
    assert!(
        !matches(&at_start, "speedVehicle"),
        "`^` must not match in the middle"
    );

    let at_end = compiled("speed$", MatchMode::Regex);
    assert!(
        matches(&at_end, "VehicleSpeed"),
        "`$` pins the pattern to the end"
    );
    assert!(
        !matches(&at_end, "speedometer"),
        "`$` must not match mid-word"
    );

    let both = compiled("^a.c$", MatchMode::Regex);
    assert!(matches(&both, "abc"), "both anchors together");
    assert!(
        !matches(&both, "xabcy"),
        "an anchored pattern cannot match a longer string"
    );
}

#[test]
fn unanchored_regex_matches_in_the_middle() {
    let pattern = compiled("peed", MatchMode::Regex);
    assert!(
        matches(&pattern, "VehicleSpeedKmh"),
        "without anchors the search floats"
    );
}

#[test]
fn regex_quantifiers_repeat_only_the_preceding_item() {
    let star = compiled("ab*c", MatchMode::Regex);
    assert!(matches(&star, "ac"), "`*` allows zero");
    assert!(matches(&star, "abbbc"), "`*` allows a run");

    let plus = compiled("ab+c", MatchMode::Regex);
    assert!(!matches(&plus, "ac"), "`+` needs at least one");
    assert!(matches(&plus, "abbc"), "`+` allows a run");

    let optional = compiled("colou?r", MatchMode::Regex);
    assert!(matches(&optional, "color"), "`?` allows zero");
    assert!(matches(&optional, "colour"), "`?` allows one");
}

#[test]
fn empty_pattern_matches_everything_in_every_mode() {
    for mode in [MatchMode::Substring, MatchMode::Wildcard, MatchMode::Regex] {
        let pattern = compiled("", mode);
        assert!(
            matches(&pattern, "VehicleSpeed"),
            "empty {mode:?} matches anything"
        );
        assert!(
            matches(&pattern, ""),
            "empty {mode:?} matches the empty string too"
        );
    }
}

#[test]
fn malformed_patterns_become_errors_and_never_panic() {
    let unclosed = compile("wheel[ab", MatchMode::Regex).unwrap_err();
    assert!(
        unclosed.contains('['),
        "the error must name the offending bracket: {unclosed}"
    );

    assert!(
        compile("a|b", MatchMode::Regex).is_err(),
        "alternation is not supported"
    );
    assert!(
        compile("(ab)*", MatchMode::Regex).is_err(),
        "groups are not supported"
    );
    assert!(
        compile("ab\\", MatchMode::Regex).is_err(),
        "a trailing backslash is rejected"
    );
    assert!(
        compile("*ab", MatchMode::Regex).is_err(),
        "`*` with nothing before it is rejected"
    );
    assert!(
        compile("+", MatchMode::Regex).is_err(),
        "`+` with nothing before it is rejected"
    );
    assert!(
        compile("?x", MatchMode::Regex).is_err(),
        "`?` with nothing before it is rejected"
    );
}
