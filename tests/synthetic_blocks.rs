//! End-to-end tests for blocks no available file contains.
//!
//! Attachments, events and the file-history chain are parsed and surfaced, but
//! the sample corpus has none of them — so until now nothing read one from an
//! actual file. A block parser passing its own unit tests only proves it agrees
//! with the fixture beside it; it does not prove the reader reaches the block,
//! follows its links, or hands back what it found.
//!
//! These tests build complete MF4 files containing those blocks and read them
//! through the public API, which closes that gap without needing a vendor file.

use falcon_mdf::Mf4File;

const HEADER: usize = 24;

/// Assembles an MF4 file from blocks, tracking where each lands.
///
/// Blocks reference one another by absolute file offset, so a builder has to
/// know an offset before the block that points at it can be written. Space is
/// reserved first, then filled in.
struct FileBuilder {
    bytes: Vec<u8>,
}

impl FileBuilder {
    /// Starts a file with a valid identification block and nothing else.
    fn new() -> Self {
        let mut bytes = vec![0u8; 64];
        bytes[0..8].copy_from_slice(b"MDF     ");
        bytes[8..16].copy_from_slice(b"4.11    ");
        bytes[16..24].copy_from_slice(b"falcon  ");
        bytes[28..30].copy_from_slice(&411u16.to_le_bytes());
        FileBuilder { bytes }
    }

