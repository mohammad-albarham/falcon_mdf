//! Query matching for the channel search box.
//!
//! Pure `std`, nothing from `egui`: the panel compiles what the user typed
//! once with [`compile`], then asks [`matches`] about every channel name.
//! Plain backtracking over `Vec<char>` — a few thousand channel names is the
//! expected workload, so clarity beats cleverness.

/// How a query is matched against a channel's name and metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchMode {
    /// The query appears verbatim somewhere in the text, ignoring case.
    #[default]
    Substring,
    /// `*` matches any run of characters including none, `?` matches exactly
    /// one character, everything else is a literal. The whole text must
    /// match, not a part of it.
    Wildcard,
    /// A deliberately small regex subset: literals, `.`, postfix `*` `+` `?`
    /// on the single preceding item, `[abc]`, `[^abc]`, the `^` and `$`
    /// anchors, and `\` to quote one character. Without `^` or `$` the
    /// search is unanchored. Alternation `|` and groups `(` `)` are not
    /// supported and are rejected with an `Err` at compile time.
    Regex,
}

/// A compiled query, ready to match many strings.
#[derive(Debug)]
pub struct Pattern {
    kind: Kind,
}

#[derive(Debug)]
enum Kind {
    /// The empty query, which matches everything in every mode.
    Empty,
    Substring(Vec<char>),
    Wildcard(Vec<Wild>),
    Regex {
        anchored_start: bool,
        anchored_end: bool,
        items: Vec<Item>,
    },
}

#[derive(Debug)]
enum Wild {
    Lit(char),
    Star,
    Question,
}
#[derive(Debug)]
enum Atom {
    Lit(char),
    Dot,
    Class { negated: bool, members: Vec<char> },
}
#[derive(Debug, PartialEq, Eq)]
enum Quant {
    One,
    Star,
    Plus,
    Optional,
}
#[derive(Debug)]
struct Item {
    atom: Atom,
    quant: Quant,
}

/// One atom, exactly once: the building block quantifiers grow out of.
fn once(atom: Atom) -> Item {
    Item {
        atom,
        quant: Quant::One,
    }
}

/// Compiles `text` for `mode`. `Err` carries a human sentence naming what is
/// wrong. Matching is case-insensitive in every mode, and an empty `text`
/// matches everything.
pub fn compile(text: &str, mode: MatchMode) -> Result<Pattern, String> {
    if text.is_empty() {
        return Ok(Pattern { kind: Kind::Empty });
    }
    let chars: Vec<char> = text.to_lowercase().chars().collect();
    match mode {
        MatchMode::Substring => Ok(Pattern {
            kind: Kind::Substring(chars),
        }),
        MatchMode::Wildcard => {
            let wilds = chars
                .into_iter()
                .map(|c| match c {
                    '*' => Wild::Star,
                    '?' => Wild::Question,
                    lit => Wild::Lit(lit),
                })
                .collect();
            Ok(Pattern {
                kind: Kind::Wildcard(wilds),
            })
        }
        MatchMode::Regex => compile_regex(chars),
    }
}

/// True when `haystack` matches `pattern`. Case-insensitive in every mode.
pub fn matches(pattern: &Pattern, haystack: &str) -> bool {
    let hay: Vec<char> = haystack.to_lowercase().chars().collect();
    match &pattern.kind {
        Kind::Empty => true,
        Kind::Substring(needle) => hay.windows(needle.len()).any(|window| window == needle),
        Kind::Wildcard(wilds) => wildcard_match(wilds, &hay),
        Kind::Regex {
            anchored_start,
            anchored_end,
            items,
        } => regex_search(items, *anchored_start, *anchored_end, &hay),
    }
}

