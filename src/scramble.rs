//! Anonymising a measurement by replacing its text, byte for byte.
//!
//! Sharing a measurement means sharing the names of everything in it: channel
//! names, units, comments, the device that recorded it, the strings a
//! value-to-text table maps to. The numbers are usually the part worth sharing
//! and the names are the part that cannot be. [`scramble_file`] writes a copy
//! of a file with every piece of identifying text replaced by random letters
//! of the same byte length, and nothing else touched at all.
//!
//! ```no_run
//! use falcon_mdf::scramble_file;
//!
//! let report = scramble_file("measurement.mf4", "shareable.mf4", 0x5EED)?;
//! println!("{} text blocks randomised", report.blocks_scrambled);
//! # Ok::<(), falcon_mdf::error::Mf4Error>(())
//! ```
//!
//! # What is preserved
//!
//! Every byte that is not text inside a `##TX` or `##MD` block. The copy is
//! made with [`std::fs::copy`] and then patched in place, so block addresses,
//! lengths, link sections, the record layout and every sample byte are the
//! bytes of the original file. Within a text block the replacement runs only
//! up to the null terminator: the terminator itself and the padding a writer
//! left after it stay as they were, so the block's length field remains true
//! and the text keeps its original length. A reader of the scrambled file sees
//! names of the same shape as the ones it replaced.
//!
//! # What is not scrambled, and why
//!
//! Some text in an MF4 file is not a name but an instruction the file needs in
//! order to decode: randomising it would change the numbers, which is the one
//! thing this must never do. Those blocks are left alone and counted in
//! [`ScrambleReport::blocks_preserved`]:
//!
//! - the formula of an algebraic conversion (CC type 3), which is arithmetic,
//!   not a name;
//! - the *key* side of the text-keyed tables — every reference of a
//!   text-to-value table (type 9) and the even references of a text-to-text
//!   table (type 10). A key that no longer matches the sample text sends the
//!   lookup to its default and changes the result.
//!
//! The text those tables *produce* — value-to-text (type 7), range-to-text
//! (type 8) and the odd, replacement references of type 10 — is scrambled: it
//! is output, so it is identifying, and no number depends on it.
//!
//! # Limits
//!
//! - Only blocks reachable from the header block are visited, which is exactly
//!   the set a reader can see. Text in a region no link points at is left as
//!   it is rather than risk writing over the records of an unfinalized file,
//!   whose tail is uncovered by design.
//! - Channel *names* are what bus decoding matches on, so a scrambled
//!   bus-logging file no longer decodes to named CAN signals. Its frames and
//!   samples are unchanged.
//! - Sample data is never touched, so text carried as samples (a string
//!   channel's values) survives scrambling. Those are measurements, not
//!   metadata.
//! - Lengths are preserved, so a scrambled file still says how long each name
//!   was.

use std::collections::HashSet;
use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use crate::blocks::conversion::ConversionType;
use crate::blocks::BLOCK_HEADER_SIZE;
use crate::error::{Mf4Error, Result};
use crate::inspect::BlockMap;
use crate::io::{ByteSource, IoBackend};
use crate::parser::{parse_cc_block, parse_id_block};

/// What a [`scramble_file`] run changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrambleReport {
    /// Text blocks whose text was replaced.
    pub blocks_scrambled: usize,
    /// Bytes of text overwritten, terminators and padding excluded.
    pub bytes_scrambled: u64,
    /// Text blocks deliberately left alone because decoding reads them — see
    /// the module documentation for which ones and why.
    pub blocks_preserved: usize,
}

