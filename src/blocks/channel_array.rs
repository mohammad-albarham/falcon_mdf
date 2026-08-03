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
//!   stored adjacently in the record, described by one template CN. This is the
//!   common form, and the one this implementation decodes.
//! - **CG template** (`ca_storage = 1`): each element lives in its own channel
//!   group.
//! - **DG template** (`ca_storage = 2`): each element lives in its own data
//!   group.
//!
//! The latter two spread one sample's elements across several record streams,
//! which nothing here gathers. A channel using either stays unreadable with a
//! diagnostic reason rather than being silently decoded as though its elements
//! were adjacent.

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
    /// A type template — defines element layout without being a channel
    /// itself (`ca_type = 2`).
    TypeTemplate,
    /// Fixed-length array (`ca_type = 3`).
    FixedLength,
    /// Unknown array type code.
    Unknown(u8),
}

impl CaArrayType {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => CaArrayType::Array,
            1 => CaArrayType::ScaleAxis,
            2 => CaArrayType::TypeTemplate,
            3 => CaArrayType::FixedLength,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CaFlags {
    /// The array has an axis (bit 0).
    pub has_axis: bool,
    /// The axis values are fixed and stored in the CA block (bit 1).
    pub fixed_axis: bool,
    /// Each axis has a conversion (CC) block (bit 2).
    pub axis_conversion: bool,
    /// Each axis has a name (TX) block (bit 3).
    pub axis_name: bool,
    /// Inverse layout (bit 4).
    pub inverse_layout: bool,
    /// Precomputed minimum values are present (bit 5).
    pub precomputed_min: bool,
    /// Precomputed maximum values are present (bit 6).
    pub precomputed_max: bool,
}

impl CaFlags {
    fn from_u32(value: u32) -> Self {
        CaFlags {
            has_axis: (value & 0x0001) != 0,
            fixed_axis: (value & 0x0002) != 0,
            axis_conversion: (value & 0x0004) != 0,
            axis_name: (value & 0x0008) != 0,
            inverse_layout: (value & 0x0010) != 0,
            precomputed_min: (value & 0x0020) != 0,
            precomputed_max: (value & 0x0040) != 0,
        }
    }
}

/// The Channel Array (CA) block.
///
/// Describes the shape and element layout of an array channel. The parent
/// channel's `composition` link points here.
#[derive(Debug, Clone)]
pub struct CaBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Link to the template CN block (for `Array` and `TypeTemplate` types).
    /// Zero for `ScaleAxis`.
    pub ca_composition: u64,
    /// Links to scale-axis CN blocks, one per dimension.
    pub ca_scale_axis: Vec<u64>,
    /// Links to axis conversion (CC) blocks, one per dimension.
    /// Present only when `flags.axis_conversion` is set.
    pub ca_axis_cc: Vec<u64>,
    /// Links to axis name (TX) blocks, one per dimension.
    /// Present only when `flags.axis_name` is set.
    pub ca_axis_name: Vec<u64>,
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
    /// Number of elements per dimension.
    pub ca_dim_size: Vec<u64>,
    /// Precomputed minimum per dimension, if present.
    pub ca_precomputed_min: Vec<f64>,
    /// Precomputed maximum per dimension, if present.
    pub ca_precomputed_max: Vec<f64>,
    /// Fixed axis values, if present. Laid out as consecutive segments
    /// whose lengths are `ca_dim_size[i]`.
    pub ca_axis_values: Vec<f64>,
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
        //   ca_composition (1, if Array or TypeTemplate)
        //   ca_scale_axis[ndim] (if Array or ScaleAxis)
        //   ca_axis_cc[ndim] (if flags.axis_conversion)
        //   ca_axis_name[ndim] (if flags.axis_name)
        let needs_composition = matches!(ca_type, CaArrayType::Array | CaArrayType::TypeTemplate);
        let needs_scale_axis = matches!(ca_type, CaArrayType::Array | CaArrayType::ScaleAxis);

        let mut link_idx = 0usize;
        let ca_composition = if needs_composition {
            let v = *all_links.get(link_idx).unwrap_or(&0);
            link_idx += 1;
            v
        } else {
            0
        };

        let ca_scale_axis = if needs_scale_axis {
            let end = (link_idx + ca_ndim as usize).min(all_links.len());
            let v = all_links[link_idx..end].to_vec();
            link_idx = end;
            v
        } else {
            Vec::new()
        };

        let ca_axis_cc = if flags.axis_conversion {
            let end = (link_idx + ca_ndim as usize).min(all_links.len());
            let v = all_links[link_idx..end].to_vec();
            link_idx = end;
            v
        } else {
            Vec::new()
        };

        let ca_axis_name = if flags.axis_name {
            let end = (link_idx + ca_ndim as usize).min(all_links.len());
            all_links[link_idx..end].to_vec()
        } else {
            Vec::new()
        };

        // Read dimension sizes.
        if ca_ndim == 0 {
            return Err(Mf4Error::invalid_block_size("CA", ca_ndim as u64, 1));
        }