fn compile_regex(chars: Vec<char>) -> Result<Pattern, String> {
    let mut items: Vec<Item> = Vec::new();
    let mut anchored_start = false;
    let mut anchored_end = false;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '^' if i == 0 => anchored_start = true,
            '$' if i == chars.len() - 1 => anchored_end = true,
            '|' => return Err("alternation with `|` is not supported by this matcher".to_string()),
            '(' | ')' => {
                return Err("groups with `(` and `)` are not supported by this matcher".to_string())
            }
            c @ ('*' | '+' | '?') => {
                let quant = match c {
                    '*' => Quant::Star,
                    '+' => Quant::Plus,
                    _ => Quant::Optional,
                };
                match items.last_mut() {
                    Some(item) if item.quant == Quant::One => item.quant = quant,
                    _ => return Err(format!(
                        "`{c}` has nothing before it to repeat; it must follow a literal, `.`, or `[...]`"
                    )),
                }
            }
            '.' => items.push(once(Atom::Dot)),
            '\\' => {
                let quoted = chars.get(i + 1).copied().ok_or_else(|| {
                    "the pattern ends with a trailing `\\`; there is no character after it to quote"
                        .to_string()
                })?;
                items.push(once(Atom::Lit(quoted)));
                i += 1;
            }
            '[' => {
                let (atom, consumed) = parse_class(&chars[i + 1..])?;
                items.push(once(atom));
                i += consumed;
            }
            lit => items.push(once(Atom::Lit(lit))),
        }
        i += 1;
    }
    Ok(Pattern {
        kind: Kind::Regex {
            anchored_start,
            anchored_end,
            items,
        },
    })
}

/// Parses the body after `[` up to and including the closing `]`, returning
/// the class atom and how many characters were consumed.
fn parse_class(body: &[char]) -> Result<(Atom, usize), String> {
    let mut j = 0;
    let negated = body.first() == Some(&'^');
    if negated {
        j += 1;
    }
    let mut members = Vec::new();
    while j < body.len() {
        match body[j] {
            ']' => return Ok((Atom::Class { negated, members }, j + 1)),
            '\\' => {
                let quoted = body.get(j + 1).copied().ok_or_else(|| {
                    "the pattern ends with a trailing `\\`; there is no character after it to quote"
                        .to_string()
                })?;
                members.push(quoted);
                j += 2;
            }
            member => {
                members.push(member);
                j += 1;
            }
        }
    }
    Err("unclosed `[`: the character class is missing its closing `]`".to_string())
}

/// Plain backtracking: `*` either matches nothing or consumes one more char.
fn wildcard_match(pattern: &[Wild], hay: &[char]) -> bool {
    match (pattern.first(), hay.first()) {
        (None, None) => true,
        (Some(Wild::Star), _) => {
            wildcard_match(&pattern[1..], hay)
                || (!hay.is_empty() && wildcard_match(pattern, &hay[1..]))
        }
        (Some(Wild::Lit(lit)), Some(h)) if lit == h => wildcard_match(&pattern[1..], &hay[1..]),
        (Some(Wild::Question), Some(_)) => wildcard_match(&pattern[1..], &hay[1..]),
        _ => false,
    }
}

fn regex_search(items: &[Item], anchored_start: bool, anchored_end: bool, hay: &[char]) -> bool {
    if anchored_start {
        return match_here(items, hay, anchored_end);
    }
    (0..=hay.len()).any(|start| match_here(items, &hay[start..], anchored_end))
}

fn match_here(items: &[Item], hay: &[char], anchored_end: bool) -> bool {
    let Some((item, rest)) = items.split_first() else {
        return !anchored_end || hay.is_empty();
    };
    match item.quant {
        Quant::One => {
            atom_matches(&item.atom, hay.first().copied())
                && match_here(rest, &hay[1..], anchored_end)
        }
        Quant::Optional => {
            (atom_matches(&item.atom, hay.first().copied())
                && match_here(rest, &hay[1..], anchored_end))
                || match_here(rest, hay, anchored_end)
        }
        Quant::Star => match_repeat(&item.atom, rest, hay, anchored_end),
        Quant::Plus => {
            atom_matches(&item.atom, hay.first().copied())
                && match_repeat(&item.atom, rest, &hay[1..], anchored_end)
        }
    }
}

/// `atom*` followed by `rest`: consume as many characters as possible, then
/// back off one at a time until `rest` fits.
fn match_repeat(atom: &Atom, rest: &[Item], hay: &[char], anchored_end: bool) -> bool {
    if atom_matches(atom, hay.first().copied()) && match_repeat(atom, rest, &hay[1..], anchored_end)
    {
        return true;
    }
    match_here(rest, hay, anchored_end)
}

fn atom_matches(atom: &Atom, c: Option<char>) -> bool {
    match (atom, c) {
        (_, None) => false,
        (Atom::Lit(lit), Some(c)) => *lit == c,
        (Atom::Dot, Some(_)) => true,
        (Atom::Class { negated, members }, Some(c)) => members.contains(&c) != *negated,
    }
}
