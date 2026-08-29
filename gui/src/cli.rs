//! What the launcher does with its command line.
//!
//! The viewer is shipped as a standalone binary, so it is run from a shell as
//! often as it is double-clicked, and a shell expects `--help` and
//! `--version` to work. Before this module the first argument was taken as a
//! path whatever it was, so `falcon --version` opened a window reporting that
//! it could not open a file called `--version` — a first impression that
//! reads as broken rather than as unsupported.
//!
//! Parsing lives here rather than in `main.rs` so it can be tested without
//! spawning a process or opening a window, the same reason the rest of the
//! viewer's logic is in this library.

use std::path::PathBuf;

/// What the arguments asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// Open the window, with this file if one was named.
    Window(Option<PathBuf>),
    /// Print [`HELP`] and exit successfully.
    Help,
    /// Print [`VERSION`] and exit successfully.
    Version,
    /// The arguments made no sense; this is why, for stderr.
    Usage(String),
}

/// The version line, naming the binary and the release it was built from.
pub const VERSION: &str = concat!("falcon ", env!("CARGO_PKG_VERSION"));

/// The `--help` text. Deliberately short: this is a window, and the window is
/// where everything else is explained.
pub const HELP: &str = "\
falcon — a desktop viewer for MF4 measurement files

STATUS:
    Pre-1.0 and not stable. The interface, the export formats and the saved
    session state may all change between versions, and coverage of what
    vendors emit is uneven. Check anything that matters against the source
    measurement. See RUNNING.md.

USAGE:
    falcon [OPTIONS] [FILE]

ARGS:
    <FILE>    An MDF file (.mf4 / .mdf) to open at startup. Without one,
              falcon opens an empty window; a file can then be opened from
              the top bar, dropped onto the window, or picked from the
              recent-files list.

OPTIONS:
    -h, --help       Print this help and exit
    -V, --version    Print the version and exit
        --           Treat what follows as a file name, for a file whose
                     name begins with a dash

Full documentation: gui/RUNNING.md in the falcon_mdf repository.";

/// Decides what the arguments asked for. Takes the arguments *after* the
/// program name, so a test can pass a list without faking `argv[0]`.
pub fn parse<I, S>(args: I) -> Launch
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut path: Option<PathBuf> = None;
    let mut args = args.into_iter().map(Into::into);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Launch::Help,
            "-V" | "--version" => return Launch::Version,
            // Everything after `--` is a file name, dashes and all. A
            // measurement can be called anything the filesystem allows, and
            // this is the only convention that lets a shell say so.
            "--" => {
                for rest in args.by_ref() {
                    if let Some(taken) = &path {
                        return Launch::Usage(too_many(taken, &rest));
                    }
                    path = Some(PathBuf::from(rest));
                }
                break;
            }
            // A bare `-` is a file called `-`, not a flag; anything else
            // starting with a dash is an option this build does not have.
            other if other.starts_with('-') && other != "-" => {
                return Launch::Usage(format!(
                    "unrecognised option '{other}'\n\nTry 'falcon --help'."
                ));
            }
            other => {
                if let Some(taken) = &path {
                    return Launch::Usage(too_many(taken, other));
                }
                path = Some(PathBuf::from(other));
            }
        }
    }

    Launch::Window(path)
}

/// Naming both files matters: the second one is usually a shell glob that
/// matched more than the person expected, and seeing the pair says so.
fn too_many(first: &std::path::Path, second: &str) -> String {
    format!(
        "falcon opens one file at a time, but two were given:\n  {}\n  {}\n\n\
         Open the second from the window, or start a second falcon.",
        first.display(),
        second
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(args: &[&str]) -> Launch {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_arguments_opens_an_empty_window() {
        assert_eq!(parse_str(&[]), Launch::Window(None));
    }

    #[test]
    fn a_path_opens_that_file() {
        assert_eq!(
            parse_str(&["measurement.mf4"]),
            Launch::Window(Some(PathBuf::from("measurement.mf4")))
        );
    }

    #[test]
    fn help_and_version_are_recognised_in_both_spellings() {
        assert_eq!(parse_str(&["-h"]), Launch::Help);
        assert_eq!(parse_str(&["--help"]), Launch::Help);
        assert_eq!(parse_str(&["-V"]), Launch::Version);
        assert_eq!(parse_str(&["--version"]), Launch::Version);
    }

    #[test]
    fn help_wins_over_a_path_so_it_never_opens_a_window() {
        assert_eq!(parse_str(&["a.mf4", "--help"]), Launch::Help);
    }

    #[test]
    fn an_unknown_flag_is_a_usage_error_naming_it() {
        let Launch::Usage(message) = parse_str(&["--plot"]) else {
            panic!("expected a usage error");
        };
        assert!(message.contains("--plot"), "{message}");
    }

    #[test]
    fn a_flag_is_not_mistaken_for_a_file_name() {
        // The bug this module exists for: `--version` used to become a path,
        // and the window reported that it could not be opened.
        assert!(!matches!(parse_str(&["--version"]), Launch::Window(_)));
    }

    #[test]
    fn two_files_are_a_usage_error_naming_both() {
        let Launch::Usage(message) = parse_str(&["a.mf4", "b.mf4"]) else {
            panic!("expected a usage error");
        };
        assert!(
            message.contains("a.mf4") && message.contains("b.mf4"),
            "{message}"
        );
    }

    #[test]
    fn a_dashed_file_name_can_be_opened_after_a_separator() {
        assert_eq!(
            parse_str(&["--", "--odd-name.mf4"]),
            Launch::Window(Some(PathBuf::from("--odd-name.mf4")))
        );
    }

    #[test]
    fn a_lone_dash_is_a_file_name_not_a_flag() {
        assert_eq!(parse_str(&["-"]), Launch::Window(Some(PathBuf::from("-"))));
    }

    #[test]
    fn the_version_line_names_the_binary_and_a_version() {
        assert!(VERSION.starts_with("falcon "), "{VERSION}");
        assert!(VERSION.len() > "falcon ".len(), "{VERSION}");
    }

    #[test]
    fn the_help_text_says_the_viewer_is_not_stable() {
        // The binary is the only part of a release some people ever read, so
        // the status belongs in `--help` and not only in a shipped Markdown
        // file they may never open.
        assert!(HELP.contains("not stable"), "{HELP}");
    }

    #[test]
    fn the_help_text_documents_every_option_it_accepts() {
        for flag in ["-h", "--help", "-V", "--version"] {
            assert!(HELP.contains(flag), "help does not mention {flag}");
        }
    }
}