    /// Returns the offset the next block will start at.
    fn next_offset(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// Appends a block and returns where it began.
    fn push(&mut self, block: &[u8]) -> u64 {
        let at = self.next_offset();
        self.bytes.extend_from_slice(block);
        at
    }

    /// Overwrites eight bytes, for filling in a link once its target is placed.
    fn patch_link(&mut self, at: u64, value: u64) {
        let at = at as usize;
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    /// Writes the file and opens it.
    fn open(&self, name: &str) -> falcon_mdf::Result<Mf4File> {
        let path = std::env::temp_dir().join(format!("falcon_mdf_synth_{name}.mf4"));
        std::fs::write(&path, &self.bytes).expect("write temp file");
        let result = Mf4File::open(&path);
        let _ = std::fs::remove_file(&path);
        result
    }
}

/// Builds a block: four-character id, links, then the data section.
fn block(id: &[u8; 4], links: &[u64], data: &[u8]) -> Vec<u8> {
    let total = HEADER + links.len() * 8 + data.len();
    let mut out = vec![0u8; HEADER];
    out[0..4].copy_from_slice(id);
    out[8..16].copy_from_slice(&(total as u64).to_le_bytes());
    out[16..24].copy_from_slice(&(links.len() as u64).to_le_bytes());
    for link in links {
        out.extend_from_slice(&link.to_le_bytes());
    }
    out.extend_from_slice(data);
    out
}

/// A text block holding `text`.
fn tx(text: &str) -> Vec<u8> {
    let mut data = text.as_bytes().to_vec();
    data.push(0);
    while data.len() % 8 != 0 {
        data.push(0);
    }
    block(b"##TX", &[], &data)
}

/// A metadata block holding `xml`.
fn md(xml: &str) -> Vec<u8> {
    let mut data = xml.as_bytes().to_vec();
    data.push(0);
    while data.len() % 8 != 0 {
        data.push(0);
    }
    block(b"##MD", &[], &data)
}

/// A minimal header block. Its links are patched once the targets are placed.
///
/// The data section is 32 bytes: start time, timezone and daylight offsets,
/// time flags and class, header flags, a reserved byte, and the start angle and
/// distance.
fn hd() -> Vec<u8> {
    let mut data = vec![0u8; 32];
    data[0..8].copy_from_slice(&1_600_000_000_000_000_000u64.to_le_bytes());
    block(b"##HD", &[0; 6], &data)
}

/// Offset of the header's `n`th link within the file.
///
/// The header's links are, in order: first data group, first file-history
/// entry, first channel-hierarchy node, first attachment, first event, and the
/// comment.
fn hd_link(n: usize) -> u64 {
    (64 + HEADER + n * 8) as u64
}

/// Link indices within the header block.
const HD_FH: usize = 1;
const HD_AT: usize = 3;
const HD_EV: usize = 4;

/// A file-history entry pointing at `comment`.
fn fh(next: u64, comment: u64, time_ns: u64) -> Vec<u8> {
    let mut data = vec![0u8; 16];
    data[0..8].copy_from_slice(&time_ns.to_le_bytes());
    block(b"##FH", &[next, comment], &data)
}

#[test]
fn an_embedded_attachment_is_found_and_its_bytes_read_back() {
    // The payload is what a caller ultimately wants; everything else is
    // bookkeeping that exists to locate it.
    let payload: Vec<u8> = (0u8..=255).collect();

    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("sensor-config.json"));
    let mime = f.push(&tx("application/json"));
    let fh_comment = f.push(&md(
        "<FHcomment><TX>created</TX><tool_id>test</tool_id></FHcomment>",
    ));
    let history = f.push(&fh(0, fh_comment, 1_600_000_000_000_000_000));

    // AT data: flags (embedded), creator, four reserved, 16-byte checksum, then
    // the original and embedded sizes, followed by the bytes themselves.
    let mut at_data = Vec::new();
    at_data.extend_from_slice(&1u16.to_le_bytes()); // embedded
    at_data.extend_from_slice(&0u16.to_le_bytes()); // creator index
    at_data.extend_from_slice(&0u32.to_le_bytes()); // reserved
    at_data.extend_from_slice(&[0u8; 16]); // checksum
    at_data.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    at_data.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    at_data.extend_from_slice(&payload);
    let attachment = f.push(&block(b"##AT", &[0, name, mime, 0], &at_data));

    f.patch_link(hd_link(HD_FH), history);
    f.patch_link(hd_link(HD_AT), attachment);

    let file = f.open("attachment").expect("synthetic file should open");

    let attachments = file.attachments();
    assert_eq!(attachments.len(), 1, "the attachment was not found");

    let at = &attachments[0];
    assert_eq!(at.file_name, "sensor-config.json");
    assert!(at.is_embedded);
    assert_eq!(at.original_size, payload.len() as u64);

    let read_back = file
        .attachment_data(at)
        .expect("reading embedded data should not fail")
        .expect("an embedded attachment should yield its bytes");
    assert_eq!(
        read_back, payload,
        "the bytes read back are not the bytes written"
    );
}

#[test]
fn an_external_attachment_reports_no_embedded_bytes() {
    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("recording.avi"));

    let mut at_data = Vec::new();
    at_data.extend_from_slice(&0u16.to_le_bytes()); // not embedded
    at_data.extend_from_slice(&0u16.to_le_bytes());
    at_data.extend_from_slice(&0u32.to_le_bytes());
    at_data.extend_from_slice(&[0u8; 16]);
    at_data.extend_from_slice(&4096u64.to_le_bytes()); // original size
    at_data.extend_from_slice(&0u64.to_le_bytes()); // nothing embedded
    let attachment = f.push(&block(b"##AT", &[0, name, 0, 0], &at_data));

    f.patch_link(hd_link(HD_AT), attachment);

    let file = f.open("attachment_external").expect("should open");
    let at = &file.attachments()[0];

    assert!(!at.is_embedded);
    assert_eq!(at.original_size, 4096);
    assert_eq!(
        file.attachment_data(at).unwrap(),
        None,
        "an external attachment has no bytes in this file"
    );
}