/// Writes `dst` as a copy of `src` with every identifying string randomised.
///
/// `seed` fixes the randomisation: the same file and seed give the same
/// output, which is what makes a scrambled file reproducible and testable.
/// The mapping is one-way regardless of the seed — the replacement text is
/// drawn without reference to the text it replaces, so no seed recovers an
/// original name.
///
/// Returns [`Mf4Error::Unsupported`] for a file that is not MF4; the block
/// walk this relies on is MF4's. MDF 3.x files are read by `falcon_mdf::mdf3` and
/// have a different block layout, so they are rejected by name rather than
/// copied through unchanged.
///
/// # Errors
///
/// Fails if `src` cannot be read, if it is not an MF4 file, or if `dst`
/// cannot be written. On a write failure `dst` is left partially patched and
/// should be discarded.
pub fn scramble_file<P: AsRef<Path>, Q: AsRef<Path>>(
    src: P,
    dst: Q,
    seed: u64,
) -> Result<ScrambleReport> {
    let src = src.as_ref();
    let dst = dst.as_ref();

    let patches = {
        let source = IoBackend::open(src)?;

        let id = parse_id_block(&source)?;
        if id.version_number < 400 {
            return Err(Mf4Error::unsupported(
                "scramble",
                format!(
                    "file declares MDF version {}, and this walks MF4 blocks; \
                     only version 400 and above can be scrambled",
                    id.version_number
                ),
            ));
        }

        plan_patches(&source, seed)?
    };

    fs::copy(src, dst)?;

    let mut out = fs::OpenOptions::new().write(true).open(dst)?;
    let mut report = ScrambleReport {
        blocks_scrambled: 0,
        bytes_scrambled: 0,
        blocks_preserved: patches.preserved,
    };
    for patch in &patches.writes {
        // The replacement is built by overwriting a copy of the block's data
        // in place, so it cannot change length. Checking anyway is the cheap
        // half of a bargain whose expensive half is a file whose every block
        // after this one has silently moved.
        if patch.replacement.len() as u64 != patch.original_len {
            return Err(Mf4Error::write_error(format!(
                "refusing to scramble: replacement text for the block at {:#x} is {} bytes \
                 where the original is {}, and writing it would shift every later block",
                patch.address,
                patch.replacement.len(),
                patch.original_len
            )));
        }
        out.seek(SeekFrom::Start(patch.address))?;
        out.write_all(&patch.replacement)?;
        report.blocks_scrambled += 1;
        report.bytes_scrambled += patch.changed;
    }
    out.flush()?;

    Ok(report)
}

/// One text block's data section, rewritten.
struct Patch {
    /// Where the block's data section starts in the file.
    address: u64,
    /// The bytes to write there.
    replacement: Vec<u8>,
    /// Length of the data section being replaced, which the replacement must
    /// match exactly.
    original_len: u64,
    /// How many of those bytes are text that changed.
    changed: u64,
}

/// The edits a scramble will make, worked out before anything is written.
struct Patches {
    /// Every block's rewritten data section.
    writes: Vec<Patch>,
    /// Text blocks skipped because decoding reads them.
    preserved: usize,
}

/// Walks the block graph and builds the replacement text for every text block
/// that is safe to replace.
fn plan_patches<S: ByteSource>(source: &S, seed: u64) -> Result<Patches> {
    let map = BlockMap::scan(source);
    let protected = decode_critical_text(source, &map);

    let mut rng = Rng::new(seed);
    let mut writes = Vec::new();
    let mut preserved = 0;

    for block in &map.blocks {
        let is_md = match block.block_type.as_str() {
            "##TX" => false,
            "##MD" => true,
            _ => continue,
        };

        if protected.contains(&block.address) {
            preserved += 1;
            continue;
        }

        let start = block.address + BLOCK_HEADER_SIZE as u64 + 8 * block.link_count;
        let len = block.data_size as usize;
        if len == 0 {
            continue;
        }

        let data = source.read_bytes(start, len)?;
        let (replacement, changed) = if is_md {
            scramble_markup(&data, &mut rng)
        } else {
            scramble_text(&data, &mut rng)
        };
        if changed > 0 {
            writes.push(Patch {
                address: start,
                replacement,
                original_len: len as u64,
                changed,
            });
        }
    }

    Ok(Patches { writes, preserved })
}

/// Returns the addresses of text blocks a conversion reads to produce numbers.
///
/// A block whose header will not parse is simply not protected; the walk that
/// found it already recorded the damage, and this is not the place to report
/// it a second time.
fn decode_critical_text<S: ByteSource>(source: &S, map: &BlockMap) -> HashSet<u64> {
    let mut protected = HashSet::new();

    for block in &map.blocks {
        if block.block_type != "##CC" {
            continue;
        }
        let Ok(cc) = parse_cc_block(source, block.address) else {
            continue;
        };

        match cc.conversion_type {
            // cc_ref[0] is the formula text.
            ConversionType::Algebraic => protected.extend(cc.references.first().copied()),
            // Every reference is a lookup key.
            ConversionType::TabTextToValue => protected.extend(cc.references.iter().copied()),
            // References alternate key, replacement, ending in a default; the
            // keys are the even ones.
            ConversionType::TabTextToText => {
                protected.extend(cc.references.iter().step_by(2).copied())
            }
            _ => {}
        }
    }

    protected.remove(&0);
    protected
}

