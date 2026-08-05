//! The CSV export's format, pinned byte for byte.
//!
//! The expected texts below are exactly what the `export_to_csv` example
//! produced for these channels before the example and the GUI were unified
//! on `write_csv` — captured, not derived, so the test fails if the shared
//! function ever drifts from the format the example established. The
//! multi-column case has no pre-existing artefact to capture; its expected
//! text is hand-derived from the same two single-column exports.

use falcon_mdf::Mf4File;

const FILE: &str = "test_data/reference/dSPACE_LinearConversion.mf4";

#[test]
fn a_single_channel_export_is_byte_identical_to_the_example() {
    let file = Mf4File::open(FILE).expect("reference file opens");
    let channel = file
        .find_channel("Signal_LinearConversion")
        .expect("channel");

    let mut out = Vec::new();
    falcon_mdf::write_csv(&file, &[channel], &mut out).expect("export");

    assert_eq!(
        String::from_utf8(out).expect("csv is text"),
        "Time [],Signal_LinearConversion\n\
         0.000000000,0.000000000\n\
         0.001000000,3.280950000\n\
         0.002000000,6.561900000\n\
         0.003000000,9.842850000\n\
         0.004000000,13.123800000\n"
    );
}

#[test]
fn exporting_the_master_uses_its_own_times() {
    let file = Mf4File::open(FILE).expect("reference file opens");
    let master = file.find_channel("XAxis").expect("master");

    let mut out = Vec::new();
    falcon_mdf::write_csv(&file, &[master], &mut out).expect("export");

    assert_eq!(
        String::from_utf8(out).expect("csv is text"),
        "Time [],XAxis\n\
         0.000000000,0.000000000\n\
         0.001000000,0.001000000\n\
         0.002000000,0.002000000\n\
         0.003000000,0.003000000\n\
         0.004000000,0.004000000\n"
    );
}

#[test]
fn several_channels_share_the_first_channels_time_column() {
    let file = Mf4File::open(FILE).expect("reference file opens");
    let signal = file
        .find_channel("Signal_LinearConversion")
        .expect("channel");
    let master = file.find_channel("XAxis").expect("master");

    let mut out = Vec::new();
    falcon_mdf::write_csv(&file, &[signal, master], &mut out).expect("export");

    assert_eq!(
        String::from_utf8(out).expect("csv is text"),
        "Time [],Signal_LinearConversion,XAxis\n\
         0.000000000,0.000000000,0.000000000\n\
         0.001000000,3.280950000,0.001000000\n\
         0.002000000,6.561900000,0.002000000\n\
         0.003000000,9.842850000,0.003000000\n\
         0.004000000,13.123800000,0.004000000\n"
    );
}

#[test]
fn exporting_nothing_writes_nothing() {
    let file = Mf4File::open(FILE).expect("reference file opens");

    let mut out = Vec::new();
    falcon_mdf::write_csv(&file, &[], &mut out).expect("export");
    assert!(out.is_empty());
}

mod builder {
    //! The smallest file builder that can name a channel badly, duplicated
    //! here for the same reason the other test files carry their own: each
    //! acceptance test stays self-contained.
    use super::Mf4File;

    const HEADER: usize = 24;

    pub struct FileBuilder {
        bytes: Vec<u8>,
    }

    impl FileBuilder {
        pub fn new() -> Self {
            let mut bytes = vec![0u8; 64];
            bytes[0..8].copy_from_slice(b"MDF     ");
            bytes[8..16].copy_from_slice(b"4.11    ");
            bytes[16..24].copy_from_slice(b"falcon  ");
            bytes[28..30].copy_from_slice(&411u16.to_le_bytes());
            Self { bytes }
        }

        pub fn push(&mut self, block: &[u8]) -> u64 {
            let at = self.bytes.len() as u64;
            self.bytes.extend_from_slice(block);
            at
        }

        pub fn patch_link(&mut self, at: u64, value: u64) {
            let at = at as usize;
            self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }

        pub fn open(&self, name: &str) -> falcon_mdf::Result<Mf4File> {
            let path = std::env::temp_dir().join(format!("falcon_mdf_export_{name}.mf4"));
            std::fs::write(&path, &self.bytes).expect("write temp file");
            let result = Mf4File::open(&path);
            let _ = std::fs::remove_file(&path);
            result
        }
    }

