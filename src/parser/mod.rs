//! Parser module for MF4 files.
//!
//! This module contains the parsing pipeline that transforms raw bytes
//! into structured block data and ultimately into the high-level data model.

pub mod binary;
pub mod version;

use crate::blocks::*;
use crate::error::Result;
use crate::io::ByteSource;

pub use version::Mf4Version;

/// Parses a block header at the given offset.
pub fn parse_block_header<S: ByteSource>(source: &S, offset: u64) -> Result<BlockHeader> {
    let data = source.read_bytes(offset, BLOCK_HEADER_SIZE)?;
    BlockHeader::parse(&data, offset)
}

/// Parses an ID block from the start of the file.
pub fn parse_id_block<S: ByteSource>(source: &S) -> Result<IdBlock> {
    let data = source.read_bytes(0, ID_BLOCK_SIZE)?;
    IdBlock::parse(&data)
}

/// Parses an HD block at the given offset.
pub fn parse_hd_block<S: ByteSource>(source: &S, offset: u64) -> Result<HdBlock> {
    let header = parse_block_header(source, offset)?;
    let data = source.read_bytes(offset, header.length as usize)?;
    HdBlock::parse(&data, offset)
}

/// Parses a DG block at the given offset.
pub fn parse_dg_block<S: ByteSource>(source: &S, offset: u64) -> Result<DgBlock> {
    let header = parse_block_header(source, offset)?;
    let data = source.read_bytes(offset, header.length as usize)?;
    DgBlock::parse(&data, offset)
}

/// Parses a CG block at the given offset.
pub fn parse_cg_block<S: ByteSource>(source: &S, offset: u64) -> Result<CgBlock> {
    let header = parse_block_header(source, offset)?;
    let data = source.read_bytes(offset, header.length as usize)?;
    CgBlock::parse(&data, offset)
}

/// Parses a CN block at the given offset.
pub fn parse_cn_block<S: ByteSource>(source: &S, offset: u64) -> Result<CnBlock> {
    let header = parse_block_header(source, offset)?;
    let data = source.read_bytes(offset, header.length as usize)?;
    CnBlock::parse(&data, offset)
}

/// Parses a CC block at the given offset.
pub fn parse_cc_block<S: ByteSource>(source: &S, offset: u64) -> Result<CcBlock> {
    let header = parse_block_header(source, offset)?;
    let data = source.read_bytes(offset, header.length as usize)?;
    CcBlock::parse(&data, offset)
}

/// Parses a text block (TX or MD) at the given offset.
pub fn parse_text_block<S: ByteSource>(source: &S, offset: u64) -> Result<TextOrMetadata> {
    let header = parse_block_header(source, offset)?;
    let data = source.read_bytes(offset, header.length as usize)?;
    TextOrMetadata::parse(&data, offset)
}

/// Reads text from a TX or MD block at the given link.
/// Returns an empty string if link is 0.
pub fn read_text<S: ByteSource>(source: &S, link: u64) -> Result<String> {
    if link == 0 {
        return Ok(String::new());
    }
    let block = parse_text_block(source, link)?;
    Ok(block.as_str_trimmed().to_string())
}

/// Parses an SI block at the given offset.
pub fn parse_si_block<S: ByteSource>(source: &S, offset: u64) -> Result<SiBlock> {
    let header = parse_block_header(source, offset)?;
    let data = source.read_bytes(offset, header.length as usize)?;
    SiBlock::parse(&data, offset)
}

/// Parses a data block (DT, DZ, DL, HL) at the given offset.
pub fn parse_data_block<S: ByteSource>(source: &S, offset: u64) -> Result<DataBlock> {
    let header = parse_block_header(source, offset)?;
    let data = source.read_bytes(offset, header.length as usize)?;
    DataBlock::parse(&data, offset)
}

/// Iterator over linked blocks (following a chain of "next" links).
pub struct LinkedBlockIterator<'a, S: ByteSource, T, F>
where
    F: Fn(&S, u64) -> Result<T>,
{
    source: &'a S,
    next_offset: u64,
    parse_fn: F,
    get_next: fn(&T) -> u64,
}

impl<'a, S: ByteSource, T, F> Iterator for LinkedBlockIterator<'a, S, T, F>
where
    F: Fn(&S, u64) -> Result<T>,
{
    type Item = Result<T>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_offset == 0 {
            return None;
        }

        match (self.parse_fn)(self.source, self.next_offset) {
            Ok(block) => {
                self.next_offset = (self.get_next)(&block);
                Some(Ok(block))
            }
            Err(e) => {
                self.next_offset = 0; // Stop iteration on error
                Some(Err(e))
            }
        }
    }
}

/// Creates an iterator over data group blocks.
pub fn iter_data_groups<S: ByteSource>(
    source: &S,
    first_dg: u64,
) -> impl Iterator<Item = Result<DgBlock>> + '_ {
    LinkedBlockIterator {
        source,
        next_offset: first_dg,
        parse_fn: parse_dg_block,
        get_next: |dg: &DgBlock| dg.dg_next,
    }
}

/// Creates an iterator over channel group blocks.
pub fn iter_channel_groups<S: ByteSource>(
    source: &S,
    first_cg: u64,
) -> impl Iterator<Item = Result<CgBlock>> + '_ {
    LinkedBlockIterator {
        source,
        next_offset: first_cg,
        parse_fn: parse_cg_block,
        get_next: |cg: &CgBlock| cg.cg_next,
    }
}

/// Creates an iterator over channel blocks.
pub fn iter_channels<S: ByteSource>(
    source: &S,
    first_cn: u64,
) -> impl Iterator<Item = Result<CnBlock>> + '_ {
    LinkedBlockIterator {
        source,
        next_offset: first_cn,
        parse_fn: parse_cn_block,
        get_next: |cn: &CnBlock| cn.cn_next,
    }
}