        let mut ca_dim_size = Vec::with_capacity(ca_ndim as usize);
        for _ in 0..ca_ndim {
            ca_dim_size.push(cursor.read_u64::<LittleEndian>()?);
        }

        // Optional precomputed min/max per dimension.
        let mut ca_precomputed_min = Vec::new();
        let mut ca_precomputed_max = Vec::new();
        if flags.precomputed_min {
            for _ in 0..ca_ndim {
                ca_precomputed_min.push(cursor.read_f64::<LittleEndian>()?);
            }
        }
        if flags.precomputed_max {
            for _ in 0..ca_ndim {
                ca_precomputed_max.push(cursor.read_f64::<LittleEndian>()?);
            }
        }

        // Fixed axis values: for each dimension i, ca_dim_size[i] f64 values.
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
            ca_scale_axis,
            ca_axis_cc,
            ca_axis_name,
            ca_type,
            ca_storage,
            ca_ndim,
            flags,
            ca_byte_offset_base,
            ca_invalidation_bit_base,
            ca_dim_size,
            ca_precomputed_min,
            ca_precomputed_max,
            ca_axis_values,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a CA block with the field widths the standard specifies: a `u16`
    /// dimension count and a `u32` flags word, followed by the byte-offset and
    /// invalidation-bit bases.
    fn create_ca_block(
        ca_type: u8,
        ca_storage: u8,
        ndim: u16,
        flags: u32,
        dim_sizes: &[u64],
    ) -> Vec<u8> {
        let needs_composition = ca_type == 0 || ca_type == 2;
        let needs_scale_axis = ca_type == 0 || ca_type == 1;
        let needs_cc = (flags & 0x0004) != 0;
        let needs_name = (flags & 0x0008) != 0;
        let needs_min = (flags & 0x0020) != 0;
        let needs_max = (flags & 0x0040) != 0;
        let fixed_axis = (flags & 0x0002) != 0;

        let mut links: Vec<u64> = Vec::new();
        if needs_composition {
            links.push(1000);
        }
        if needs_scale_axis {
            for i in 0..ndim as u64 {
                links.push(2000 + i * 100);
            }
        }
        if needs_cc {
            for i in 0..ndim as u64 {
                links.push(3000 + i * 100);
            }
        }
        if needs_name {
            for i in 0..ndim as u64 {
                links.push(4000 + i * 100);
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
        if needs_min {
            for _ in 0..ndim {
                data_section.extend_from_slice(&0.0f64.to_le_bytes());
            }
        }
        if needs_max {
            for _ in 0..ndim {
                data_section.extend_from_slice(&1.0f64.to_le_bytes());
            }
        }
        if fixed_axis {
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
    fn test_ca_block_array_contiguous() {
        let data = create_ca_block(0, 0, 2, 0, &[3, 4]);
        let ca = CaBlock::parse(&data, 0).unwrap();

        assert_eq!(ca.ca_type, CaArrayType::Array);
        assert_eq!(ca.ca_storage, CaStorage::CnTemplate);
        assert_eq!(ca.ca_ndim, 2);
        assert_eq!(ca.ca_composition, 1000);
        assert_eq!(ca.ca_scale_axis, vec![2000, 2100]);
        assert_eq!(ca.ca_dim_size, vec![3, 4]);
        assert_eq!(ca.total_elements(), 12);
        assert_eq!(ca.shape(), &[3, 4]);
    }

    #[test]
    fn test_ca_block_fixed_axis() {
        let data = create_ca_block(0, 1, 1, 0x0002, &[3]);
        let ca = CaBlock::parse(&data, 0).unwrap();

        assert!(ca.flags.fixed_axis);
        assert_eq!(ca.ca_dim_size, vec![3]);
        assert_eq!(ca.ca_axis_values, vec![0.0, 1.0, 2.0]);
    }

    #[test]
    fn test_ca_block_with_conversion_and_name() {
        let data = create_ca_block(0, 1, 2, 0x000C, &[3, 4]);
        let ca = CaBlock::parse(&data, 0).unwrap();

        assert!(ca.flags.axis_conversion);
        assert!(ca.flags.axis_name);
        assert_eq!(ca.ca_axis_cc, vec![3000, 3100]);
        assert_eq!(ca.ca_axis_name, vec![4000, 4100]);
    }

    #[test]
    fn test_ca_block_precomputed_min_max() {
        let data = create_ca_block(0, 1, 1, 0x0060, &[2]);
        let ca = CaBlock::parse(&data, 0).unwrap();

        assert!(ca.flags.precomputed_min);
        assert!(ca.flags.precomputed_max);
        assert_eq!(ca.ca_precomputed_min, vec![0.0]);
        assert_eq!(ca.ca_precomputed_max, vec![1.0]);
    }

    #[test]
    fn test_ca_block_scale_axis() {
        let data = create_ca_block(1, 0, 1, 0, &[5]);
        let ca = CaBlock::parse(&data, 0).unwrap();

        assert_eq!(ca.ca_type, CaArrayType::ScaleAxis);
        assert_eq!(ca.ca_composition, 0);
        assert_eq!(ca.ca_scale_axis, vec![2000]);
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