    pub fn block(id: &[u8; 4], links: &[u64], data: &[u8]) -> Vec<u8> {
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

    pub fn tx(text: &str) -> Vec<u8> {
        let mut data = text.as_bytes().to_vec();
        data.push(0);
        while !data.len().is_multiple_of(8) {
            data.push(0);
        }
        block(b"##TX", &[], &data)
    }

    pub fn hd() -> Vec<u8> {
        let mut data = vec![0u8; 32];
        data[0..8].copy_from_slice(&1_600_000_000_000_000_000u64.to_le_bytes());
        block(b"##HD", &[0; 6], &data)
    }

    pub fn cn_f64(next: u64, name: u64, channel_type: u8, byte_offset: u32) -> Vec<u8> {
        let mut d = vec![0u8; 72];
        d[0] = channel_type;
        d[2] = 4; // FloatLe
        d[4..8].copy_from_slice(&byte_offset.to_le_bytes());
        d[8..12].copy_from_slice(&64u32.to_le_bytes());
        block(b"##CN", &[next, 0, name, 0, 0, 0, 0, 0], &d)
    }

    pub fn cg(cn_first: u64, cycle_count: u64, data_bytes: u32) -> Vec<u8> {
        let mut d = vec![0u8; 32];
        d[8..16].copy_from_slice(&cycle_count.to_le_bytes());
        d[24..28].copy_from_slice(&data_bytes.to_le_bytes());
        block(b"##CG", &[0, cn_first, 0, 0, 0, 0], &d)
    }

    pub fn dg(cg_first: u64, data: u64) -> Vec<u8> {
        block(b"##DG", &[0, cg_first, data, 0], &[0u8; 8])
    }

    pub fn dt(records: &[u8]) -> Vec<u8> {
        block(b"##DT", &[], records)
    }
}

#[test]
fn names_with_commas_and_quotes_are_escaped_per_rfc_4180() {
    // A bus-signal name like `Boost, psi` is ordinary file-supplied text;
    // unescaped, it would become two columns in every spreadsheet the export
    // opens in. RFC 4180 quoting keeps it one field, and plain names stay
    // byte-identical to the unquoted format the other tests pin.
    use builder::*;

    let mut f = FileBuilder::new();
    f.push(&hd());

    let master_name = f.push(&tx("Time"));
    let comma_name = f.push(&tx("Boost, psi"));
    let quote_name = f.push(&tx("Say \"hi\""));

    let quote_ch = f.push(&cn_f64(0, quote_name, 0, 16));
    let comma_ch = f.push(&cn_f64(quote_ch, comma_name, 0, 8));
    let master = f.push(&cn_f64(comma_ch, master_name, 2, 0));
    let group = f.push(&cg(master, 2, 24));

    let mut records = Vec::new();
    for i in 0..2u64 {
        records.extend_from_slice(&(i as f64).to_le_bytes());
        records.extend_from_slice(&(i as f64 * 1.5).to_le_bytes());
        records.extend_from_slice(&(i as f64 * 2.5).to_le_bytes());
    }
    let data_block = f.push(&dt(&records));
    let group_block = f.push(&dg(group, data_block));
    f.patch_link(64 + 24, group_block);

    let file = f.open("quoting").expect("synthetic file opens");
    let channels: Vec<&falcon_mdf::Channel> =
        vec![file.find_channel("Boost, psi").expect("comma channel")];
    let quoted: Vec<&falcon_mdf::Channel> =
        vec![file.find_channel("Say \"hi\"").expect("quote channel")];

    let mut out = Vec::new();
    falcon_mdf::write_csv(&file, &channels, &mut out).expect("export");
    assert_eq!(
        String::from_utf8(out).expect("csv is text"),
        "Time [],\"Boost, psi\"\n0.000000000,0.000000000\n1.000000000,1.500000000\n"
    );

    let mut out = Vec::new();
    falcon_mdf::write_csv(&file, &quoted, &mut out).expect("export");
    assert_eq!(
        String::from_utf8(out).expect("csv is text"),
        "Time [],\"Say \"\"hi\"\"\"\n0.000000000,0.000000000\n1.000000000,2.500000000\n"
    );
}