#[test]
fn a_chain_of_attachments_is_followed_to_the_end() {
    let mut f = FileBuilder::new();
    f.push(&hd());

    let names: Vec<u64> = ["first.bin", "second.bin", "third.bin"]
        .iter()
        .map(|n| f.push(&tx(n)))
        .collect();

    // Built back to front, so each block knows the offset of its successor.
    let mut next = 0u64;
    for name in names.iter().rev() {
        let mut data = vec![0u8; 40];
        data[24..32].copy_from_slice(&8u64.to_le_bytes());
        next = f.push(&block(b"##AT", &[next, *name, 0, 0], &data));
    }
    f.patch_link(hd_link(HD_AT), next);

    let file = f.open("attachment_chain").expect("should open");
    let found: Vec<&str> = file
        .attachments()
        .iter()
        .map(|a| a.file_name.as_str())
        .collect();

    assert_eq!(
        found,
        ["first.bin", "second.bin", "third.bin"],
        "the whole chain should be walked, in order"
    );
}

#[test]
fn events_are_read_with_their_position_and_comment() {
    let mut f = FileBuilder::new();
    f.push(&hd());

    let comment = f.push(&md("<EVcomment><TX>brake applied</TX></EVcomment>"));

    // EV data: type, sync type, range type, cause, flags, three reserved, a
    // scope count, two counts, then the base value and factor.
    let mut ev_data = Vec::new();
    ev_data.extend_from_slice(&[4u8, 0, 0, 4, 0, 0, 0, 0]);
    ev_data.extend_from_slice(&0u32.to_le_bytes()); // scope count
    ev_data.extend_from_slice(&0u16.to_le_bytes()); // attachments
    ev_data.extend_from_slice(&0u16.to_le_bytes()); // creator
    ev_data.extend_from_slice(&2_500_000_000i64.to_le_bytes());
    ev_data.extend_from_slice(&1e-9f64.to_le_bytes());
    let event = f.push(&block(b"##EV", &[0, 0, 0, comment], &ev_data));

    f.patch_link(hd_link(HD_EV), event);

    let file = f.open("event").expect("should open");
    let events = file.events();
    assert_eq!(events.len(), 1, "the event was not found");

    let ev = &events[0];
    assert_eq!(ev.comment, "brake applied");
    assert_eq!(ev.sync_base_value, 2_500_000_000);
    assert_eq!(ev.sync_factor, 1e-9);
    assert!(
        (ev.position() - 2.5).abs() < 1e-12,
        "position should be base times factor, got {}",
        ev.position()
    );
}

#[test]
fn the_file_history_chain_is_read_in_order() {
    let mut f = FileBuilder::new();
    f.push(&hd());

    let created = f.push(&md(
        "<FHcomment><TX>created</TX><tool_id>writer</tool_id></FHcomment>",
    ));
    let edited = f.push(&md(
        "<FHcomment><TX>edited</TX><tool_id>editor</tool_id></FHcomment>",
    ));

    let second = f.push(&fh(0, edited, 2_000_000_000_000_000_000));
    let first = f.push(&fh(second, created, 1_000_000_000_000_000_000));
    f.patch_link(hd_link(HD_FH), first);

    let file = f.open("history").expect("should open");
    let history = file.file_history();

    assert_eq!(history.len(), 2);
    assert_eq!(history[0].comment, "created");
    assert_eq!(history[0].tool_id(), Some("writer"));
    assert_eq!(history[0].time.timestamp_ns, 1_000_000_000_000_000_000);
    assert_eq!(history[1].comment, "edited");
    assert_eq!(history[1].tool_id(), Some("editor"));
}

#[test]
fn a_cycle_in_an_attachment_chain_is_rejected() {
    // The same guard that protects the data-group and channel chains has to
    // cover these too, or a crafted file loops forever here instead.
    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("looping.bin"));

    let mut data = vec![0u8; 40];
    data[24..32].copy_from_slice(&8u64.to_le_bytes());
    let at = f.push(&block(b"##AT", &[0, name, 0, 0], &data));

    // Point the attachment's next link at itself.
    f.patch_link(at + HEADER as u64, at);
    f.patch_link(hd_link(HD_AT), at);

    assert!(
        f.open("attachment_cycle").is_err(),
        "a self-referential attachment chain must be rejected"
    );
}
