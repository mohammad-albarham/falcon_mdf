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

use falcon_mdf::blocks::{EvSyncType, EventType};
use falcon_mdf::{Mf4Error, Mf4File, ReductionKind, SignalValues, UnreadableReason};

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
const HD_CH: usize = 2;
const HD_AT: usize = 3;
const HD_EV: usize = 4;

/// A file-history entry pointing at `comment`.
fn fh(next: u64, comment: u64, time_ns: u64) -> Vec<u8> {
    let mut data = vec![0u8; 16];
    data[0..8].copy_from_slice(&time_ns.to_le_bytes());
    block(b"##FH", &[next, comment], &data)
}

/// A channel-hierarchy node. Links are next sibling, first child, name,
/// comment, then three per element (data group, channel group, channel). The
/// data section is the element count, the hierarchy type, and three reserved
/// bytes.
#[allow(clippy::too_many_arguments)]
fn ch(
    next: u64,
    first_child: u64,
    name: u64,
    comment: u64,
    elements: &[(u64, u64, u64)],
    ch_type: u8,
) -> Vec<u8> {
    let mut links = vec![next, first_child, name, comment];
    for (dg, cg, cn) in elements {
        links.extend_from_slice(&[*dg, *cg, *cn]);
    }
    let mut data = vec![0u8; 8];
    data[0..4].copy_from_slice(&(elements.len() as u32).to_le_bytes());
    data[4] = ch_type;
    block(b"##CH", &links, &data)
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
fn a_compressed_attachment_is_decompressed_before_it_is_handed_back() {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    // Repetitive enough that the compressed form is genuinely shorter, so a
    // reader that skips decompression cannot accidentally return the payload.
    let payload: Vec<u8> = b"warning: coolant temperature high\n"
        .iter()
        .cycle()
        .take(4096)
        .copied()
        .collect();

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&payload).unwrap();
    let compressed = encoder.finish().unwrap();
    assert!(
        compressed.len() < payload.len(),
        "the fixture must actually compress, or it proves nothing"
    );

    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("diagnostics.log"));

    // Flags: embedded (bit 0), compressed (bit 1), checksum valid (bit 2).
    let mut at_data = Vec::new();
    at_data.extend_from_slice(&0x0007u16.to_le_bytes());
    at_data.extend_from_slice(&0u16.to_le_bytes()); // creator index
    at_data.extend_from_slice(&0u32.to_le_bytes()); // reserved
    at_data.extend_from_slice(&[0xABu8; 16]); // checksum
                                              // The original size is the payload's; the embedded size counts the
                                              // compressed bytes actually present in the file.
    at_data.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    at_data.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
    at_data.extend_from_slice(&compressed);
    let attachment = f.push(&block(b"##AT", &[0, name, 0, 0], &at_data));

    f.patch_link(hd_link(HD_AT), attachment);

    let file = f.open("attachment_compressed").expect("should open");
    let at = &file.attachments()[0];

    assert!(at.is_embedded);
    assert!(at.is_compressed, "bit 1 says the bytes are compressed");
    assert!(at.md5_valid, "bit 2 says the checksum means something");
    assert_eq!(
        at.original_size,
        payload.len() as u64,
        "original_size is the size after decompression"
    );

    let read_back = file
        .attachment_data(at)
        .expect("reading should not fail")
        .expect("an embedded attachment should yield its bytes");
    assert_eq!(
        read_back, payload,
        "a compressed attachment must come back decompressed, not as a \
         deflate stream the caller has to recognise"
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
    // ev_type 6 is a marker and ev_sync_type 1 is seconds. Both used to be
    // written here as whatever the parser happened to decode, which is how the
    // numbering stayed wrong: a marker read back as `Unknown(6)`, and a
    // time-domain event as `Angle`.
    let mut ev_data = Vec::new();
    ev_data.extend_from_slice(&[6u8, 1, 0, 4, 0, 0, 0, 0]);
    ev_data.extend_from_slice(&0u32.to_le_bytes()); // scope count
    ev_data.extend_from_slice(&0u16.to_le_bytes()); // attachments
    ev_data.extend_from_slice(&0u16.to_le_bytes()); // creator
    ev_data.extend_from_slice(&2_500_000_000i64.to_le_bytes());
    ev_data.extend_from_slice(&1e-9f64.to_le_bytes());
    // Five links: next, parent, range start, name, comment. This fixture used
    // four, matching a parser that also used four — so both agreed on a layout
    // the standard does not have, and the name was read as the comment.
    let name = f.push(&tx("Brake"));
    let event = f.push(&block(b"##EV", &[0, 0, 0, name, comment], &ev_data));

    f.patch_link(hd_link(HD_EV), event);

    let file = f.open("event").expect("should open");
    let events = file.events();
    assert_eq!(events.len(), 1, "the event was not found");

    let ev = &events[0];
    assert_eq!(
        ev.name, "Brake",
        "the name link is index 3, not the comment"
    );
    assert_eq!(ev.comment, "brake applied");
    assert_eq!(
        ev.event_type,
        EventType::Marker,
        "a marker is ev_type 6, and must not fall through to Unknown"
    );
    assert_eq!(
        ev.sync_type,
        EvSyncType::Time,
        "ev_sync_type 1 is seconds; anything else hides time events from \
         every consumer that filters on the time domain"
    );
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

/// A data group block. Links: next, first channel group, data, comment.
fn dg(next: u64, cg_first: u64, data: u64, rec_id_size: u8) -> Vec<u8> {
    let mut d = vec![0u8; 8];
    d[0] = rec_id_size;
    block(b"##DG", &[next, cg_first, data, 0], &d)
}

/// A channel group block. Links: next, first channel, acquisition name,
/// acquisition source, first sample reduction, comment.
fn cg(cn_first: u64, cycle_count: u64, data_bytes: u32) -> Vec<u8> {
    let mut d = vec![0u8; 32];
    d[8..16].copy_from_slice(&cycle_count.to_le_bytes());
    d[24..28].copy_from_slice(&data_bytes.to_le_bytes());
    block(b"##CG", &[0, cn_first, 0, 0, 0, 0], &d)
}

/// A channel block. Links: next, composition, name, source, conversion, data,
/// unit, comment.
#[allow(clippy::too_many_arguments)]
fn cn(
    next: u64,
    composition: u64,
    name: u64,
    channel_type: u8,
    data_type: u8,
    byte_offset: u32,
    bit_count: u32,
) -> Vec<u8> {
    let mut d = vec![0u8; 72];
    d[0] = channel_type;
    d[2] = data_type;
    d[4..8].copy_from_slice(&byte_offset.to_le_bytes());
    d[8..12].copy_from_slice(&bit_count.to_le_bytes());
    block(b"##CN", &[next, composition, name, 0, 0, 0, 0, 0], &d)
}

/// A channel array block describing a one-dimensional array of `len` elements.
fn ca(template_cn: u64, len: u64, element_bytes: i32) -> Vec<u8> {
    let mut d = Vec::new();
    d.push(0u8); // ca_type = Array
    d.push(0u8); // ca_storage = CN template: elements adjacent in the record
    d.extend_from_slice(&1u16.to_le_bytes()); // one dimension
    d.extend_from_slice(&0u32.to_le_bytes()); // flags
    d.extend_from_slice(&element_bytes.to_le_bytes()); // byte offset base
    d.extend_from_slice(&0u32.to_le_bytes()); // invalidation bit base
    d.extend_from_slice(&len.to_le_bytes()); // ca_dim_size[0]

    // With no flags set the link list is the composition link and nothing
    // else: every other section of it is introduced by a flag. The trailing
    // zero link this fixture used to carry was one the standard never places.
    block(b"##CA", &[template_cn], &d)
}

/// A data block holding raw records.
fn dt(records: &[u8]) -> Vec<u8> {
    block(b"##DT", &[], records)
}

#[test]
fn an_array_channel_decodes_to_its_elements() {
    // Three f64 elements per sample, two samples — the shape a vector-valued
    // signal such as a three-axis accelerometer takes.
    let samples: [[f64; 3]; 2] = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]];
    let mut records = Vec::new();
    for s in &samples {
        for v in s {
            records.extend_from_slice(&v.to_le_bytes());
        }
    }

    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Acceleration"));
    let template = f.push(&cn(0, 0, 0, 0, 4, 0, 64)); // one f64 element
    let array = f.push(&ca(template, 3, 8));
    let channel = f.push(&cn(0, array, name, 0, 4, 0, 64));
    let group = f.push(&cg(channel, samples.len() as u64, 24));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));

    f.patch_link(hd_link(0), group_block);

    let file = f.open("array").expect("synthetic file should open");
    let ch = file
        .find_channel("Acceleration")
        .expect("the array channel should be listed");

    assert_eq!(
        ch.array_shape(),
        Some(&[3u64][..]),
        "the array's shape should come from the CA block"
    );
    assert!(
        ch.unreadable().is_none(),
        "an array channel with a template should be readable"
    );

    let values = file
        .signal(ch)
        .expect("signal")
        .values_f64()
        .expect("an array channel should decode");

    assert_eq!(
        values,
        vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        "elements should come back flattened, sample by sample"
    );
}

#[test]
fn an_array_shape_claiming_more_elements_than_the_record_holds_is_rejected() {
    // Same shape as `an_array_channel_decodes_to_its_elements`, but the CA
    // block declares an astronomical element count — what one flipped byte in
    // a file's `ca_dim_size` produces in the wild (B35: byte-flip sweep over
    // `dSPACE_MeasurementArrays.mf4`, offset 1331). The record is 24 bytes;
    // the declared shape asks for ten billion 8-byte elements, which must be
    // rejected before anything is allocated on the strength of it, not
    // silently truncated to what fits.
    let samples: [[f64; 3]; 2] = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]];
    let mut records = Vec::new();
    for s in &samples {
        for v in s {
            records.extend_from_slice(&v.to_le_bytes());
        }
    }

    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Acceleration"));
    let template = f.push(&cn(0, 0, 0, 0, 4, 0, 64)); // one f64 element
    let array = f.push(&ca(template, 10_000_000_000, 8));
    let channel = f.push(&cn(0, array, name, 0, 4, 0, 64));
    let group = f.push(&cg(channel, samples.len() as u64, 24));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));

    f.patch_link(hd_link(0), group_block);

    let file = f
        .open("array_huge_shape")
        .expect("synthetic file should open");
    let ch = file
        .find_channel("Acceleration")
        .expect("the array channel should still be listed");

    let err = file
        .signal(ch)
        .expect("signal")
        .values_f64()
        .expect_err("a shape bigger than the record can hold must be rejected, not allocated");
    let message = err.to_string();
    assert!(
        message.contains("Acceleration"),
        "the error should name the channel: {message}"
    );
}

#[test]
fn a_look_up_array_composed_with_another_ca_block_combines_their_shapes() {
    // B30: a look-up array whose `ca_composition` names another CA block
    // rather than a template CN — an array whose elements are themselves
    // arrays. Two 1-D levels here, both with `ca_composition` chasing to the
    // next, the inner one ending at 0 (elements typed by the parent channel,
    // a plain u16 — B31's rule, applied at the end of the chain rather than
    // to a single CA).
    //
    // Outer dims [2], byte_offset_base 6 (one inner row's worth of bytes: 3
    // elements * 2 bytes). Inner dims [3], byte_offset_base 2 (one u16).
    // Combined shape [2, 3]: element (i, j) sits at byte i*6 + j*2, which for
    // i in 0..2, j in 0..3 is exactly bytes 0..12 with no gaps — so the
    // record's 6 u16 values, read in file order, are already the expected
    // row-major output: [10, 11, 12, 20, 21, 22].
    let stored: [u16; 6] = [10, 11, 12, 20, 21, 22];
    let mut record = Vec::new();
    for v in stored {
        record.extend_from_slice(&v.to_le_bytes());
    }

    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("Nested"));
    let inner = f.push(&ca(0, 3, 2)); // dims=[3], stride 2, composition 0
    let outer = f.push(&ca(inner, 2, 6)); // dims=[2], stride 6, composition = inner
    let channel = f.push(&cn(0, outer, name, 0, 0, 0, 16)); // u16 parent
    let group = f.push(&cg(channel, 1, 12));
    let data = f.push(&dt(&record));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f
        .open("array_ca_chain")
        .expect("synthetic file should open");
    let ch = file.find_channel("Nested").expect("channel");
    assert_eq!(
        ch.array_shape(),
        Some(&[2u64, 3u64][..]),
        "the combined shape is the outer level's dims followed by the inner's"
    );
    assert!(ch.unreadable().is_none());

    let values = file.signal(ch).expect("signal").values().expect("decode");
    assert_eq!(
        values,
        SignalValues::Array {
            values: vec![10.0, 11.0, 12.0, 20.0, 21.0, 22.0],
            elements_per_sample: 6,
        }
    );
}

