//! Channel Array (CA) block parsing.
//!
//! A CA block describes the layout of an array channel — a channel whose
//! record field holds many values per sample rather than one. The parent
//! channel is always of type `ByteArray`; the CA block names a *template*
//! CN block that describes the individual elements, and a set of dimensions
//! that give the array's shape.
//!
//! ## Storage forms
//!
//! MF4 defines three, named for the block that acts as the template:
//!
//! - **CN template** (`ca_storage = 0`): all elements of one sample's array are
//!   stored adjacently in the record, described by one template CN.
//! - **CG template** (`ca_storage = 1`): each element lives in its own channel
//!   group.
//! - **DG template** (`ca_storage = 2`): each element lives in its own data
//!   group.
//!
//! All three storage forms are decoded. A CN-template array reads adjacent
//! record fields; CG- and DG-template arrays gather elements across member
//! channel groups or data groups named in the CA block's link list.
//!
//! A CN-template array's dimensions can still vary per sample (`flags.dynamic_size`):
//! `ca_dim_size` is then only the largest shape any sample may take, and the
//! real count comes from a channel named by `ca_dynamic_size`. A single
//! dynamic dimension, sized by a channel in the same record, is decoded; more
//! than one, or a sizing channel elsewhere, stays unreadable for the same
//! reason as above — nothing here gathers bytes from another record stream.
//!
//! A look-up array's `ca_composition` can also name another CA block rather
//! than a template CN — an array whose elements are themselves arrays. That
//! chain is followed and its dimensions combined; see `Mf4File::expand_composition_channels`.
//!
//! ## Layout
//!
//! Most of this block is optional, and `ca_flags` is what says which parts are
//! there — so the flag bits decide how the link list is partitioned, not just
//! what the block means. Both the link order and the flag numbering are set out
//! at [`CaFlags`] and in `CaBlock::parse`.

use crate::blocks::common::{read_links, BlockHeader, ParseBlock, BLOCK_HEADER_SIZE};
use crate::error::{Mf4Error, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

/// Array type, from the CA block's `ca_type` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaArrayType {
    /// A full array channel (`ca_type = 0`). The parent CN is `ByteArray`.
    Array,
    /// A scale axis describing one dimension of an array (`ca_type = 1`).
    ScaleAxis,
    /// A look-up table: an array read through its axes (`ca_type = 2`).
    Lookup,
    /// Unknown array type code.
    Unknown(u8),
}

impl CaArrayType {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => CaArrayType::Array,
            1 => CaArrayType::ScaleAxis,
            2 => CaArrayType::Lookup,
            v => CaArrayType::Unknown(v),
        }
    }
}

/// Where an array's elements are stored, named for the block that templates
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaStorage {
    /// All elements of one sample's array are adjacent in the record, described
    /// by one template CN (`ca_storage = 0`). The common form.
    CnTemplate,
    /// Each element lives in its own channel group (`ca_storage = 1`).
    CgTemplate,
    /// Each element lives in its own data group (`ca_storage = 2`).
    DgTemplate,
    /// Unknown storage code.
    Unknown(u8),
}

impl CaStorage {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => CaStorage::CnTemplate,
            1 => CaStorage::CgTemplate,
            2 => CaStorage::DgTemplate,
            v => CaStorage::Unknown(v),
        }
    }
}

/// CA block flags.
///
/// Each flag decides whether a section of the link list is present, so the bit
/// numbering is not cosmetic: reading one wrongly shifts everything after it.
/// The previous numbering here started at "has axis" and invented a flag for
/// axis names — see B23.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CaFlags {
    /// The size of each dimension varies per sample, given by another channel
    /// (bit 0). The sizes in `ca_dim_size` are then maxima, not the shape.
    pub dynamic_size: bool,
    /// Each dimension names an input quantity channel (bit 1).
    pub input_quantity: bool,
    /// The array names an output quantity channel (bit 2).
    pub output_quantity: bool,
    /// The array names a comparison quantity channel (bit 3).
    pub comparison_quantity: bool,
    /// The array has axes (bit 4).
    pub axis: bool,
    /// The axis values are fixed and stored in this block rather than in a
    /// channel (bit 5).
    pub fixed_axis: bool,
    /// Elements are stored in reverse dimension order (bit 6).
    pub inverse_layout: bool,
    /// Axis intervals are open on the left (bit 7).
    pub left_open_interval: bool,
    /// The axis is the standard axis of the look-up (bit 8).
    pub standard_axis: bool,
}