/// Replaces a text block's string with random letters of the same byte length.
///
/// Returns the block's new data section and how many bytes of it changed.
/// Everything from the null terminator on is copied through untouched.
fn scramble_text(data: &[u8], rng: &mut Rng) -> (Vec<u8>, u64) {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    let mut out = data.to_vec();
    for byte in &mut out[..end] {
        *byte = rng.letter();
    }
    (out, end as u64)
}

/// Replaces the text content of an XML metadata block, leaving its markup.
///
/// An MD block carries its comment as XML. Randomising the whole payload the
/// way [`scramble_text`] does would leave a block that is no longer XML, so
/// only the runs between `>` and `<` are replaced — the element and attribute
/// names stay, the content they hold does not. Byte lengths are unchanged
/// either way, so this costs nothing over the cruder option.
fn scramble_markup(data: &[u8], rng: &mut Rng) -> (Vec<u8>, u64) {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    let mut out = data.to_vec();
    let mut in_tag = false;
    let mut changed = 0;

    for byte in &mut out[..end] {
        match *byte {
            b'<' => in_tag = true,
            b'>' => in_tag = false,
            // Whitespace between elements is layout, not content; keeping it
            // stops a pretty-printed document collapsing into one long word.
            _ if in_tag || byte.is_ascii_whitespace() => {}
            _ => {
                *byte = rng.letter();
                changed += 1;
            }
        }
    }

    (out, changed)
}

/// SplitMix64, seeded by the caller.
///
/// A generator is wanted here for unpredictability of the *replacement*, not
/// for statistical quality, and this one is a few lines rather than a
/// dependency. It is not a cryptographic generator and the scrambling does not
/// need it to be: the replacement text is drawn independently of the text it
/// replaces, so there is nothing in the output to work backwards from.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// An uppercase ASCII letter, so the replacement is one byte per byte and
    /// always valid UTF-8 whatever the original encoded.
    fn letter(&mut self) -> u8 {
        b'A' + (self.next_u64() % 26) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_keeps_its_length_terminator_and_padding() {
        let data = b"Speed\0\0\0";
        let (out, changed) = scramble_text(data, &mut Rng::new(1));

        assert_eq!(
            out.len(),
            data.len(),
            "the data section may not change size"
        );
        assert_eq!(changed, 5);
        assert_eq!(&out[5..], b"\0\0\0", "terminator and padding are untouched");
        assert_ne!(&out[..5], b"Speed");
        assert!(out[..5].iter().all(|b| b.is_ascii_uppercase()));
    }

    #[test]
    fn a_multibyte_name_stays_the_same_number_of_bytes() {
        // "Temperatur°C" is 12 characters but 13 bytes: the degree sign
        // takes two. Replacement counts bytes, not characters.
        let data = "Temperatur°C\0".as_bytes();
        assert_eq!(data.len(), 14, "13 text bytes and a terminator");
        let (out, changed) = scramble_text(data, &mut Rng::new(2));

        assert_eq!(out.len(), data.len());
        assert_eq!(changed, 13, "the degree sign counts as its two bytes");
        assert!(
            std::str::from_utf8(&out[..13]).is_ok(),
            "replacing bytes with ASCII letters must leave valid UTF-8"
        );
    }

    #[test]
    fn empty_text_has_nothing_to_replace() {
        let (out, changed) = scramble_text(b"\0\0\0\0", &mut Rng::new(3));
        assert_eq!(changed, 0);
        assert_eq!(&out, b"\0\0\0\0");
    }

    #[test]
    fn markup_survives_the_scrambling_of_what_it_holds() {
        let data = b"<TXcomment><TX>Engine speed</TX></TXcomment>\0";
        let (out, changed) = scramble_markup(data, &mut Rng::new(4));
        let text = std::str::from_utf8(&out[..out.len() - 1]).unwrap();

        assert_eq!(out.len(), data.len());
        assert_eq!(changed, 11, "'Engine speed' less its space");
        assert!(text.starts_with("<TXcomment><TX>"), "tags are kept: {text}");
        assert!(text.ends_with("</TX></TXcomment>"), "tags are kept: {text}");
        assert!(!text.contains("Engine"), "the content is gone: {text}");
        assert!(text.contains(' '), "word spacing is kept: {text}");
    }

    #[test]
    fn the_same_seed_gives_the_same_scrambling() {
        let data = b"Coolant temperature\0";
        let (a, _) = scramble_text(data, &mut Rng::new(7));
        let (b, _) = scramble_text(data, &mut Rng::new(7));
        let (c, _) = scramble_text(data, &mut Rng::new(8));

        assert_eq!(a, b, "one seed, one result");
        assert_ne!(a, c, "a different seed is a different result");
    }
}