#[test]
fn a_composed_ca_chain_too_large_for_its_record_is_rejected() {
    // The same B35 structural bound as a single-level array, generalized: the
    // combined shape's furthest element must still fit inside the record, or
    // one flipped byte in either level's dimension count would be read as
    // real elements past the channel's own bytes.
    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("Nested"));
    let inner = f.push(&ca(0, 3, 2));
    // 10 billion "columns" of 6 bytes each: far more than any record holds.
    let outer = f.push(&ca(inner, 10_000_000_000, 6));
    let channel = f.push(&cn(0, outer, name, 0, 0, 0, 16));
    let group = f.push(&cg(channel, 1, 12));
    let data = f.push(&dt(&[0u8; 12]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f
        .open("array_ca_chain_huge")
        .expect("synthetic file should open");
    let ch = file.find_channel("Nested").expect("channel");

    // The combined element count alone (2 * 10 billion = 20 billion) already
    // exceeds what `Limits::max_alloc` worth of f64s could hold, so the
    // channel is refused at parse time rather than becoming a readable
    // channel whose *read* later fails.
    assert!(
        ch.unreadable().is_some(),
        "a chain whose combined count cannot fit max_alloc must be refused up front"
    );
    assert!(file.signal(ch).expect("signal").values().is_err());
}

#[test]
fn a_dynamic_size_array_with_more_than_one_dynamic_dimension_stays_unreadable() {
    // `ArrayElement` has one flat stride for the whole array; combining that
    // with more than one per-sample count has no representation this build
    // can decode honestly, so it is refused rather than guessed at.
    let mut d = Vec::new();
    d.push(0u8); // ca_type = Array
    d.push(0u8); // ca_storage = CN template
    d.extend_from_slice(&2u16.to_le_bytes()); // two dimensions
    d.extend_from_slice(&1u32.to_le_bytes()); // flags: dynamic size
    d.extend_from_slice(&8i32.to_le_bytes());
    d.extend_from_slice(&0u32.to_le_bytes());
    d.extend_from_slice(&2u64.to_le_bytes()); // dim 0 max
    d.extend_from_slice(&3u64.to_le_bytes()); // dim 1 max
    let ca_two_dynamic = |template_cn: u64| block(b"##CA", &[template_cn, 0, 0, 0, 0, 0, 0], &d);

    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("Grid"));
    let template = f.push(&cn(0, 0, 0, 0, 4, 0, 64));
    let array = f.push(&ca_two_dynamic(template));
    let channel = f.push(&cn(0, array, name, 0, 4, 0, 64));
    let group = f.push(&cg(channel, 1, 48));
    let data = f.push(&dt(&[0u8; 48]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f
        .open("array_dynamic_2d")
        .expect("synthetic file should open");
    let ch = file.find_channel("Grid").expect("channel");

    assert_eq!(
        ch.unreadable(),
        Some(falcon_mdf::UnreadableReason::ArrayDynamicSize),
        "more than one dynamic dimension is refused, not read at its maxima"
    );
    assert!(file.signal(ch).expect("signal").values().is_err());
}

#[test]
fn an_array_without_a_template_decodes_from_the_parent_channels_type() {
    // This test used to assert the opposite — that such an array is unreadable,
    // "because without a template nothing says how wide an element is". That
    // reasoning was wrong twice over: the parent channel's own data type and
    // bit count describe the element, and `ca_byte_offset_base` gives the
    // stride. Vector and dSPACE both emit look-up tables this way, and refusing
    // them cost 15 channels across four of their reference files.
    let mut records = Vec::new();
    for v in [1.5f64, 2.5, 3.5, 4.5] {
        records.extend_from_slice(&v.to_le_bytes());
    }

    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Mystery"));
    let array = f.push(&ca_no_template(&[4], 8, 0));
    let channel = f.push(&cn(0, array, name, 0, 4, 0, 64));
    let group = f.push(&cg(channel, 1, 32));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("array_parent_template").expect("should open");
    let ch = file
        .find_channel("Mystery")
        .expect("channel should be listed");

    assert!(
        ch.unreadable().is_none(),
        "the parent describes the element"
    );
    assert_eq!(
        file.signal(ch).expect("signal").values().expect("decode"),
        SignalValues::Array {
            values: vec![1.5, 2.5, 3.5, 4.5],
            elements_per_sample: 4,
        }
    );
}

/// A sample-reduction block. Two links: next level, and the reduction data.
fn sr(next: u64, data: u64, cycle_count: u64, interval: f64, sync_type: u8) -> Vec<u8> {
    let mut d = vec![0u8; 24];
    d[0..8].copy_from_slice(&cycle_count.to_le_bytes());
    d[8..16].copy_from_slice(&interval.to_le_bytes());
    d[16] = sync_type;
    block(b"##SR", &[next, data], &d)
}

/// A channel group with a sample-reduction chain.
fn cg_with_reductions(cn_first: u64, sr_first: u64, cycle_count: u64, data_bytes: u32) -> Vec<u8> {
    let mut d = vec![0u8; 32];
    d[8..16].copy_from_slice(&cycle_count.to_le_bytes());
    d[24..28].copy_from_slice(&data_bytes.to_le_bytes());
    block(b"##CG", &[0, cn_first, 0, 0, sr_first, 0], &d)
}

#[test]
fn sample_reduction_levels_are_listed_with_their_parameters() {
    // A group may carry several reductions, each condensing a longer interval
    // than the last. The descriptors are readable; the reduced values are not,
    // and that distinction is the point of this test.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Speed"));
    let channel = f.push(&cn(0, 0, name, 0, 4, 0, 64));

    // Built back to front so each level knows its successor's offset.
    let coarse = f.push(&sr(0, 0, 10, 1.0, 0));
    let fine = f.push(&sr(coarse, 0, 100, 0.1, 0));

    let group = f.push(&cg_with_reductions(channel, fine, 1000, 8));
    let data = f.push(&dt(&[0u8; 8000]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("sample_reduction").expect("should open");
    let cg = &file.data_groups()[0].channel_groups[0];
    let levels = cg.sample_reductions();

    assert_eq!(levels.len(), 2, "both reduction levels should be found");

    assert_eq!(levels[0].cycle_count, 100);
    assert_eq!(levels[0].interval, 0.1);
    assert_eq!(levels[1].cycle_count, 10);
    assert_eq!(levels[1].interval, 1.0);

    // The group's own data is unaffected by the presence of reductions.
    assert_eq!(cg.sample_count, 1000);
}

#[test]
fn a_group_without_reductions_reports_none() {
    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("Speed"));
    let channel = f.push(&cn(0, 0, name, 0, 4, 0, 64));
    let group = f.push(&cg(channel, 2, 8));
    let data = f.push(&dt(&[0u8; 16]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("no_reduction").expect("should open");
    assert!(file.data_groups()[0].channel_groups[0]
        .sample_reductions()
        .is_empty());
}

#[test]
fn a_cycle_in_a_reduction_chain_is_rejected() {
    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("Speed"));
    let channel = f.push(&cn(0, 0, name, 0, 4, 0, 64));

    let level = f.push(&sr(0, 0, 10, 1.0, 0));
    // Point the level's next link at itself.
    f.patch_link(level + HEADER as u64, level);

    let group = f.push(&cg_with_reductions(channel, level, 10, 8));
    let data = f.push(&dt(&[0u8; 80]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    assert!(
        f.open("reduction_cycle").is_err(),
        "a self-referential reduction chain must be rejected"
    );
}

#[test]
fn reduced_values_are_read_from_the_right_third_of_each_record() {
    // A reduced record holds three copies of the group's normal record: the
    // means, then the minima, then the maxima. Reading the wrong third returns
    // real numbers from the wrong series, which is worse than failing.
    let means = [10.0f64, 20.0];
    let mins = [1.0f64, 2.0];
    let maxes = [100.0f64, 200.0];

    let mut reduced = Vec::new();
    for i in 0..2 {
        reduced.extend_from_slice(&means[i].to_le_bytes());
        reduced.extend_from_slice(&mins[i].to_le_bytes());
        reduced.extend_from_slice(&maxes[i].to_le_bytes());
    }

    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Speed"));
    let channel = f.push(&cn(0, 0, name, 0, 4, 0, 64));
    let reduction_data = f.push(&block(b"##RD", &[], &reduced));
    let level = f.push(&sr(0, reduction_data, 2, 1.0, 0));
    let group = f.push(&cg_with_reductions(channel, level, 100, 8));
    let data = f.push(&dt(&[0u8; 800]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("reduced_values").expect("should open");
    let ch = file.find_channel("Speed").expect("channel");
    let cg = &file.data_groups()[0].channel_groups[0];
    let level = &cg.sample_reductions()[0];

    let read = |kind| {
        file.reduced_signal(ch, level, kind)
            .expect("reduced signal")
            .values_f64()
            .expect("values")
    };

    assert_eq!(read(ReductionKind::Mean), vec![10.0, 20.0]);
    assert_eq!(read(ReductionKind::Min), vec![1.0, 2.0]);
    assert_eq!(read(ReductionKind::Max), vec![100.0, 200.0]);
}

#[test]
fn a_reduction_naming_no_data_fails_rather_than_returning_nothing() {
    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("Speed"));
    let channel = f.push(&cn(0, 0, name, 0, 4, 0, 64));
    let level = f.push(&sr(0, 0, 4, 1.0, 0)); // no data link
    let group = f.push(&cg_with_reductions(channel, level, 10, 8));
    let data = f.push(&dt(&[0u8; 80]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("reduction_no_data").expect("should open");
    let ch = file.find_channel("Speed").unwrap();
    let level = &file.data_groups()[0].channel_groups[0].sample_reductions()[0];

    assert!(
        file.reduced_signal(ch, level, ReductionKind::Mean).is_err(),
        "a reduction with no data block must fail, not report zero samples"
    );
}

/// A channel block carrying a conversion link, which `cn` leaves empty.
#[allow(clippy::too_many_arguments)]
fn cn_converted(
    next: u64,
    name: u64,
    conversion: u64,
    channel_type: u8,
    data_type: u8,
    byte_offset: u32,
    bit_count: u32,
) -> Vec<u8> {
    let mut d = vec![0u8; 72];
    d[0] = channel_type;
    d[2] = data_type;
    d[4..8].copy_from_slice(&byte_offset.to_le_bytes());
    d[8..12].copy_from_slice(&bit_count.to_le_bytes());
    block(b"##CN", &[next, 0, name, 0, conversion, 0, 0, 0], &d)
}

/// A conversion block. Links after the four fixed ones are `cc_ref`; `values`
/// are the `cc_val` parameters.
fn cc(conversion_type: u8, references: &[u64], values: &[f64]) -> Vec<u8> {
    let mut d = Vec::new();
    d.push(conversion_type);
    d.push(0); // precision
    d.extend_from_slice(&0u16.to_le_bytes()); // flags
    d.extend_from_slice(&(references.len() as u16).to_le_bytes()); // cc_ref_count
    d.extend_from_slice(&(values.len() as u16).to_le_bytes()); // cc_val_count
    d.extend_from_slice(&0f64.to_le_bytes()); // min physical
    d.extend_from_slice(&0f64.to_le_bytes()); // max physical
    for v in values {
        d.extend_from_slice(&v.to_le_bytes());
    }

    let mut links = vec![0u64; 4];
    links.extend_from_slice(references);
    block(b"##CC", &links, &d)
}

/// Packs each string into a fixed-width, NUL-padded record field.
fn text_records(samples: &[&str], width: usize) -> Vec<u8> {
    let mut out = vec![0u8; samples.len() * width];
    for (slot, s) in out.chunks_exact_mut(width).zip(samples) {
        let bytes = s.as_bytes();
        slot[..bytes.len()].copy_from_slice(bytes);
    }
    out
}

#[test]
fn a_text_to_value_conversion_maps_each_string_to_its_number() {
    // MF4 type 9. Every cc_ref is a key; the default is the *last cc_val*,
    // there being no default link — which is what distinguishes its layout
    // from the value-to-text table of type 7.
    let width = 8;
    let records = text_records(&["off", "on", "off", "unknown"], width);

    let mut f = FileBuilder::new();
    f.push(&hd());

    let key_off = f.push(&tx("off"));
    let key_on = f.push(&tx("on"));
    let conv = f.push(&cc(9, &[key_off, key_on], &[0.0, 1.0, -1.0]));

    let name = f.push(&tx("Ignition"));
    let channel = f.push(&cn_converted(0, name, conv, 0, 6, 0, (width * 8) as u32));
    let group = f.push(&cg(channel, 4, width as u32));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("ttab").expect("synthetic file should open");
    let ch = file
        .find_channel("Ignition")
        .expect("channel should be listed");

    let values = file
        .signal(ch)
        .expect("signal")
        .values_f64()
        .expect("a text-to-value channel should decode");

    // The fourth sample matches no key and takes the trailing default.
    assert_eq!(values, vec![0.0, 1.0, 0.0, -1.0]);
}

#[test]
fn a_text_to_value_table_without_a_default_yields_nan_for_an_unknown_key() {
    let width = 8;
    let records = text_records(&["on", "missing"], width);

    let mut f = FileBuilder::new();
    f.push(&hd());

    let key_on = f.push(&tx("on"));
    let conv = f.push(&cc(9, &[key_on], &[1.0]));

    let name = f.push(&tx("Ignition"));
    let channel = f.push(&cn_converted(0, name, conv, 0, 6, 0, (width * 8) as u32));
    let group = f.push(&cg(channel, 2, width as u32));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f
        .open("ttab_nodefault")
        .expect("synthetic file should open");
    let ch = file
        .find_channel("Ignition")
        .expect("channel should be listed");
    let values = file
        .signal(ch)
        .expect("signal")
        .values_f64()
        .expect("decode");

    assert_eq!(values[0], 1.0);
    assert!(
        values[1].is_nan(),
        "an unmatched key with no default has no value, not zero: {}",
        values[1]
    );
}

#[test]
fn a_text_to_text_conversion_translates_each_string() {
    // MF4 type 10. References alternate key, replacement, key, replacement,
    // with one default at the end — so the count is always odd.
    let width = 8;
    let records = text_records(&["ok", "err", "other"], width);

    let mut f = FileBuilder::new();
    f.push(&hd());

    let in_ok = f.push(&tx("ok"));
    let out_ok = f.push(&tx("Healthy"));
    let in_err = f.push(&tx("err"));
    let out_err = f.push(&tx("Faulted"));
    let default = f.push(&tx("Unrecognised"));
    let conv = f.push(&cc(10, &[in_ok, out_ok, in_err, out_err, default], &[]));

    let name = f.push(&tx("Status"));
    let channel = f.push(&cn_converted(0, name, conv, 0, 6, 0, (width * 8) as u32));
    let group = f.push(&cg(channel, 3, width as u32));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("trans").expect("synthetic file should open");
    let ch = file
        .find_channel("Status")
        .expect("channel should be listed");

    assert_eq!(
        ch.value_kind(),
        falcon_mdf::ValueKind::Str,
        "a text-to-text conversion still produces text"
    );

    match file.signal(ch).expect("signal").values().expect("decode") {
        falcon_mdf::SignalValues::Str(v) => {
            assert_eq!(v, vec!["Healthy", "Faulted", "Unrecognised"]);
        }
        other => panic!("expected strings, got {}", other.kind()),
    }
}

#[test]
fn a_text_to_text_table_with_an_even_reference_count_is_rejected() {
    // Pairs plus one default is always odd. An even count means a replacement
    // or the default is missing, and guessing which would silently mistranslate
    // every sample after the gap.
    let width = 8;
    let mut f = FileBuilder::new();
    f.push(&hd());

    let in_ok = f.push(&tx("ok"));
    let out_ok = f.push(&tx("Healthy"));
    let conv = f.push(&cc(10, &[in_ok, out_ok], &[]));

    let name = f.push(&tx("Status"));
    let channel = f.push(&cn_converted(0, name, conv, 0, 6, 0, (width * 8) as u32));
    let group = f.push(&cg(channel, 1, width as u32));
    let data = f.push(&dt(&text_records(&["ok"], width)));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("trans_even").expect("synthetic file should open");
    let ch = file
        .find_channel("Status")
        .expect("channel should be listed");
    assert!(
        file.signal(ch).expect("signal").values().is_err(),
        "a malformed text-to-text table must fail rather than translate half of it"
    );
}

#[test]
fn a_range_to_text_table_closes_both_bounds_and_takes_the_last_match() {
    // MF4 type 8. `cc_val` holds a (lower, upper) pair per entry, `cc_ref` one
    // text per pair plus a trailing default.
    //
    // The shape is taken from two vendor files this cannot read from, because
    // they are not redistributed. `ASAP2_Demo_V171.mf4` declares six ranges of
    // the form [100,100] — a single value each — which an exclusive upper bound
    // can never match, leaving six labels its author wrote unreachable. And
    // `Vector_ValueRange2TextConversion.mf4` declares abutting ranges, where a
    // value on the shared bound belongs to two of them and the file means the
    // later. Both facts are pinned here so a fresh clone still holds the rule.
    let records = [0u8, 5, 10, 20, 100, 21];

    let mut f = FileBuilder::new();
    f.push(&hd());

    let low = f.push(&tx("low"));
    let high = f.push(&tx("high"));
    let hundred = f.push(&tx("hundred"));
    let default = f.push(&tx("out of range"));
    let conv = f.push(&cc(
        8,
        &[low, high, hundred, default],
        &[0.0, 10.0, 10.0, 20.0, 100.0, 100.0],
    ));

    let name = f.push(&tx("Level"));
    let channel = f.push(&cn_converted(0, name, conv, 0, 0, 0, 8));
    let group = f.push(&cg(channel, records.len() as u64, 1));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("rangetab").expect("synthetic file should open");
    let ch = file
        .find_channel("Level")
        .expect("channel should be listed");

    let v = match file.signal(ch).expect("signal").values().expect("decode") {
        falcon_mdf::SignalValues::Str(v) => v,
        other => panic!("expected strings, got {}", other.kind()),
    };

    assert_eq!(v[0], "low", "the lower bound is inside its range");
    assert_eq!(v[1], "low");
    assert_eq!(
        v[2], "high",
        "10 sits in both [0,10] and [10,20]; the later range is the answer"
    );
    assert_eq!(v[3], "high", "20 is its range's upper bound, and inside it");
    assert_eq!(
        v[4], "hundred",
        "a single-value range [100,100] is unreachable unless both bounds close"
    );
    assert_eq!(v[5], "out of range", "outside every range");
}

/// A bitfield conversion block. Unlike every other conversion type, its
/// `cc_val` parameters are unsigned integers rather than doubles.
fn cc_bitfield(references: &[u64], masks: &[u64]) -> Vec<u8> {
    let mut d = Vec::new();
    d.push(11u8);
    d.push(0); // precision
    d.extend_from_slice(&0u16.to_le_bytes()); // flags
    d.extend_from_slice(&(references.len() as u16).to_le_bytes());
    d.extend_from_slice(&(masks.len() as u16).to_le_bytes());
    d.extend_from_slice(&0f64.to_le_bytes());
    d.extend_from_slice(&0f64.to_le_bytes());
    for m in masks {
        d.extend_from_slice(&m.to_le_bytes());
    }

    let mut links = vec![0u64; 4];
    links.extend_from_slice(references);
    block(b"##CC", &links, &d)
}

/// A conversion block carrying a name, which `cc` leaves empty.
fn cc_named(conversion_type: u8, name: u64, references: &[u64], values: &[f64]) -> Vec<u8> {
    let mut b = cc(conversion_type, references, values);
    // The name is the first of the four fixed links, immediately after the
    // 24-byte block header.
    b[24..32].copy_from_slice(&name.to_le_bytes());
    b
}

#[test]
fn a_bitfield_conversion_renders_each_field_of_a_status_word() {
    // MF4 type 11: each entry masks the raw value and renders the result, and
    // a nested table's name labels its part. A gearbox status word packing the
    // gear in the low nibble and a clutch flag above it is the shape this
    // conversion exists for.
    let samples: [u16; 3] = [0x0011, 0x0002, 0x0005];
    let mut records = Vec::new();
    for v in &samples {
        records.extend_from_slice(&v.to_le_bytes());
    }

    let mut f = FileBuilder::new();
    f.push(&hd());

    // Nested value-to-text table, keyed by the *masked* value.
    let first = f.push(&tx("first"));
    let second = f.push(&tx("second"));
    let unknown = f.push(&tx("unknown"));
    let gear_name = f.push(&tx("gear"));
    let gear_table = f.push(&cc_named(
        7,
        gear_name,
        &[first, second, unknown],
        &[1.0, 2.0],
    ));

    // A bare text reference is a flag's label.
    let clutch = f.push(&tx("clutch"));

    let conv = f.push(&cc_bitfield(&[gear_table, clutch], &[0x000F, 0x0010]));

    let name = f.push(&tx("GearboxStatus"));
    let channel = f.push(&cn_converted(0, name, conv, 0, 0, 0, 16));
    let group = f.push(&cg(channel, samples.len() as u64, 2));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("bitfield").expect("synthetic file should open");
    let ch = file
        .find_channel("GearboxStatus")
        .expect("channel should be listed");

    match file.signal(ch).expect("signal").values().expect("decode") {
        falcon_mdf::SignalValues::Str(v) => {
            assert_eq!(
                v,
                vec![
                    // gear 1 and the clutch bit set
                    "gear = first | clutch",
                    // gear 2, clutch clear: the flag contributes nothing
                    "gear = second",
                    // gear 5 matches no key, so the nested table's default applies
                    "gear = unknown",
                ]
            );
        }
        other => panic!("expected strings, got {}", other.kind()),
    }
}

#[test]
fn a_bitfield_referencing_itself_is_rejected_rather_than_recursed() {
    // A `cc_ref` pointing back at its own block would recurse until the stack
    // ran out. The depth limit turns that into an error.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let conv = f.next_offset();
    f.push(&cc_bitfield(&[conv], &[0xFF]));

    let name = f.push(&tx("Status"));
    let channel = f.push(&cn_converted(0, name, conv, 0, 0, 0, 16));
    let group = f.push(&cg(channel, 1, 2));
    let data = f.push(&dt(&[0u8; 2]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    // Either the file refuses to open or the channel refuses to decode; what
    // matters is that neither hangs nor overflows the stack.
    if let Ok(file) = f.open("bitfield_cycle") {
        if let Some(ch) = file.find_channel("Status") {
            let _ = file.signal(ch).and_then(|s| s.values());
        }
    }
}

/// A channel array block using a storage form whose elements live outside the
/// record — one channel group per element.
fn ca_cg_template(template_cn: u64, len: u64, element_bytes: i32) -> Vec<u8> {
    let mut d = Vec::new();
    d.push(0u8); // ca_type = Array
    d.push(1u8); // ca_storage = CG template
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&0u32.to_le_bytes());
    d.extend_from_slice(&element_bytes.to_le_bytes());
    d.extend_from_slice(&0u32.to_le_bytes());
    d.extend_from_slice(&len.to_le_bytes());
    block(b"##CA", &[template_cn], &d)
}

#[test]
fn an_array_stored_one_group_per_element_stays_unreadable() {
    // Only the CN-template form keeps a sample's elements adjacent in the
    // record. Decoding a CG-template array with the same striding would return
    // whatever bytes happen to follow the channel — plausible-looking numbers
    // that are not the array.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Acceleration"));
    let template = f.push(&cn(0, 0, 0, 0, 4, 0, 64));
    let array = f.push(&ca_cg_template(template, 3, 8));
    let channel = f.push(&cn(0, array, name, 0, 4, 0, 64));
    let group = f.push(&cg(channel, 2, 24));
    let data = f.push(&dt(&[0u8; 48]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("array_cg").expect("synthetic file should open");
    let ch = file
        .find_channel("Acceleration")
        .expect("the array channel should still be listed");

    assert_eq!(
        ch.unreadable(),
        Some(falcon_mdf::UnreadableReason::ArrayGroupTemplate),
        "a caller should learn which shape was refused, not just that one was"
    );
    assert!(file.signal(ch).expect("signal").values().is_err());
}

/// A CN-template array whose dimension sizes vary per sample.
///
/// The dynamic-size flag is bit 0 of `ca_flags`, and it introduces three links
/// per dimension — the data group, channel group and channel giving each
/// sample's actual size. Both facts come from the standard: this reader used to
/// read bit 0 as "has axis" and so could not see the flag at all.
fn ca_dynamic_size(template_cn: u64, max_len: u64, element_bytes: i32) -> Vec<u8> {
    let mut d = Vec::new();
    d.push(0u8); // ca_type = Array
    d.push(0u8); // ca_storage = CN template
    d.extend_from_slice(&1u16.to_le_bytes()); // one dimension
    d.extend_from_slice(&1u32.to_le_bytes()); // flags: dynamic size
    d.extend_from_slice(&element_bytes.to_le_bytes());
    d.extend_from_slice(&0u32.to_le_bytes());
    d.extend_from_slice(&max_len.to_le_bytes()); // the maximum, not the shape
    block(b"##CA", &[template_cn, 0, 0, 0], &d)
}

/// A CA block naming no element template, with `ca_byte_offset_base` as the
/// stride. `flags` selects inverse layout when set.
fn ca_no_template(dims: &[u64], element_bytes: i32, flags: u32) -> Vec<u8> {
    let mut d = Vec::new();
    d.push(0u8); // ca_type = Array
    d.push(0u8); // ca_storage = CN template
    d.extend_from_slice(&(dims.len() as u16).to_le_bytes());
    d.extend_from_slice(&flags.to_le_bytes());
    d.extend_from_slice(&element_bytes.to_le_bytes()); // ca_byte_offset_base
    d.extend_from_slice(&0u32.to_le_bytes());
    for &n in dims {
        d.extend_from_slice(&n.to_le_bytes());
    }
    // No composition link: the parent channel describes the element.
    block(b"##CA", &[0], &d)
}

#[test]
fn an_array_without_a_template_takes_its_element_from_the_parent_channel() {
    // Vector and dSPACE both emit look-up tables this way. `ca_composition` is
    // zero and needs to be: the parent channel's own data type and bit count
    // describe one element, and `ca_byte_offset_base` gives the stride. This
    // reader refused them on the grounds that "nothing says how wide an element
    // is", which was wrong on both counts and cost 15 channels across four
    // vendor files.
    let mut records = Vec::new();
    for v in [10u8, 20, 30, 40, 50, 60] {
        records.push(v);
    }

    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("Table"));
    let array = f.push(&ca_no_template(&[6], 1, 0));
    // The parent is a plain u8 channel; the CA says there are six of them.
    let channel = f.push(&cn(0, array, name, 0, 0, 0, 8));
    let group = f.push(&cg(channel, 1, 6));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("array_no_template").expect("should open");
    let ch = file.find_channel("Table").expect("channel");
    assert_eq!(ch.array_shape(), Some(&[6u64][..]));

    let values = file.signal(ch).expect("signal").values().expect("decode");
    assert_eq!(
        values,
        SignalValues::Array {
            values: vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0],
            elements_per_sample: 6,
        }
    );
}

#[test]
fn an_inverse_layout_array_is_returned_in_row_major_order() {
    // `ca_flags` bit 6 says the *first* dimension varies fastest in the record,
    // so the stored order is the transpose of what `SignalValues::Array`
    // documents itself as returning. The flag was parsed and ignored, which
    // handed back a transposed matrix — right dtype, right count, wrong
    // positions. dSPACE writes its matrices this way.
    //
    // A 2x3 matrix whose row-major contents are 1..6. Stored with the first
    // dimension fastest, that is column by column: 1, 4, 2, 5, 3, 6.
    let stored: [u8; 6] = [1, 4, 2, 5, 3, 6];

    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("Matrix"));
    let array = f.push(&ca_no_template(&[2, 3], 1, 1 << 6));
    let channel = f.push(&cn(0, array, name, 0, 0, 0, 8));
    let group = f.push(&cg(channel, 1, 6));
    let data = f.push(&dt(&stored));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("array_inverse").expect("should open");
    let ch = file.find_channel("Matrix").expect("channel");
    let values = file.signal(ch).expect("signal").values().expect("decode");

    assert_eq!(
        values,
        SignalValues::Array {
            values: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            elements_per_sample: 6,
        },
        "the stored order is column-major; the reported order is not"
    );
}

#[test]
fn a_dynamic_size_array_decodes_each_samples_real_count() {
    // `ca_dim_size` on a dynamic-size array is the largest shape a sample may
    // take; the real count for each sample lives in a companion channel of the
    // same record — here a one-byte "Count" field ahead of the array. Sample 0
    // uses 2 of its 3 possible elements, sample 1 all 3; the unused third slot
    // of sample 0 must not appear in the result, or a reader would hand back
    // the field's leftover bytes as though they were data.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let count_name = f.push(&tx("Count"));
    let values_name = f.push(&tx("Values"));
    let template = f.push(&cn(0, 0, 0, 0, 4, 0, 64)); // one f64 element

    // The dynamic-size triple (dg, cg, cn) cannot be known until the group
    // and the count channel are placed, so the CA block is written with
    // placeholders and patched once they are.
    let array = f.push(&ca_dynamic_size(template, 3, 8));

    // Values holds up to 3 f64s starting one byte into the record, after Count.
    let values_channel = f.push(&cn(0, array, values_name, 0, 4, 1, 64));
    // Count is a plain u8 at offset 0, chained ahead of Values.
    let count_channel = f.push(&cn(values_channel, 0, count_name, 0, 0, 0, 8));

    let group = f.push(&cg(count_channel, 2, 25)); // 1 count byte + 3*8 value bytes
    let mut records = Vec::new();
    records.push(2u8); // sample 0: 2 of 3 used
    records.extend_from_slice(&1.0f64.to_le_bytes());
    records.extend_from_slice(&2.0f64.to_le_bytes());
    records.extend_from_slice(&99.0f64.to_le_bytes()); // unused slot: must not surface
    records.push(3u8); // sample 1: all 3 used
    records.extend_from_slice(&4.0f64.to_le_bytes());
    records.extend_from_slice(&5.0f64.to_le_bytes());
    records.extend_from_slice(&6.0f64.to_le_bytes());
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    // Patch the CA block's dynamic-size triple now that its targets exist:
    // link[0] is ca_composition, so the triple starts at HEADER + 8.
    let triple_at = array + HEADER as u64 + 8;
    f.patch_link(triple_at, group_block); // dg: the ##DG block's own offset
    f.patch_link(triple_at + 8, group); // cg: the ##CG block's own offset
    f.patch_link(triple_at + 16, count_channel); // cn: the count channel

    let file = f.open("array_dynamic").expect("synthetic file should open");
    let ch = file
        .find_channel("Values")
        .expect("the array channel should be listed");
    assert_eq!(
        ch.array_shape(),
        Some(&[3u64][..]),
        "3 is the maximum, not a fixed shape"
    );
    assert!(
        ch.unreadable().is_none(),
        "a resolvable dynamic-size array should be readable"
    );

    let values = file.signal(ch).expect("signal").values().expect("decode");
    assert_eq!(
        values,
        SignalValues::ArrayVarLen {
            values: vec![1.0, 2.0, 4.0, 5.0, 6.0],
            starts: vec![0, 2, 5],
        },
        "sample 0 contributes its 2 real elements, not the 3rd unused slot"
    );
}

#[test]
fn a_dynamic_size_array_whose_sizing_channel_cannot_be_resolved_fails_to_read() {
    // The dynamic-size triple names a channel by (data group, channel group,
    // channel) file offsets, which the standard allows to point anywhere. This
    // build can only read a sizing channel sharing the array's own record, so
    // an unresolved triple — here, one left at zero — must not be read as
    // though the array were fixed at its declared maximum. Unlike the earlier
    // refusal this reader used for every dynamic-size array, the channel stays
    // listed as readable (its shape is a real upper bound, and the CA block
    // does name a template): only the *read* fails, exactly as a
    // maximum-length channel with no length channel does.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Detections"));
    let template = f.push(&cn(0, 0, 0, 0, 4, 0, 64));
    let array = f.push(&ca_dynamic_size(template, 3, 8));
    let channel = f.push(&cn(0, array, name, 0, 4, 0, 64));
    let group = f.push(&cg(channel, 2, 24));
    let data = f.push(&dt(&[0u8; 48]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f
        .open("array_dynamic_unresolved")
        .expect("synthetic file should open");
    let ch = file
        .find_channel("Detections")
        .expect("the array channel should still be listed");

    assert!(
        ch.unreadable().is_none(),
        "the shape is a real upper bound and the CA names a template, so the \
         channel is not unreadable — only its dynamic size is unresolved"
    );
    assert!(
        file.signal(ch).expect("signal").values().is_err(),
        "a dynamic size that cannot be resolved must fail rather than read at its maximum"
    );
}

#[test]
fn a_dynamic_size_arrays_declared_maximum_too_large_for_its_record_is_rejected() {
    // Even a resolvable dynamic-size array is bounded by B35's structural
    // check: the declared *maximum* still has to fit the record, checked once
    // up front, before any per-sample count is trusted. A dynamic size does
    // not exempt an array from the same file-supplied-number problem B35 was.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let count_name = f.push(&tx("Count"));
    let values_name = f.push(&tx("Values"));
    let template = f.push(&cn(0, 0, 0, 0, 4, 0, 64));
    // A billion as the declared maximum; the record only ever holds a few.
    let array = f.push(&ca_dynamic_size(template, 1_000_000_000, 8));
    let values_channel = f.push(&cn(0, array, values_name, 0, 4, 1, 64));
    let count_channel = f.push(&cn(values_channel, 0, count_name, 0, 0, 0, 8));
    let group = f.push(&cg(count_channel, 1, 25));
    let mut records = vec![2u8];
    records.extend_from_slice(&1.0f64.to_le_bytes());
    records.extend_from_slice(&2.0f64.to_le_bytes());
    records.extend_from_slice(&0.0f64.to_le_bytes());
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let triple_at = array + HEADER as u64 + 8;
    f.patch_link(triple_at, group_block);
    f.patch_link(triple_at + 8, group);
    f.patch_link(triple_at + 16, count_channel);

    let file = f
        .open("array_dynamic_huge_max")
        .expect("synthetic file should open");
    let ch = file.find_channel("Values").expect("channel");

    let err = file
        .signal(ch)
        .expect("signal")
        .values()
        .expect_err("a declared maximum bigger than the record can hold must be rejected");
    assert!(
        err.to_string().contains("Values"),
        "the error should name the channel"
    );
}

/// A channel with no conversion link, for pairing a stored channel with a
/// virtual one in the same group.
fn cn_named(next: u64, name: u64, channel_type: u8, byte_offset: u32, bit_count: u32) -> Vec<u8> {
    cn(next, 0, name, channel_type, 0, byte_offset, bit_count)
}

#[test]
fn a_virtual_master_channel_builds_a_time_base_from_its_factor() {
    // B21. Field values here are taken from the standard, not from the reader:
    // cn_type 3 is a virtual master, and its cn_bit_count *must* be 0 because
    // the channel occupies no bytes in the record. Its raw value is the
    // zero-based sample index, which the conversion scales — that is how a file
    // stores a regularly-spaced time base without writing one sample of it.
    //
    // The factor is deliberately non-zero. Every virtual channel in the corpus
    // has a factor of 0, which collapses the index to a constant and makes a
    // reader that ignores the index indistinguishable from a correct one.
    let mut f = FileBuilder::new();
    f.push(&hd());

    // phys = cc_val[0] + cc_val[1] * raw — 10 ms per sample, starting at 0.
    let conv = f.push(&cc(1, &[], &[0.0, 0.01]));

    // Records the virtual channel must not read: two bytes per sample, all
    // non-zero, so striding them would give 0x0707 rather than a ramp.
    let records = vec![7u8; 8];

    let temp_name = f.push(&tx("Temperature"));
    let time_name = f.push(&tx("t"));
    let time = f.push(&cn_converted(0, time_name, conv, 3, 0, 0, 0));
    let temp = f.push(&cn_named(time, temp_name, 0, 0, 16));
    let group = f.push(&cg(temp, 4, 2));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f
        .open("virtual_master")
        .expect("synthetic file should open");

    let time_ch = file.find_channel("t").expect("the master should be listed");
    let values = file
        .signal(time_ch)
        .expect("signal")
        .values_f64()
        .expect("a virtual master should decode");
    assert_eq!(values, vec![0.0, 0.01, 0.02, 0.03]);

    // The stored channel sharing the group is unaffected: the virtual rule must
    // not change how an ordinary field is read.
    let temp_ch = file
        .find_channel("Temperature")
        .expect("the stored channel should be listed");
    assert_eq!(
        file.signal(temp_ch)
            .expect("signal")
            .values_f64()
            .expect("a stored channel should decode"),
        vec![1799.0; 4],
        "0x0707 little-endian"
    );
}

#[test]
fn a_virtual_data_channel_counts_samples_when_it_has_no_conversion() {
    // Without a conversion the raw index is the value, and it must be reported
    // at a width that can hold it — cn_bit_count is 0, so sizing the value from
    // the field would wrap the count at 256.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Index"));
    let channel = f.push(&cn_named(0, name, 6, 0, 0));
    let group = f.push(&cg(channel, 300, 1));
    let data = f.push(&dt(&vec![0xFFu8; 300]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("virtual_data").expect("synthetic file should open");
    let ch = file
        .find_channel("Index")
        .expect("channel should be listed");
    let values = file
        .signal(ch)
        .expect("signal")
        .values_f64()
        .expect("a virtual data channel should decode");

    assert_eq!(values.len(), 300);
    assert_eq!(values[0], 0.0);
    assert_eq!(values[299], 299.0, "the index must not wrap at 256");
}

/// A channel group that declares invalidation bytes after its data bytes.
///
/// The record is `[record id][cg_data_bytes][cg_inval_bytes]`, so the
/// invalidation area begins where the data ends.
fn cg_with_inval(cn_first: u64, cycle_count: u64, data_bytes: u32, inval_bytes: u32) -> Vec<u8> {
    let mut d = vec![0u8; 32];
    d[8..16].copy_from_slice(&cycle_count.to_le_bytes());
    d[24..28].copy_from_slice(&data_bytes.to_le_bytes());
    d[28..32].copy_from_slice(&inval_bytes.to_le_bytes());
    block(b"##CG", &[0, cn_first, 0, 0, 0, 0], &d)
}

/// A one-byte unsigned channel carrying a per-sample invalidation bit.
///
/// `cn_flags` bit 1 marks the bit as present, and `cn_inval_bit_pos` locates it
/// within the group's invalidation bytes.
fn cn_invalidated(next: u64, name: u64, byte_offset: u32, inval_bit_pos: u32) -> Vec<u8> {
    let mut d = vec![0u8; 72];
    d[2] = 0; // cn_data_type: unsigned, little-endian
    d[4..8].copy_from_slice(&byte_offset.to_le_bytes());
    d[8..12].copy_from_slice(&8u32.to_le_bytes()); // cn_bit_count
    d[12..16].copy_from_slice(&0x0002u32.to_le_bytes()); // cn_flags: invalidation bit
    d[16..20].copy_from_slice(&inval_bit_pos.to_le_bytes());
    block(b"##CN", &[next, 0, name, 0, 0, 0, 0, 0], &d)
}

#[test]
fn an_unfinalized_file_reports_which_bookkeeping_its_writer_never_did() {
    // 4.10.5. The seven flags were collapsed to one boolean, so a caller could
    // learn that a file was unfinalized but not what that meant for the values
    // it was about to read — whether the sample counts were stale (compensated
    // for) or a variable-length channel's offsets were never written (not).
    //
    // Bit positions come from the standard: 0 CG counters, 1 SR counters,
    // 2 last DT length, 3 last RD length, 4 last DL, 5 VLSD byte counts,
    // 6 VLSD offsets.
    let mut f = FileBuilder::new();
    f.bytes[0..8].copy_from_slice(b"UnFinMF ");
    // CG counters, last DT length and VLSD offsets: one from each end and one
    // in the middle, so a shifted table cannot land on all three.
    f.bytes[60..62].copy_from_slice(&0b0100_0101u16.to_le_bytes());
    f.bytes[62..64].copy_from_slice(&0xBEEFu16.to_le_bytes());
    f.push(&hd());

    let name = f.push(&tx("Speed"));
    let channel = f.push(&cn(0, 0, name, 0, 0, 0, 8));
    let group = f.push(&cg(channel, 3, 1));
    let data = f.push(&dt(&[1u8, 2, 3]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f
        .open("unfinalized")
        .expect("an unfinalized file still opens");
    let flags = file
        .unfinalized()
        .expect("a file with the UnFinMF signature is unfinalized");

    assert!(flags.update_cg_counters, "bit 0");
    assert!(flags.update_last_dt_length, "bit 2");
    assert!(flags.update_vlsd_offsets, "bit 6");

    assert!(!flags.update_sr_counters, "bit 1 was not set");
    assert!(!flags.update_last_rd_length, "bit 3 was not set");
    assert!(!flags.update_last_dl, "bit 4 was not set");
    assert!(!flags.update_vlsd_bytes, "bit 5 was not set");

    assert_eq!(
        flags.custom, 0xBEEF,
        "a writer's own flags are passed through"
    );

    // And the compensation the flags describe is real: the counts come from the
    // data, not from a cg_cycle_count the writer never revised.
    let ch = file.find_channel("Speed").expect("channel");
    assert_eq!(file.signal(ch).expect("signal").len(), 3);
}

#[test]
fn a_finalized_file_reports_nothing_left_undone() {
    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("Speed"));
    let channel = f.push(&cn(0, 0, name, 0, 0, 0, 8));
    let group = f.push(&cg(channel, 1, 1));
    let data = f.push(&dt(&[7u8]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("finalized").expect("should open");
    assert_eq!(file.unfinalized(), None);
}

#[test]
fn a_data_block_this_build_cannot_read_is_refused_rather_than_read_as_empty() {
    // 4.10.4 / B25. `##LD` is the list-data block a 4.2 file uses in place of a
    // DL; this reader does not know it. It used to fall through to an empty
    // index, so the file opened, the channel appeared, and every one of its
    // three samples silently became zero samples. An empty measurement is a
    // plausible answer to "what does this file contain", which is what makes it
    // worse than an error.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Speed"));
    let channel = f.push(&cn(0, 0, name, 0, 0, 0, 8));
    let group = f.push(&cg(channel, 3, 1));
    let unknown = f.push(&block(b"##LD", &[0], &[1u8, 2, 3]));
    let group_block = f.push(&dg(0, group, unknown, 0));
    f.patch_link(hd_link(0), group_block);

    let err = match f.open("unknown_data_block") {
        Err(e) => e,
        Ok(file) => panic!(
            "a data group pointing at an unreadable block must not open as \
             valid; got {} channel(s)",
            file.channel_count()
        ),
    };
    let text = err.to_string();
    assert!(
        text.contains("LD"),
        "the error should name the block it could not read: {text}"
    );
}

#[test]
fn a_data_list_naming_a_block_this_build_cannot_read_is_refused() {
    // The same fallthrough, one level down and more damaging: a data list holds
    // one block per segment of a group's records, so skipping an entry drops a
    // slice out of the middle of the stream and shifts every segment after it.
    // The samples that survive are real values at the wrong times.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Speed"));
    let channel = f.push(&cn(0, 0, name, 0, 0, 0, 8));
    let group = f.push(&cg(channel, 4, 1));

    let first = f.push(&dt(&[1u8, 2]));
    let unreadable = f.push(&block(b"##LD", &[0], &[3u8, 4]));

    // DL data: flags, three reserved bytes, the block count, then one offset
    // per block. Links are the next DL and one per data block.
    let mut d = vec![0u8; 8];
    d[4..8].copy_from_slice(&2u32.to_le_bytes());
    d.extend_from_slice(&0u64.to_le_bytes()); // offset of the first block
    d.extend_from_slice(&2u64.to_le_bytes()); // offset of the second
    let list = f.push(&block(b"##DL", &[0, first, unreadable], &d));

    let group_block = f.push(&dg(0, group, list, 0));
    f.patch_link(hd_link(0), group_block);

    assert!(
        f.open("unknown_in_data_list").is_err(),
        "a list naming a block this build cannot read must not open as valid"
    );
}

/// A one-byte unsigned channel the file declares wholly invalid.
///
/// `cn_flags` bit 0 is "all values invalid", and it stands alone: it needs no
/// invalidation bit, no `cn_inval_bit_pos`, and no `cg_inval_bytes` in the
/// group. That independence is the point — a reader that only consults the
/// per-sample bit never sees this at all.
fn cn_all_invalid(next: u64, name: u64, byte_offset: u32) -> Vec<u8> {
    let mut d = vec![0u8; 72];
    d[2] = 0; // cn_data_type: unsigned, little-endian
    d[4..8].copy_from_slice(&byte_offset.to_le_bytes());
    d[8..12].copy_from_slice(&8u32.to_le_bytes()); // cn_bit_count
    d[12..16].copy_from_slice(&0x0001u32.to_le_bytes()); // cn_flags: all invalid
    block(b"##CN", &[next, 0, name, 0, 0, 0, 0, 0], &d)
}

#[test]
fn a_channel_flagged_wholly_invalid_reports_no_valid_samples() {
    // 4.10.3 / B24. The flag was parsed and dropped on the way to `Channel`,
    // so a channel the file says holds no measurements reported every sample
    // valid and handed the bytes back as data.
    //
    // Two channels share the record: one flagged, one not. A fix that simply
    // marked everything invalid would pass the first assertion and fail the
    // second.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let records: Vec<u8> = vec![10, 20, 30, 40, 50, 60];

    let plain_name = f.push(&tx("Measured"));
    let plain = f.push(&cn_named(0, plain_name, 0, 1, 8));
    let flagged_name = f.push(&tx("Unusable"));
    let flagged = f.push(&cn_all_invalid(plain, flagged_name, 0));

    let group = f.push(&cg(flagged, 3, 2));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("all_invalid").expect("synthetic file should open");

    let flagged_ch = file.find_channel("Unusable").expect("channel");
    let signal = file.signal(flagged_ch).expect("signal");
    assert_eq!(
        signal.validity(),
        Some(vec![false, false, false]),
        "a channel flagged all-invalid has no valid sample"
    );
    assert_eq!(signal.valid_count(), 0);
    assert!(!signal.is_valid(0), "not even the first");

    // The flag is per channel, so its neighbour in the same record is
    // untouched — it carries no invalidation bit and is wholly valid.
    let plain_ch = file.find_channel("Measured").expect("channel");
    let plain_signal = file.signal(plain_ch).expect("signal");
    assert_eq!(
        plain_signal.validity(),
        None,
        "a channel with no invalidation information is not affected"
    );
    assert_eq!(plain_signal.valid_count(), 3);
}

#[test]
fn a_synchronisation_channel_is_reported_unreadable_before_decoding() {
    // G3. A synchronisation channel (`cn_type` 4) indexes a media stream
    // rather than carrying measurements. Until now that only surfaced when a
    // read was attempted, so a caller listing channels — a UI asking of every
    // channel whether it can be shown — could not learn it in advance.
    // Parsing already knows, so `unreadable()` reports it; the decode-time
    // refusal stays as the backstop for signals assembled by hand.
    let mut f = FileBuilder::new();
    f.push(&hd());

    // Four records of [master f64][data f64][the sync channel's 8 bytes,
    // which no decode ever reads].
    let mut records = Vec::new();
    for i in 0..4u64 {
        records.extend_from_slice(&(i as f64 * 0.001).to_le_bytes());
        records.extend_from_slice(&(i as f64).to_le_bytes());
        records.extend_from_slice(&0u64.to_le_bytes());
    }

    let master_name = f.push(&tx("Time"));
    let data_name = f.push(&tx("Data"));
    let sync_name = f.push(&tx("Media"));

    // Channels are pushed in reverse link order. Data type 4 is FloatLe; the
    // sync channel's data type is irrelevant since it is never decoded.
    let sync = f.push(&cn(0, 0, sync_name, 4, 0, 16, 64));
    let data = f.push(&cn(sync, 0, data_name, 0, 4, 8, 64));
    let master = f.push(&cn(data, 0, master_name, 2, 4, 0, 64));

    let group = f.push(&cg(master, 4, 24));
    let data_block = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data_block, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f
        .open("sync_unreadable")
        .expect("synthetic file should open");

    let sync_ch = file.find_channel("Media").expect("sync channel");
    assert_eq!(
        sync_ch.unreadable(),
        Some(UnreadableReason::SyncChannel),
        "parsing knows a sync channel cannot be decoded; the listing should too"
    );

    // The decode-time refusal remains, and both sides agree on why.
    match file.signal(sync_ch).expect("signal").values() {
        Err(Mf4Error::Unsupported { feature, detail }) => {
            assert!(feature.contains("synchronisation"), "feature: {feature}");
            assert!(detail.contains("media stream"), "detail: {detail}");
        }
        other => panic!("a sync channel must not decode: {other:?}"),
    }

    // Its neighbour in the same record is untouched — a fix that marked the
    // whole group unreadable would fail here.
    let data_ch = file.find_channel("Data").expect("data channel");
    assert!(data_ch.unreadable().is_none());
    let values = file
        .signal(data_ch)
        .expect("signal")
        .values_f64()
        .expect("decode");
    assert_eq!(values, vec![0.0, 1.0, 2.0, 3.0]);
}

#[test]
fn invalidation_bits_mark_the_samples_the_file_says_are_not_measurements() {
    // 4.9.2. Polarity is the fact worth getting from the standard rather than
    // from the reader: a *set* cn_inval_bit_pos bit means the sample is
    // INVALID. `validity()` inverts that, so `true` there means valid.
    //
    // Two channels share one invalidation byte at different bit positions, so
    // the test fails if a channel reads its neighbour's bit rather than its own.
    let mut f = FileBuilder::new();
    f.push(&hd());

    // [a][b][inval] per sample. Bit 0 invalidates `a`, bit 3 invalidates `b`.
    let records: Vec<u8> = vec![
        10,
        20,
        0b0000_0000, // both valid
        11,
        21,
        0b0000_0001, // a invalid
        12,
        22,
        0b0000_1000, // b invalid
        13,
        23,
        0b0000_1001, // both invalid
    ];

    let name_a = f.push(&tx("a"));
    let name_b = f.push(&tx("b"));
    let ch_b = f.push(&cn_invalidated(0, name_b, 1, 3));
    let ch_a = f.push(&cn_invalidated(ch_b, name_a, 0, 0));
    let group = f.push(&cg_with_inval(ch_a, 4, 2, 1));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("invalidation").expect("synthetic file should open");

    let a = file
        .signal(file.find_channel("a").expect("a"))
        .expect("signal");
    assert_eq!(
        a.validity(),
        Some(vec![true, false, true, false]),
        "channel a must follow bit 0"
    );
    assert_eq!(a.valid_count(), 2);

    let b = file
        .signal(file.find_channel("b").expect("b"))
        .expect("signal");
    assert_eq!(
        b.validity(),
        Some(vec![true, true, false, false]),
        "channel b must follow bit 3"
    );
    assert_eq!(b.valid_count(), 2);

    // Invalid samples are still returned — the documented contract is that
    // `validity()` says which of them are measurements, not that the rest are
    // removed or zeroed.
    assert_eq!(
        a.values_f64().expect("a should decode"),
        vec![10.0, 11.0, 12.0, 13.0]
    );
    assert!(a.is_valid(0) && !a.is_valid(1) && a.is_valid(2) && !a.is_valid(3));
}

#[test]
fn a_channel_without_the_flag_reports_no_validity_even_beside_one_that_has_it() {
    // Invalidation bytes belong to the group, the flag to the channel. A
    // channel without the flag has no bit in that area, and must not be given
    // one by position — reporting `None` is what says "every sample is valid".
    let mut f = FileBuilder::new();
    f.push(&hd());

    // Bit 0 is set on every sample; the unflagged channel must ignore it.
    let records: Vec<u8> = vec![10, 20, 1, 11, 21, 1];

    let name_plain = f.push(&tx("plain"));
    let name_flagged = f.push(&tx("flagged"));
    let plain_ch = f.push(&cn_named(0, name_plain, 0, 1, 8));
    let flagged = f.push(&cn_invalidated(plain_ch, name_flagged, 0, 0));
    let group = f.push(&cg_with_inval(flagged, 2, 2, 1));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("invalidation_mixed").expect("file should open");

    let plain = file
        .signal(file.find_channel("plain").expect("plain"))
        .expect("signal");
    assert_eq!(plain.validity(), None);
    assert_eq!(plain.valid_count(), 2);

    let flagged = file
        .signal(file.find_channel("flagged").expect("flagged"))
        .expect("signal");
    assert_eq!(flagged.validity(), Some(vec![false, false]));
    assert_eq!(flagged.valid_count(), 0);
}

#[test]
fn text_channels_decode_by_the_codes_the_standard_assigns() {
    // 4.10.1 / B22. `cn_data_type` runs 6 = string SBC (ISO-8859-1), 7 = UTF-8,
    // 8 = UTF-16LE, 9 = UTF-16BE. The reader had no SBC variant and read every
    // code above it one too low, so UTF-8 text was decoded as UTF-16 and
    // UTF-16LE text was byte-swapped — both silent garbage.
    //
    // Every sample here is non-ASCII on purpose. ASCII survives all four
    // encodings, which is exactly why a fixture written in it would pass
    // against the shifted table and prove nothing.
    let mut f = FileBuilder::new();
    f.push(&hd());

    // "Öl" in ISO-8859-1 is two bytes and is not valid UTF-8, so a decoder
    // reaching for UTF-8 yields a replacement character instead.
    let sbc = [0xD6u8, 0x6C, 0x00, 0x00];
    // "°C" in UTF-8. Read as UTF-16LE this is U+B0C2 followed by a stray byte.
    let utf8 = [0xC2u8, 0xB0, 0x43, 0x00];
    // "°C" in UTF-16LE, and the same text in UTF-16BE. The two are byte-swaps
    // of each other, so reading either with the wrong endianness gives U+B000
    // and U+4300 rather than a degree sign and a C.
    let utf16le = [0xB0u8, 0x00, 0x43, 0x00];
    let utf16be = [0x00u8, 0xB0, 0x00, 0x43];

    let mut record = Vec::new();
    for field in [&sbc, &utf8, &utf16le, &utf16be] {
        record.extend_from_slice(field);
    }

    // One channel per encoding, four bytes each, laid out end to end.
    let mut next = 0u64;
    for (i, (label, code)) in [("Sbc", 6u8), ("Utf8", 7), ("Utf16Le", 8), ("Utf16Be", 9)]
        .iter()
        .enumerate()
        .rev()
    {
        let name = f.push(&tx(label));
        next = f.push(&cn(next, 0, name, 0, *code, (i * 4) as u32, 32));
    }

    let group = f.push(&cg(next, 1, record.len() as u32));
    let data = f.push(&dt(&record));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f
        .open("text_encodings")
        .expect("synthetic file should open");

    // Every encoding is checked before anything is asserted. Stopping at the
    // first mismatch would report one shifted code and hide the other three,
    // which is the opposite of what a table this easy to misnumber needs.
    let mut wrong = Vec::new();
    for label in ["Sbc", "Utf8", "Utf16Le", "Utf16Be"] {
        let expected = if label == "Sbc" { "Öl" } else { "°C" };
        let ch = file.find_channel(label).expect("channel");
        let got = match file.signal(ch).expect("signal").values() {
            Ok(SignalValues::Str(text)) => text[0].clone(),
            Ok(other) => format!("not text: {other:?}"),
            Err(e) => format!("error: {e}"),
        };
        if got != expected {
            wrong.push(format!("{label}: expected {expected:?}, got {got:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "text decoded wrongly:\n  {}",
        wrong.join("\n  ")
    );
}

/// A fixed-width text channel carrying a conversion block.
fn cn_text_with_conversion(name: u64, conversion: u64, data_type: u8, bytes: u32) -> Vec<u8> {
    let mut d = vec![0u8; 72];
    d[2] = data_type;
    d[8..12].copy_from_slice(&(bytes * 8).to_le_bytes()); // cn_bit_count
    block(b"##CN", &[0, 0, name, 0, conversion, 0, 0, 0], &d)
}

/// A rational conversion (`cc_type` 2) whose coefficients evaluate to `x`.
///
/// Writers attach these to channels that need no conversion at all, which is
/// how a text channel ends up carrying one.
fn cc_rational_identity() -> Vec<u8> {
    // cc_type, precision, flags, cc_ref_count, cc_val_count, then the physical
    // range pair — 24 bytes before the parameters themselves.
    let mut d = vec![0u8; 24];
    d[0] = 2; // cc_type = rational
    d[6..8].copy_from_slice(&6u16.to_le_bytes()); // cc_val_count
    for p in [0.0f64, 1.0, 0.0, 0.0, 0.0, 1.0] {
        d.extend_from_slice(&p.to_le_bytes());
    }
    block(b"##CC", &[0, 0, 0, 0], &d)
}

#[test]
fn a_numeric_conversion_does_not_turn_a_text_channel_into_numbers() {
    // Found in `ASAP2_Demo_V171.mf4`, written by TGT 15.0 — the first real file
    // from another tool to carry a string channel. Its `$CalibrationLog` is a
    // 256-byte SBC text field with an identity *rational* conversion attached.
    // The reader saw a non-identity conversion, concluded the channel was
    // numeric, and returned 0.0 for every sample.
    //
    // The data type decides what the record holds. A conversion keyed by
    // numbers cannot consume text, so it does not apply — which is what the
    // reference does too, returning the bytes and ignoring the conversion.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let mut record = Vec::new();
    record.extend_from_slice(b"page-1\0\0");
    record.extend_from_slice(b"page-22\0");

    let name = f.push(&tx("CalibrationLog"));
    let conversion = f.push(&cc_rational_identity());
    let channel = f.push(&cn_text_with_conversion(name, conversion, 6, 8));
    let group = f.push(&cg(channel, 2, 8));
    let data = f.push(&dt(&record));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("text_with_conversion").expect("should open");
    let ch = file.find_channel("CalibrationLog").expect("channel");
    let values = file.signal(ch).expect("signal").values().expect("decode");

    assert_eq!(
        values,
        SignalValues::Str(vec!["page-1".into(), "page-22".into()]),
        "a text channel stays text however its writer decorated it"
    );
}

#[test]
fn canopen_records_from_a_vendor_file_decode_to_the_instants_they_encode() {
    // The records here are copied byte for byte out of Vector's reference files
    // `Vector_CANOpenDate.mf4` and `Vector_CANOpenTime.mf4` — the first real
    // files this decoder has ever seen, 4.9.3 having been built from the
    // standard alone with no way to check it against a writer.
    //
    // The expected instants were derived from the bytes by hand against CiA
    // 301, not read out of this implementation. Worth stating because the
    // obvious oracle failed here: asammdf returns nine bytes for a
    // seven-byte field, misaligned, so its output could not settle it.
    let dates: [(&[u8; 7], i64, u8); 2] = [
        // 1996-10-15 11:19:30.000, a Monday. Byte 4 is 0x2f: day 15 in the low
        // five bits, day-of-week 1 in the top three.
        (
            &[0x30, 0x75, 0x13, 0x0b, 0x2f, 0x0a, 0x0c],
            845_378_370_000_000_000,
            1,
        ),
        // 1996-10-16 13:20:00.000, a Tuesday.
        (
            &[0x00, 0x00, 0x14, 0x0d, 0x50, 0x0a, 0x0c],
            845_472_000_000_000_000,
            2,
        ),
    ];

    let mut records = Vec::new();
    for (bytes, _, _) in &dates {
        records.extend_from_slice(*bytes);
    }

    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("Vector date"));
    let channel = f.push(&cn(0, 0, name, 0, 13, 0, 56));
    let group = f.push(&cg(channel, dates.len() as u64, 7));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("vector_canopen_date").expect("should open");
    let ch = file.find_channel("Vector date").expect("channel");
    let SignalValues::CanopenDate(got) = file.signal(ch).expect("signal").values().expect("decode")
    else {
        panic!("a CANopen date channel must decode as dates");
    };

    for (i, (_, nanos, dow)) in dates.iter().enumerate() {
        assert_eq!(got[i].to_unix_nanos(), *nanos, "date {i}");
        assert_eq!(got[i].day_of_week, *dow, "date {i} day of week");
    }

    // 12:31:00.000 on day 10511 after 1984-01-01, with the top four bits of the
    // millisecond word set — reserved, and part of why this layout needs a real
    // file to confirm.
    let time_record: [u8; 6] = [0xa0, 0x8f, 0xaf, 0x02, 0x0f, 0x29];

    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("Vector time"));
    let channel = f.push(&cn(0, 0, name, 0, 14, 0, 48));
    let group = f.push(&cg(channel, 1, 6));
    let data = f.push(&dt(&time_record));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("vector_canopen_time").expect("should open");
    let ch = file.find_channel("Vector time").expect("channel");
    let SignalValues::CanopenTime(got) = file.signal(ch).expect("signal").values().expect("decode")
    else {
        panic!("a CANopen time channel must decode as times");
    };
    assert_eq!(got[0].days_since_1984, 10_511);
    assert_eq!(got[0].to_unix_nanos(), 1_349_958_660_000_000_000);
}

#[test]
fn a_canopen_date_channel_decodes_each_field_from_its_own_bits() {
    // 4.9.3. Every field but the milliseconds shares a byte with reserved or
    // flag bits, so the fixture deliberately *sets* those neighbours: a decoder
    // that reads a byte whole rather than masking it will fold them in and
    // produce a month of 0x83 or an hour with the summer-time bit added.
    //
    // 2026-08-03T12:34:56.789, Monday, summer time.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let ms: u16 = 56_789; // seconds live inside the minute's ms field
    let mut sample = Vec::new();
    sample.extend_from_slice(&ms.to_le_bytes()); // bytes 0-1
    sample.push(34 | 0b1100_0000); // minutes, reserved bits 6-7 set
    sample.push(12 | 0b1000_0000); // hours + summer time
    sample.push(3 | (1 << 5)); // day of month + day of week (Monday)
    sample.push(8 | 0b1100_0000); // month, reserved bits 6-7 set
    sample.push(42 | 0b1000_0000); // 1984 + 42 = 2026, reserved bit 7 set
    assert_eq!(sample.len(), 7);

    let name = f.push(&tx("Timestamp"));
    // cn_data_type 13 = CANopen date; cn_bit_count 56 = seven bytes.
    let channel = f.push(&cn(0, 0, name, 0, 13, 0, 56));
    let group = f.push(&cg(channel, 1, 7));
    let data = f.push(&dt(&sample));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("canopen_date").expect("synthetic file should open");
    let ch = file.find_channel("Timestamp").expect("channel");
    let values = file.signal(ch).expect("signal").values().expect("decode");

    let SignalValues::CanopenDate(dates) = values else {
        panic!("a CANopen date channel must decode as dates, got {values:?}");
    };
    let d = dates[0];
    assert_eq!(d.year, 2026, "year counts from 1984, masked to seven bits");
    assert_eq!(d.month, 8);
    assert_eq!(d.day, 3);
    assert_eq!(d.hour, 12);
    assert_eq!(d.minute, 34);
    assert_eq!(d.ms, 56_789);
    assert_eq!(d.day_of_week, 1);
    assert!(d.summer_time);

    let expected = (20_668i64 * 86_400 + 12 * 3_600 + 34 * 60) * 1_000_000_000 + 56_789 * 1_000_000;
    assert_eq!(d.to_unix_nanos(), expected);
}

#[test]
fn a_canopen_time_channel_masks_its_reserved_bits() {
    // The ms field is 28 bits inside a 32-bit word; the top four are reserved
    // and set here, so a decoder taking the word whole reports a wildly wrong
    // time rather than 12:34:56.789.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let ms: u32 = (12 * 3_600 + 34 * 60) * 1_000 + 56_789;
    let days: u16 = (20_668 - 5_113) as u16; // 2026-08-03, days since 1984
    let mut sample = Vec::new();
    sample.extend_from_slice(&(ms | 0xF000_0000).to_le_bytes());
    sample.extend_from_slice(&days.to_le_bytes());
    assert_eq!(sample.len(), 6);

    let name = f.push(&tx("Elapsed"));
    // cn_data_type 14 = CANopen time; cn_bit_count 48 = six bytes.
    let channel = f.push(&cn(0, 0, name, 0, 14, 0, 48));
    let group = f.push(&cg(channel, 1, 6));
    let data = f.push(&dt(&sample));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("canopen_time").expect("synthetic file should open");
    let ch = file.find_channel("Elapsed").expect("channel");
    let values = file.signal(ch).expect("signal").values().expect("decode");

    let SignalValues::CanopenTime(times) = values else {
        panic!("a CANopen time channel must decode as times, got {values:?}");
    };
    assert_eq!(
        times[0].ms_since_midnight, ms,
        "the top four bits are reserved"
    );
    assert_eq!(times[0].days_since_1984, days);

    let expected = (20_668i64 * 86_400 + 12 * 3_600 + 34 * 60) * 1_000_000_000 + 56_789 * 1_000_000;
    assert_eq!(times[0].to_unix_nanos(), expected);
}

#[test]
fn a_complex_channel_splits_each_sample_into_its_two_parts() {
    // cn_bit_count covers the pair, so 128 bits is two f64. Reading it as one
    // number would give the real part alone and drop the imaginary half.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let mut records = Vec::new();
    for (re, im) in [(1.5f64, -2.5f64), (0.0, 7.25)] {
        records.extend_from_slice(&re.to_le_bytes());
        records.extend_from_slice(&im.to_le_bytes());
    }

    let name = f.push(&tx("Impedance"));
    // cn_data_type 15 = complex, little-endian.
    let channel = f.push(&cn(0, 0, name, 0, 15, 0, 128));
    let group = f.push(&cg(channel, 2, 16));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("complex").expect("synthetic file should open");
    let ch = file.find_channel("Impedance").expect("channel");
    let values = file.signal(ch).expect("signal").values().expect("decode");

    assert_eq!(
        values,
        SignalValues::Complex {
            re: vec![1.5, 0.0],
            im: vec![-2.5, 7.25],
        }
    );
}

#[test]
fn a_complex_channel_of_an_impossible_width_is_refused() {
    // A complex sample is two floats. Anything but 64 or 128 bits is not one,
    // and guessing which half is which would be inventing the layout.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Bad"));
    let channel = f.push(&cn(0, 0, name, 0, 15, 0, 96));
    let group = f.push(&cg(channel, 1, 12));
    let data = f.push(&dt(&[0u8; 12]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("complex_bad").expect("synthetic file should open");
    let ch = file.find_channel("Bad").expect("channel");
    assert!(file.signal(ch).expect("signal").values().is_err());
}

/// A channel whose `cn_data` link points somewhere — for MLSD, at the channel
/// counting each sample's bytes.
#[allow(clippy::too_many_arguments)]
fn cn_with_data(
    next: u64,
    name: u64,
    data: u64,
    channel_type: u8,
    data_type: u8,
    byte_offset: u32,
    bit_count: u32,
) -> Vec<u8> {
    let mut d = vec![0u8; 72];
    d[0] = channel_type;
    d[2] = data_type;
    d[4..8].copy_from_slice(&byte_offset.to_le_bytes());
    d[8..12].copy_from_slice(&bit_count.to_le_bytes());
    block(b"##CN", &[next, 0, name, 0, 0, data, 0, 0], &d)
}

#[test]
fn a_maximum_length_channel_takes_each_sample_size_from_its_length_channel() {
    // 4.9.4. An MLSD channel keeps its data in the record, sized to the longest
    // sample it will hold, and cn_data names the channel of the same group
    // whose value counts the bytes actually used. That link is what makes it
    // unlike VLSD, where cn_data points at a signal data block instead.
    //
    // The unused tail of each field is filled with 0xFF, so a decoder that
    // ignores the length and returns the whole field fails loudly.
    let mut f = FileBuilder::new();
    f.push(&hd());

    // Record: [4-byte payload field][1-byte length]
    let records: Vec<u8> = vec![
        0xDE, 0xAD, 0xFF, 0xFF, 2, // two bytes used
        0x01, 0x02, 0x03, 0x04, 4, // all four used
        0xFF, 0xFF, 0xFF, 0xFF, 0, // none used
        0x7F, 0xFF, 0xFF, 0xFF, 1, // one byte used
    ];

    let len_name = f.push(&tx("PayloadLength"));
    let data_name = f.push(&tx("Payload"));

    // The length channel is an ordinary unsigned field at byte 4.
    let len_ch = f.push(&cn(0, 0, len_name, 0, 0, 4, 8));
    // cn_type 5 = maximum length; cn_bit_count 32 is the *maximum* size, and
    // cn_data points at the length channel rather than at a data block.
    let data_ch = f.push(&cn_with_data(len_ch, data_name, len_ch, 5, 9, 0, 32));
    let group = f.push(&cg(data_ch, 4, 5));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("mlsd").expect("synthetic file should open");
    let ch = file
        .find_channel("Payload")
        .expect("channel should be listed");
    let values = file.signal(ch).expect("signal").values().expect("decode");

    assert_eq!(values.len(), 4);
    assert_eq!(values.bytes_at(0), Some(&[0xDE, 0xAD][..]));
    assert_eq!(values.bytes_at(1), Some(&[0x01, 0x02, 0x03, 0x04][..]));
    assert_eq!(values.bytes_at(2), Some(&[][..]), "a zero-length sample");
    assert_eq!(values.bytes_at(3), Some(&[0x7F][..]));

    // The length channel itself is still an ordinary readable channel.
    let len = file
        .signal(file.find_channel("PayloadLength").expect("length channel"))
        .expect("signal")
        .values_f64()
        .expect("the length channel should decode");
    assert_eq!(len, vec![2.0, 4.0, 0.0, 1.0]);
}

#[test]
fn a_maximum_length_sample_longer_than_its_field_is_rejected() {
    // A count past the declared maximum would take bytes belonging to whatever
    // follows in the record. That is the file contradicting itself, and
    // clamping it would hand those bytes back as though they were payload.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let records: Vec<u8> = vec![0x01, 0x02, 0x03, 0x04, 9];

    let len_name = f.push(&tx("Len"));
    let data_name = f.push(&tx("Payload"));
    let len_ch = f.push(&cn(0, 0, len_name, 0, 0, 4, 8));
    let data_ch = f.push(&cn_with_data(len_ch, data_name, len_ch, 5, 9, 0, 32));
    let group = f.push(&cg(data_ch, 1, 5));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("mlsd_overlong").expect("synthetic file should open");
    let ch = file.find_channel("Payload").expect("channel");
    assert!(file.signal(ch).expect("signal").values().is_err());
}

#[test]
fn a_maximum_length_channel_without_a_length_channel_stays_unreadable() {
    // cn_data is what makes MLSD decodable. Without it the used bytes cannot be
    // told from the unused ones, so the honest answer is still Unsupported.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Payload"));
    let channel = f.push(&cn(0, 0, name, 5, 9, 0, 32));
    let group = f.push(&cg(channel, 1, 4));
    let data = f.push(&dt(&[0xAAu8; 4]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f
        .open("mlsd_no_length")
        .expect("synthetic file should open");
    let ch = file.find_channel("Payload").expect("channel");
    assert!(file.signal(ch).expect("signal").values().is_err());
}

#[test]
fn a_hierarchy_nodes_name_and_comment_resolve_through_the_text_cache() {
    // 4.14.4. The CH block parser has its own unit tests, but nothing had
    // exercised the public accessor end to end.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Powertrain"));
    let comment = f.push(&md(
        "<CHcomment><TX>engine and transmission signals</TX></CHcomment>",
    ));
    let node = f.push(&ch(0, 0, name, comment, &[], 0));
    f.patch_link(hd_link(HD_CH), node);

    let file = f.open("ch_basic").expect("synthetic file should open");
    let hierarchy = file.channel_hierarchy();

    assert_eq!(hierarchy.len(), 1, "the hierarchy node was not found");
    assert_eq!(hierarchy[0].name, "Powertrain");
    assert_eq!(hierarchy[0].comment, "engine and transmission signals");
    assert!(!hierarchy[0].has_children, "ch_first was zero");
}

#[test]
fn the_ch_next_chain_is_walked_in_order() {
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name_a = f.push(&tx("A"));
    let name_b = f.push(&tx("B"));
    let name_c = f.push(&tx("C"));

    // Built back to front, so each node knows the offset of its successor.
    let third = f.push(&ch(0, 0, name_c, 0, &[], 0));
    let second = f.push(&ch(third, 0, name_b, 0, &[], 0));
    let first = f.push(&ch(second, 0, name_a, 0, &[], 0));
    f.patch_link(hd_link(HD_CH), first);

    let file = f.open("ch_chain").expect("synthetic file should open");
    let names: Vec<&str> = file
        .channel_hierarchy()
        .iter()
        .map(|n| n.name.as_str())
        .collect();

    assert_eq!(
        names,
        ["A", "B", "C"],
        "the whole sibling chain should be walked, in order"
    );
}

#[test]
fn has_children_reflects_whether_ch_first_is_set() {
    // The parser descends `ch_first`: a parent's children arrive under the
    // parent, not as top-level siblings. `Leaf` is the parent's *next*
    // sibling, so it stays at the top level — proving the sibling walk and
    // the descent do not confuse each other.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let child_name = f.push(&tx("Child"));
    let child = f.push(&ch(0, 0, child_name, 0, &[], 0));

    let leaf_name = f.push(&tx("Leaf"));
    let leaf = f.push(&ch(0, 0, leaf_name, 0, &[], 0));

    let parent_name = f.push(&tx("Parent"));
    let parent = f.push(&ch(leaf, child, parent_name, 0, &[], 0));
    f.patch_link(hd_link(HD_CH), parent);

    let file = f.open("ch_children").expect("synthetic file should open");
    let hierarchy = file.channel_hierarchy();

    assert_eq!(hierarchy.len(), 2, "the sibling chain is walked at the top");

    let parent_node = hierarchy.iter().find(|n| n.name == "Parent").unwrap();
    assert!(parent_node.has_children, "ch_first was non-zero");
    assert_eq!(
        parent_node
            .children
            .iter()
            .map(|n| n.name.as_str())
            .collect::<Vec<_>>(),
        ["Child"],
        "the child arrives under its parent"
    );
    assert!(!parent_node.children[0].has_children);

    let leaf_node = hierarchy.iter().find(|n| n.name == "Leaf").unwrap();
    assert!(!leaf_node.has_children, "ch_first was zero");
    assert!(leaf_node.children.is_empty());
}

#[test]
fn a_hierarchy_cycle_between_levels_is_visited_once_not_forever() {
    // A corrupted file can link a child's ch_next back at an ancestor.
    // Per-level cycle detection sees each level's own chain as clean, so
    // only a visited set spanning the recursion stops it. This test would
    // hang against a reader without one.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let child_name = f.push(&tx("Child"));
    let child = f.push(&ch(0, 0, child_name, 0, &[], 0));

    let parent_name = f.push(&tx("Parent"));
    let parent = f.push(&ch(0, child, parent_name, 0, &[], 0));
    f.patch_link(hd_link(HD_CH), parent);
    // The child's ch_next points back at its own ancestor.
    f.patch_link(child + HEADER as u64, parent);

    let file = f.open("ch_cycle").expect("synthetic file should open");
    let hierarchy = file.channel_hierarchy();

    assert_eq!(hierarchy.len(), 1);
    assert_eq!(hierarchy[0].children.len(), 1, "the child is visited once");
    assert!(
        hierarchy[0].children[0].children.is_empty(),
        "the link back at the ancestor is dropped, not followed"
    );
}

#[test]
fn the_element_triples_arrive_as_ch_elements_with_the_right_values() {
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Elements"));
    let node = f.push(&ch(0, 0, name, 0, &[(10, 20, 30), (40, 50, 60)], 0));
    f.patch_link(hd_link(HD_CH), node);

    let file = f.open("ch_elements").expect("synthetic file should open");
    let hierarchy = file.channel_hierarchy();

    assert_eq!(hierarchy.len(), 1);
    assert_eq!(
        hierarchy[0].elements,
        vec![
            falcon_mdf::blocks::ChElement {
                data_group: 10,
                channel_group: 20,
                channel: 30
            },
            falcon_mdf::blocks::ChElement {
                data_group: 40,
                channel_group: 50,
                channel: 60
            },
        ]
    );
}

#[test]
fn channels_matching_filters_by_predicate_in_documented_order() {
    // The GUI plan's second API finding: exact-match find_channels forced a
    // substring search to pull the whole name list and filter client-side.
    // channels_matching takes the predicate instead, and its order — name,
    // then position — is the guarantee a UI list relies on.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let master_name = f.push(&tx("Time"));
    let speed_name = f.push(&tx("Speed"));
    let setpoint_name = f.push(&tx("SpeedSetpoint"));

    let setpoint = f.push(&cn_named(0, setpoint_name, 0, 16, 64));
    let speed = f.push(&cn_named(setpoint, speed_name, 0, 8, 64));
    let master = f.push(&cn_named(speed, master_name, 2, 0, 64));
    let group = f.push(&cg(master, 3, 24));

    let mut records = Vec::new();
    for i in 0..3u64 {
        records.extend_from_slice(&(i as f64).to_le_bytes());
        records.extend_from_slice(&(i as f64 * 2.0).to_le_bytes());
        records.extend_from_slice(&(i as f64 * 4.0).to_le_bytes());
    }
    let data_block = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data_block, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f
        .open("channels_matching")
        .expect("synthetic file should open");

    fn names<'a>(chs: &[&'a falcon_mdf::Channel]) -> Vec<&'a str> {
        chs.iter().map(|c| c.name.as_str()).collect()
    }

    assert_eq!(
        names(&file.channels_matching(|n| n.contains("Spee"))),
        ["Speed", "SpeedSetpoint"],
        "substring search, sorted by name"
    );
    assert_eq!(names(&file.channels_matching(|n| n == "Time")), ["Time"]);
    assert!(
        file.channels_matching(|n| n.contains("absent")).is_empty(),
        "nothing matches, nothing returns"
    );

    // A bare &Channel carries its own sample count — the same corrected
    // number the group reports, not the raw declared one.
    for ch in file.channels_matching(|_| true) {
        assert_eq!(ch.sample_count, 3, "{} carries the group's count", ch.name);
    }
}

#[test]
fn a_hierarchy_element_resolves_to_the_channel_it_locates() {
    // G4. A `ChElement` carries block offsets, and nothing outside the
    // crate could map them back to a channel — the GUI would have had to
    // draw offsets as names. `channel_at` is the resolver, and it must
    // refuse a triple no block carries rather than guess at a neighbour.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let master_name = f.push(&tx("Time"));
    let data_name = f.push(&tx("Data"));
    let data = f.push(&cn_named(0, data_name, 0, 8, 64));
    let master = f.push(&cn_named(data, master_name, 2, 0, 64));
    let group = f.push(&cg(master, 2, 16));

    let mut records = Vec::new();
    for i in 0..2u64 {
        records.extend_from_slice(&(i as f64).to_le_bytes());
        records.extend_from_slice(&(i as f64 * 2.0).to_le_bytes());
    }
    let data_block = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data_block, 0));
    f.patch_link(hd_link(0), group_block);

    let node_name = f.push(&tx("Signals"));
    let node = f.push(&ch(0, 0, node_name, 0, &[(group_block, group, data)], 0));
    f.patch_link(hd_link(HD_CH), node);

    let file = f.open("ch_resolve").expect("synthetic file should open");
    let hierarchy = file.channel_hierarchy();
    assert_eq!(hierarchy.len(), 1);
    let element = &hierarchy[0].elements[0];

    let resolved = file.channel_at(element).expect("the element resolves");
    assert_eq!(resolved.name, "Data");

    let dangling = falcon_mdf::blocks::ChElement {
        data_group: element.data_group,
        channel_group: element.channel_group,
        channel: element.channel + 1,
    };
    assert!(
        file.channel_at(&dangling).is_none(),
        "a triple no channel carries resolves to nothing"
    );
}

#[test]
fn ch_type_maps_to_the_right_variant() {
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name_tree = f.push(&tx("Tree"));
    let name_plain = f.push(&tx("Plain"));
    let name_unknown = f.push(&tx("Unknown"));

    // Built back to front, so each node knows the offset of its successor.
    let unknown = f.push(&ch(0, 0, name_unknown, 0, &[], 42));
    let plain = f.push(&ch(unknown, 0, name_plain, 0, &[], 1));
    let tree = f.push(&ch(plain, 0, name_tree, 0, &[], 0));
    f.patch_link(hd_link(HD_CH), tree);

    let file = f.open("ch_type").expect("synthetic file should open");
    let hierarchy = file.channel_hierarchy();

    assert_eq!(hierarchy.len(), 3);
    assert_eq!(
        hierarchy[0].hierarchy_type,
        falcon_mdf::blocks::ChType::Tree
    );
    assert_eq!(
        hierarchy[1].hierarchy_type,
        falcon_mdf::blocks::ChType::Plain
    );
    assert_eq!(
        hierarchy[2].hierarchy_type,
        falcon_mdf::blocks::ChType::Unknown(42)
    );
}