impl CaFlags {
    fn from_u32(value: u32) -> Self {
        CaFlags {
            dynamic_size: (value & 0x0001) != 0,
            input_quantity: (value & 0x0002) != 0,
            output_quantity: (value & 0x0004) != 0,
            comparison_quantity: (value & 0x0008) != 0,
            axis: (value & 0x0010) != 0,
            fixed_axis: (value & 0x0020) != 0,
            inverse_layout: (value & 0x0040) != 0,
            left_open_interval: (value & 0x0080) != 0,
            standard_axis: (value & 0x0100) != 0,
        }
    }
}

/// Where an axis channel lives: a data group, a channel group within it, and a
/// channel within that.
///
/// The standard locates an axis with all three, because an axis need not be in
/// the same group — or even the same data group — as the array it describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisRef {
    /// Link to the data group holding the axis channel.
    pub dg: u64,
    /// Link to the channel group within that data group.
    pub cg: u64,
    /// Link to the axis channel itself.
    pub cn: u64,
}

/// The Channel Array (CA) block.
///
/// Describes the shape and element layout of an array channel. The parent
/// channel's `composition` link points here.
#[derive(Debug, Clone)]
pub struct CaBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Link to the template CN block describing one element. Always the first
    /// link, whatever `ca_type` is.
    pub ca_composition: u64,
    /// Links to member CG blocks (for CG template) or DG blocks (for DG template),
    /// one per array element.
    pub ca_data: Vec<u64>,
    /// Links to axis conversion (CC) blocks, one per dimension. Present only
    /// when `flags.axis` is set.
    pub ca_axis_conversion: Vec<u64>,
    /// Where each dimension's axis channel lives, one per dimension. Present
    /// only when `flags.axis` is set and `flags.fixed_axis` is not — a fixed
    /// axis stores its values in `ca_axis_values` instead.
    pub ca_axis: Vec<AxisRef>,
    /// Array type.
    pub ca_type: CaArrayType,
    /// Storage form.
    pub ca_storage: CaStorage,
    /// Number of dimensions.
    pub ca_ndim: u16,
    /// CA flags.
    pub flags: CaFlags,
    /// Base byte offset applied to each array element.
    pub ca_byte_offset_base: i32,
    /// Base invalidation-bit position applied to each array element.
    pub ca_invalidation_bit_base: u32,
    /// Number of elements per dimension. A maximum rather than the shape when
    /// `flags.dynamic_size` is set.
    pub ca_dim_size: Vec<u64>,
    /// Fixed axis values, if present. Laid out as consecutive segments
    /// whose lengths are `ca_dim_size[i]`.
    pub ca_axis_values: Vec<f64>,
    /// Where each dimension's real, per-sample size is stored, one per
    /// dimension. Present only when `flags.dynamic_size` is set; `ca_dim_size`
    /// is then only the largest shape a sample may take, not the shape any
    /// sample has.
    pub ca_dynamic_size: Vec<AxisRef>,
}

impl CaBlock {
    /// Minimum size of the CA block data section (type, storage, ndim, flags).
    /// Size of the fixed part of the data section: type and storage as single
    /// bytes, a `u16` dimension count, a `u32` flags word, a signed byte-offset
    /// base and an unsigned invalidation-bit base.
    pub const MIN_DATA_SIZE: usize = 1 + 1 + 2 + 4 + 4 + 4;

    /// Returns the total number of elements in one sample's array — the
    /// product of all dimension sizes.
    pub fn total_elements(&self) -> u64 {
        self.ca_dim_size.iter().copied().product()
    }

    /// Returns the shape as a slice of dimension sizes.
    pub fn shape(&self) -> &[u64] {
        &self.ca_dim_size
    }

    /// Returns the file offset of the template CN block, or 0 when absent.
    pub fn template_offset(&self) -> u64 {
        self.ca_composition
    }
}

impl ParseBlock for CaBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##CA", offset)?;

        // The link section is variable-length: its layout depends on ca_type
        // and ca_flags, which are in the data section. We know link_count
        // from the header, so read all links first, then partition them.
        let links_start = BLOCK_HEADER_SIZE;
        let all_links = read_links(data, links_start, header.link_count as usize)?;

        // Data section starts after the header and all links.
        let data_start = header.data_offset();
        let data_section = data
            .get(data_start..)
            .ok_or_else(|| Mf4Error::truncated(offset, data_start, data.len()))?;

        if data_section.len() < Self::MIN_DATA_SIZE {
            return Err(Mf4Error::truncated(
                offset,
                Self::MIN_DATA_SIZE,
                data_section.len(),
            ));
        }

        let mut cursor = Cursor::new(data_section);
        // Widths per the standard: the dimension count is a `u16` and the flags
        // a `u32`. Reading them a byte and two bytes narrow, as this parser once
        // did, misreads the count and leaves the flags and everything after
        // them shifted.
        let ca_type = CaArrayType::from_u8(cursor.read_u8()?);
        let ca_storage = CaStorage::from_u8(cursor.read_u8()?);
        let ca_ndim = cursor.read_u16::<LittleEndian>()?;
        let flags_raw = cursor.read_u32::<LittleEndian>()?;
        let flags = CaFlags::from_u32(flags_raw);
        let ca_byte_offset_base = cursor.read_i32::<LittleEndian>()?;
        let ca_invalidation_bit_base = cursor.read_u32::<LittleEndian>()?;

        // Partition the links. The order is:
        //   ca_composition                              1 link, always
        //   ca_data[prod(dim_size)]                     if storage is CG or DG template
        //   ca_dynamic_size[ndim]      as triples       if flags.dynamic_size
        //   ca_input_quantity[ndim]    as triples       if flags.input_quantity
        //   ca_output_quantity         one triple       if flags.output_quantity
        //   ca_comparison_quantity     one triple       if flags.comparison_quantity
        //   ca_axis_conversion[ndim]   1 link each      if flags.axis
        //   ca_axis[ndim]              as triples       if flags.axis && !fixed_axis
        //
        // The dimension count and sizes decide how many links several of these
        // sections hold, so the data section has to be read far enough to know
        // them before the links can be partitioned at all.
        if ca_ndim == 0 {
            return Err(Mf4Error::invalid_block_size("CA", ca_ndim as u64, 1));
        }

        let mut ca_dim_size = Vec::with_capacity(ca_ndim as usize);
        for _ in 0..ca_ndim {
            ca_dim_size.push(cursor.read_u64::<LittleEndian>()?);
        }

        let ndim = ca_ndim as usize;
        let mut link_idx = 0usize;
        let ca_composition = *all_links.first().unwrap_or(&0);
        link_idx += 1;

        // Sections this reader does not act on are still counted: skipping one
        // by the wrong width hands back a dynamic-size link as an axis.
        let mut ca_data = Vec::new();
        if ca_storage == CaStorage::CgTemplate || ca_storage == CaStorage::DgTemplate {
            // One data link per element, so the product of the dimensions.
            let elements = ca_dim_size
                .iter()
                .try_fold(1usize, |acc, &d| acc.checked_mul(d as usize))
                .ok_or_else(|| Mf4Error::invalid_block_size("CA", u64::MAX, 1))?;
            let end = (link_idx + elements).min(all_links.len());
            if link_idx <= all_links.len() {
                ca_data = all_links[link_idx..end].to_vec();
            }
            link_idx = link_idx.saturating_add(elements);
        }
        let mut ca_dynamic_size = Vec::new();
        if flags.dynamic_size {
            for _ in 0..ndim {
                let Some(triple) = all_links.get(link_idx..link_idx + 3) else {
                    break;
                };
                ca_dynamic_size.push(AxisRef {
                    dg: triple[0],
                    cg: triple[1],
                    cn: triple[2],
                });
                link_idx += 3;
            }
        }
        if flags.input_quantity {
            link_idx = link_idx.saturating_add(ndim * 3);
        }
        if flags.output_quantity {
            link_idx = link_idx.saturating_add(3);
        }
        if flags.comparison_quantity {
            link_idx = link_idx.saturating_add(3);
        }

        let mut ca_axis_conversion = Vec::new();
        let mut ca_axis = Vec::new();
        if flags.axis {
            let end = (link_idx + ndim).min(all_links.len());
            ca_axis_conversion = all_links[link_idx.min(end)..end].to_vec();
            link_idx = end;

            if !flags.fixed_axis {
                for _ in 0..ndim {
                    let Some(triple) = all_links.get(link_idx..link_idx + 3) else {
                        break;
                    };
                    ca_axis.push(AxisRef {
                        dg: triple[0],
                        cg: triple[1],
                        cn: triple[2],
                    });
                    link_idx += 3;
                }
            }
        }

        // Fixed axis values: for each dimension i, ca_dim_size[i] f64 values.
        // The only optional part of the data section.
        let mut ca_axis_values = Vec::new();
        if flags.fixed_axis {
            for &dim in &ca_dim_size {
                for _ in 0..dim {
                    ca_axis_values.push(cursor.read_f64::<LittleEndian>()?);
                }
            }
        }

        Ok(CaBlock {
            header,
            ca_composition,
            ca_data,
            ca_axis_conversion,
            ca_axis,
            ca_type,
            ca_storage,
            ca_ndim,
            flags,
            ca_byte_offset_base,
            ca_invalidation_bit_base,
            ca_dim_size,
            ca_axis_values,
            ca_dynamic_size,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `ca_flags` bit values, written out here rather than taken from `CaFlags`
    // so that these tests fail if the implementation's mapping drifts from the
    // standard's. Sharing the constants would make the fixture agree with any
    // numbering the parser happened to use, which is exactly how B23 survived.
    const FLAG_DYNAMIC_SIZE: u32 = 1 << 0;
    const FLAG_INPUT_QUANTITY: u32 = 1 << 1;
    const FLAG_OUTPUT_QUANTITY: u32 = 1 << 2;
    const FLAG_COMPARISON_QUANTITY: u32 = 1 << 3;
    const FLAG_AXIS: u32 = 1 << 4;
    const FLAG_FIXED_AXIS: u32 = 1 << 5;
    const FLAG_INVERSE_LAYOUT: u32 = 1 << 6;
    const FLAG_LEFT_OPEN_INTERVAL: u32 = 1 << 7;
    const FLAG_STANDARD_AXIS: u32 = 1 << 8;

    // Distinct link values per section, so a misattributed link is identifiable
    // by its magnitude rather than merely unequal to what was expected.
    const COMPOSITION: u64 = 1_000;
    const DATA: u64 = 2_000;
    const DYNAMIC_SIZE: u64 = 3_000;
    const INPUT: u64 = 4_000;
    const OUTPUT: u64 = 5_000;
    const COMPARISON: u64 = 6_000;
    const AXIS_CC: u64 = 7_000;
    const AXIS: u64 = 8_000;

    /// Builds a CA block laid out as the standard specifies.
    ///
    /// **Taken from the standard, not from this parser.** The version of this
    /// helper that preceded it derived every optional section from `CaFlags` —
    /// so a parser that misnumbered the flags produced a fixture that
    /// misnumbered them identically, and all six tests below passed against a
    /// block no writer would ever emit. That is B20's lesson, and this file is
    /// where it was ignored a second time (B23).
    ///
    /// Link section, in order:
    /// - `ca_composition`, always, whatever `ca_type` is
    /// - `ca_data[prod(dim_size)]` when storage is the DG template
    /// - `ca_dynamic_size[ndim]`, as (dg, cg, cn) *triples*
    /// - `ca_input_quantity[ndim]`, likewise
    /// - `ca_output_quantity`, one triple
    /// - `ca_comparison_quantity`, one triple
    /// - `ca_axis_conversion[ndim]`, one link each, when the axis flag is set
    /// - `ca_axis[ndim]`, triples, when the axis flag is set and fixed axis is not
    ///
    /// Data section: the fixed 16 bytes, `ca_dim_size[ndim]`, then — only when
    /// the fixed-axis flag is set — `sum(dim_size)` axis values.
    fn create_ca_block(
        ca_type: u8,
        ca_storage: u8,
        ndim: u16,
        flags: u32,
        dim_sizes: &[u64],
    ) -> Vec<u8> {
        let mut links: Vec<u64> = vec![COMPOSITION];

        if ca_storage == 1 || ca_storage == 2 {
            let count: u64 = dim_sizes.iter().product();
            for i in 0..count {
                links.push(DATA + i);
            }
        }
        if flags & FLAG_DYNAMIC_SIZE != 0 {
            for i in 0..ndim as u64 {
                links.extend([
                    DYNAMIC_SIZE + i * 10,
                    DYNAMIC_SIZE + i * 10 + 1,
                    DYNAMIC_SIZE + i * 10 + 2,
                ]);
            }
        }
        if flags & FLAG_INPUT_QUANTITY != 0 {
            for i in 0..ndim as u64 {
                links.extend([INPUT + i * 10, INPUT + i * 10 + 1, INPUT + i * 10 + 2]);
            }
        }
        if flags & FLAG_OUTPUT_QUANTITY != 0 {
            links.extend([OUTPUT, OUTPUT + 1, OUTPUT + 2]);
        }
        if flags & FLAG_COMPARISON_QUANTITY != 0 {
            links.extend([COMPARISON, COMPARISON + 1, COMPARISON + 2]);
        }
        if flags & FLAG_AXIS != 0 {
            for i in 0..ndim as u64 {
                links.push(AXIS_CC + i * 100);
            }
            if flags & FLAG_FIXED_AXIS == 0 {
                for i in 0..ndim as u64 {
                    links.extend([AXIS + i * 10, AXIS + i * 10 + 1, AXIS + i * 10 + 2]);
                }
            }
        }

        let link_count = links.len() as u64;
        let links_bytes: Vec<u8> = links.iter().flat_map(|l| l.to_le_bytes()).collect();

        let mut data_section: Vec<u8> = Vec::new();
        data_section.push(ca_type);
        data_section.push(ca_storage);
        data_section.extend_from_slice(&ndim.to_le_bytes());
        data_section.extend_from_slice(&flags.to_le_bytes());
        data_section.extend_from_slice(&0i32.to_le_bytes()); // byte offset base
        data_section.extend_from_slice(&0u32.to_le_bytes()); // invalidation base
        for &d in dim_sizes {
            data_section.extend_from_slice(&d.to_le_bytes());
        }
        if flags & FLAG_FIXED_AXIS != 0 {
            for &d in dim_sizes {
                for j in 0..d {
                    data_section.extend_from_slice(&(j as f64).to_le_bytes());
                }
            }
        }

        let total_len = BLOCK_HEADER_SIZE + links_bytes.len() + data_section.len();
        let mut data = vec![0u8; total_len];

        data[0..4].copy_from_slice(b"##CA");
        data[4..8].copy_from_slice(&[0, 0, 0, 0]);
        data[8..16].copy_from_slice(&(total_len as u64).to_le_bytes());
        data[16..24].copy_from_slice(&link_count.to_le_bytes());

        data[BLOCK_HEADER_SIZE..BLOCK_HEADER_SIZE + links_bytes.len()]
            .copy_from_slice(&links_bytes);

        let ds = BLOCK_HEADER_SIZE + links_bytes.len();
        data[ds..ds + data_section.len()].copy_from_slice(&data_section);

        data
    }

    #[test]
    fn the_storage_codes_are_the_ones_the_standard_assigns() {
        // Both independent implementations consulted agree: 0 is the CN
        // template — the ordinary, in-record layout — and 1 and 2 spread the
        // elements across channel and data groups. Reading 0 and 1 the other
        // way round rejects every ordinary array channel and silently
        // misdecodes the one form that is genuinely elsewhere.
        for (code, expected) in [
            (0u8, CaStorage::CnTemplate),
            (1, CaStorage::CgTemplate),
            (2, CaStorage::DgTemplate),
            (3, CaStorage::Unknown(3)),
        ] {
            let data = create_ca_block(0, code, 1, 0, &[2]);
            let ca = CaBlock::parse(&data, 0).unwrap();
            assert_eq!(ca.ca_storage, expected, "ca_storage = {code}");
        }
    }

    #[test]
    fn every_flag_bit_is_the_one_the_standard_assigns() {
        // B23. The whole bit table, pinned individually. The parser previously
        // read bit 0 as "has axis" where the standard has dynamic size, and
        // invented a flag for axis names — so an array declaring an input
        // quantity was read as one declaring a fixed axis, and the parse walked
        // off into the wrong section of the block.
        /// One flag bit, the accessor it should set, and a name for the message.
        type Case = (u32, fn(&CaFlags) -> bool, &'static str);

        let cases: [Case; 9] = [
            (FLAG_DYNAMIC_SIZE, |f| f.dynamic_size, "dynamic size"),
            (FLAG_INPUT_QUANTITY, |f| f.input_quantity, "input quantity"),
            (
                FLAG_OUTPUT_QUANTITY,
                |f| f.output_quantity,
                "output quantity",
            ),
            (
                FLAG_COMPARISON_QUANTITY,
                |f| f.comparison_quantity,
                "comparison quantity",
            ),
            (FLAG_AXIS, |f| f.axis, "axis"),
            (FLAG_FIXED_AXIS, |f| f.fixed_axis, "fixed axis"),
            (FLAG_INVERSE_LAYOUT, |f| f.inverse_layout, "inverse layout"),
            (
                FLAG_LEFT_OPEN_INTERVAL,
                |f| f.left_open_interval,
                "left-open interval",
            ),
            (FLAG_STANDARD_AXIS, |f| f.standard_axis, "standard axis"),
        ];

        for (bit, is_set, name) in cases {
            let ca = CaBlock::parse(&create_ca_block(0, 0, 1, bit, &[2]), 0).unwrap();
            assert!(is_set(&ca.flags), "{name} should be set for {bit:#x}");

            // And nothing else may be: a shifted table sets a neighbour too.
            let ca = CaBlock::parse(&create_ca_block(0, 0, 1, 0, &[2]), 0).unwrap();
            assert!(!is_set(&ca.flags), "{name} should be clear when no flag is");
        }
    }

    #[test]
    fn test_ca_block_array_contiguous() {
        let data = create_ca_block(0, 0, 2, 0, &[3, 4]);
        let ca = CaBlock::parse(&data, 0).unwrap();

        assert_eq!(ca.ca_type, CaArrayType::Array);
        assert_eq!(ca.ca_storage, CaStorage::CnTemplate);
        assert_eq!(ca.ca_ndim, 2);
        assert_eq!(ca.ca_composition, COMPOSITION);
        assert_eq!(ca.ca_dim_size, vec![3, 4]);
        assert_eq!(ca.total_elements(), 12);
        assert_eq!(ca.shape(), &[3, 4]);
        assert!(ca.ca_axis.is_empty(), "no axis flag, so no axis references");
        assert!(ca.ca_axis_conversion.is_empty());
    }

    #[test]
    fn test_ca_block_fixed_axis() {
        // A fixed axis stores its values in the block and therefore carries no
        // axis *channel* references — only the per-dimension conversions.
        let data = create_ca_block(0, 0, 1, FLAG_AXIS | FLAG_FIXED_AXIS, &[3]);
        let ca = CaBlock::parse(&data, 0).unwrap();

        assert!(ca.flags.axis && ca.flags.fixed_axis);
        assert_eq!(ca.ca_dim_size, vec![3]);
        assert_eq!(
            ca.ca_axis_values,
            vec![0.0, 1.0, 2.0],
            "every axis value, not one per dimension"
        );
        assert_eq!(ca.ca_axis_conversion, vec![AXIS_CC]);
        assert!(
            ca.ca_axis.is_empty(),
            "a fixed axis has no channel to point at"
        );
    }

    #[test]
    fn an_axis_that_is_not_fixed_is_a_triple_per_dimension() {
        // The standard locates an axis channel with a (data group, channel
        // group, channel) triple. Reading one link per dimension takes the data
        // group of the first axis and calls it the axis, then reads the rest of
        // that axis's triple as though it belonged to later dimensions.
        let data = create_ca_block(0, 0, 2, FLAG_AXIS, &[3, 4]);
        let ca = CaBlock::parse(&data, 0).unwrap();

        assert_eq!(ca.ca_axis_conversion, vec![AXIS_CC, AXIS_CC + 100]);
        assert_eq!(
            ca.ca_axis,
            vec![
                AxisRef {
                    dg: AXIS,
                    cg: AXIS + 1,
                    cn: AXIS + 2
                },
                AxisRef {
                    dg: AXIS + 10,
                    cg: AXIS + 11,
                    cn: AXIS + 12
                },
            ]
        );
    }

    #[test]
    fn a_dynamic_size_dimension_is_a_triple_naming_its_real_count() {
        // Bit 0 introduces one (dg, cg, cn) triple per dimension, before the
        // input/output/comparison quantities and the axis. These were
        // previously only skipped over to keep the rest of the link section
        // aligned; nothing kept them.
        let data = create_ca_block(0, 0, 2, FLAG_DYNAMIC_SIZE, &[3, 4]);
        let ca = CaBlock::parse(&data, 0).unwrap();

        assert_eq!(
            ca.ca_dynamic_size,
            vec![
                AxisRef {
                    dg: DYNAMIC_SIZE,
                    cg: DYNAMIC_SIZE + 1,
                    cn: DYNAMIC_SIZE + 2
                },
                AxisRef {
                    dg: DYNAMIC_SIZE + 10,
                    cg: DYNAMIC_SIZE + 11,
                    cn: DYNAMIC_SIZE + 12
                },
            ]
        );
    }

    #[test]
    fn the_quantity_links_are_skipped_without_shifting_the_axis() {
        // Every flag before the axis introduces links of its own, and the axis
        // section begins only after all of them. Getting any one wrong hands
        // back a dynamic-size or input-quantity link as an axis.
        let flags = FLAG_DYNAMIC_SIZE
            | FLAG_INPUT_QUANTITY
            | FLAG_OUTPUT_QUANTITY
            | FLAG_COMPARISON_QUANTITY
            | FLAG_AXIS
            | FLAG_FIXED_AXIS;
        let data = create_ca_block(0, 0, 2, flags, &[2, 3]);
        let ca = CaBlock::parse(&data, 0).unwrap();

        assert_eq!(ca.ca_composition, COMPOSITION);
        assert_eq!(ca.ca_axis_conversion, vec![AXIS_CC, AXIS_CC + 100]);
        assert_eq!(ca.ca_axis_values, vec![0.0, 1.0, 0.0, 1.0, 2.0]);
    }

    #[test]
    fn a_cg_template_array_carries_one_cg_link_per_element() {
        let data = create_ca_block(0, 1, 2, FLAG_AXIS | FLAG_FIXED_AXIS, &[2, 3]);
        let ca = CaBlock::parse(&data, 0).unwrap();

        assert_eq!(ca.ca_storage, CaStorage::CgTemplate);
        assert_eq!(
            ca.ca_data,
            vec![DATA, DATA + 1, DATA + 2, DATA + 3, DATA + 4, DATA + 5]
        );
        assert_eq!(
            ca.ca_axis_conversion,
            vec![AXIS_CC, AXIS_CC + 100],
            "six data links must not be mistaken for axis conversions"
        );
    }

    #[test]
    fn a_dg_template_array_reserves_one_data_link_per_element() {
        // Storage 2 puts each element in its own data group, so the link
        // section carries one link per element before anything else.
        let data = create_ca_block(0, 2, 2, FLAG_AXIS | FLAG_FIXED_AXIS, &[2, 3]);
        let ca = CaBlock::parse(&data, 0).unwrap();

        assert_eq!(ca.ca_storage, CaStorage::DgTemplate);
        assert_eq!(
            ca.ca_data,
            vec![DATA, DATA + 1, DATA + 2, DATA + 3, DATA + 4, DATA + 5]
        );
        assert_eq!(
            ca.ca_axis_conversion,
            vec![AXIS_CC, AXIS_CC + 100],
            "six data links must not be mistaken for axis conversions"
        );
    }

    #[test]
    fn a_scale_axis_block_still_carries_its_composition_link_first() {
        // `ca_composition` is link zero whatever `ca_type` says. The parser
        // used to suppress it for a scale axis and read the following link as
        // the composition of nothing.
        let data = create_ca_block(1, 0, 1, 0, &[5]);
        let ca = CaBlock::parse(&data, 0).unwrap();

        assert_eq!(ca.ca_type, CaArrayType::ScaleAxis);
        assert_eq!(ca.ca_composition, COMPOSITION);
    }

    #[test]
    fn the_array_type_codes_are_the_ones_the_standard_assigns() {
        for (code, expected) in [
            (0u8, CaArrayType::Array),
            (1, CaArrayType::ScaleAxis),
            (2, CaArrayType::Lookup),
            (3, CaArrayType::Unknown(3)),
        ] {
            let ca = CaBlock::parse(&create_ca_block(code, 0, 1, 0, &[2]), 0).unwrap();
            assert_eq!(ca.ca_type, expected, "ca_type = {code}");
        }
    }

    #[test]
    fn test_ca_block_zero_dims_rejected() {
        let data = create_ca_block(0, 1, 0, 0, &[]);
        let result = CaBlock::parse(&data, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_ca_block_invalid_type_rejected() {
        let mut data = create_ca_block(0, 1, 1, 0, &[3]);
        data[0..4].copy_from_slice(b"##XX");
        let result = CaBlock::parse(&data, 0);
        assert!(matches!(result, Err(Mf4Error::InvalidBlockId { .. })));
    }
}
