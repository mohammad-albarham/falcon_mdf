//! Parser module for MF4 files.
//!
//! This module contains the parsing pipeline that transforms raw bytes
//! into structured block data and ultimately into the high-level data model.

pub mod binary;
pub mod links;
pub mod version;

use crate::blocks::*;
use crate::error::{Mf4Error, Result};
use crate::io::ByteSource;

pub use version::Mf4Version;

/// Parses a block header at the given offset.
pub fn parse_block_header<S: ByteSource>(source: &S, offset: u64) -> Result<BlockHeader> {
    let data = source.read_bytes(offset, BLOCK_HEADER_SIZE)?;
    let header = BlockHeader::parse(&data, offset)?;

    // A block cannot extend past the end of the file. `BlockHeader::parse`
    // checks the header against itself but has no way to know how large the
    // file is; this is the one place every block header is read, so the check
    // belongs here. Without it a corrupt length reaches the allocations derived
    // from it and aborts the process with an allocation failure.
    let avail = source.len().saturating_sub(offset);
    let Some(end) = offset.checked_add(header.length) else {
        return Err(Mf4Error::truncated(
            offset,
            usize::try_from(header.length).unwrap_or(usize::MAX),
            usize::try_from(avail).unwrap_or(usize::MAX),
        ));
    };
    if end > source.len() {
        return Err(Mf4Error::truncated(
            offset,
            usize::try_from(header.length).unwrap_or(usize::MAX),
            usize::try_from(avail).unwrap_or(usize::MAX),
        ));
    }

    Ok(header)
}

fn read_block_bytes<S: ByteSource>(source: &S, offset: u64) -> Result<crate::io::ByteSlice<'_>> {
    let header = parse_block_header(source, offset)?;
    let len = usize::try_from(header.length).map_err(|_| {
        Mf4Error::invalid_block_size(header.block_type_str(), header.length, usize::MAX as u64)
    })?;
    source.read_bytes(offset, len)
}

/// Parses an ID block from the start of the file.
pub fn parse_id_block<S: ByteSource>(source: &S) -> Result<IdBlock> {
    let data = source.read_bytes(0, ID_BLOCK_SIZE)?;
    IdBlock::parse(&data)
}

/// Parses an HD block at the given offset.
pub fn parse_hd_block<S: ByteSource>(source: &S, offset: u64) -> Result<HdBlock> {
    let data = read_block_bytes(source, offset)?;
    HdBlock::parse(&data, offset)
}

/// Parses an FH block at the given offset.
pub fn parse_fh_block<S: ByteSource>(source: &S, offset: u64) -> Result<FhBlock> {
    let data = read_block_bytes(source, offset)?;
    FhBlock::parse(&data, offset)
}

/// Parses a DG block at the given offset.
pub fn parse_dg_block<S: ByteSource>(source: &S, offset: u64) -> Result<DgBlock> {
    let data = read_block_bytes(source, offset)?;
    DgBlock::parse(&data, offset)
}

/// Parses a CG block at the given offset.
pub fn parse_cg_block<S: ByteSource>(source: &S, offset: u64) -> Result<CgBlock> {
    let data = read_block_bytes(source, offset)?;
    CgBlock::parse(&data, offset)
}

/// Parses a CN block at the given offset.
pub fn parse_cn_block<S: ByteSource>(source: &S, offset: u64) -> Result<CnBlock> {
    let data = read_block_bytes(source, offset)?;
    CnBlock::parse(&data, offset)
}

/// Parses a CC block at the given offset.
pub fn parse_cc_block<S: ByteSource>(source: &S, offset: u64) -> Result<CcBlock> {
    let data = read_block_bytes(source, offset)?;
    CcBlock::parse(&data, offset)
}

/// Parses a text block (TX or MD) at the given offset.
pub fn parse_text_block<S: ByteSource>(source: &S, offset: u64) -> Result<TextOrMetadata> {
    let data = read_block_bytes(source, offset)?;
    TextOrMetadata::parse(&data, offset)
}

/// Reads the comment from a TX or MD block at the given link.
///
/// A metadata block's XML is unwrapped to the `<TX>` element it carries; see
/// [`TextOrMetadata::comment`]. Returns an empty string if link is 0.
pub fn read_text<S: ByteSource>(source: &S, link: u64) -> Result<String> {
    if link == 0 {
        return Ok(String::new());
    }
    let block = parse_text_block(source, link)?;
    Ok(block.comment())
}

/// Reads a metadata block's full contents at the given link.
///
/// Returns `None` when the link is zero or names a plain text block, which
/// carries no properties.
pub fn read_metadata<S: ByteSource>(
    source: &S,
    link: u64,
) -> Result<Option<crate::model::Metadata>> {
    if link == 0 {
        return Ok(None);
    }
    match parse_text_block(source, link)? {
        TextOrMetadata::Metadata(md) => Ok(Some(md.metadata())),
        TextOrMetadata::Text(_) => Ok(None),
    }
}

/// Parses an SI block at the given offset.
pub fn parse_si_block<S: ByteSource>(source: &S, offset: u64) -> Result<SiBlock> {
    let data = read_block_bytes(source, offset)?;
    SiBlock::parse(&data, offset)
}

/// Parses a data block (DT, DZ, DL, HL) at the given offset.
pub fn parse_data_block<S: ByteSource>(source: &S, offset: u64) -> Result<DataBlock> {
    let data = read_block_bytes(source, offset)?;
    DataBlock::parse(&data, offset)
}

/// Parses a Channel Array (CA) block at the given offset.
pub fn parse_ca_block<S: ByteSource>(source: &S, offset: u64) -> Result<CaBlock> {
    let data = read_block_bytes(source, offset)?;
    CaBlock::parse(&data, offset)
}

/// Parses an Attachment (AT) block at the given offset.
pub fn parse_at_block<S: ByteSource>(source: &S, offset: u64) -> Result<AtBlock> {
    let data = read_block_bytes(source, offset)?;
    AtBlock::parse(&data, offset)
}

/// Parses an Event (EV) block at the given offset.
pub fn parse_ev_block<S: ByteSource>(source: &S, offset: u64) -> Result<EvBlock> {
    let data = read_block_bytes(source, offset)?;
    EvBlock::parse(&data, offset)
}

/// Parses a Channel Hierarchy (CH) block at the given offset.
pub fn parse_ch_block<S: ByteSource>(source: &S, offset: u64) -> Result<ChBlock> {
    let data = read_block_bytes(source, offset)?;
    ChBlock::parse(&data, offset)
}

/// Parses a Sample Reduction (SR) block at the given offset.
pub fn parse_sr_block<S: ByteSource>(source: &S, offset: u64) -> Result<SrBlock> {
    let data = read_block_bytes(source, offset)?;
    SrBlock::parse(&data, offset)
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
    chain: links::LinkChain,
    chain_name: &'static str,
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
        if let Err(e) = self.chain.visit(self.next_offset, self.chain_name) {
            self.next_offset = 0;
            return Some(Err(e));
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
        chain: links::LinkChain::new(),
        chain_name: "dg_next",
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
        chain: links::LinkChain::new(),
        chain_name: "cg_next",
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
        chain: links::LinkChain::new(),
        chain_name: "cn_next",
    }
}

/// Creates an iterator over attachment blocks.
pub fn iter_attachments<S: ByteSource>(
    source: &S,
    first_at: u64,
) -> impl Iterator<Item = Result<AtBlock>> + '_ {
    LinkedBlockIterator {
        source,
        next_offset: first_at,
        parse_fn: parse_at_block,
        get_next: |at: &AtBlock| at.at_next,
        chain: links::LinkChain::new(),
        chain_name: "at_next",
    }
}

/// Creates an iterator over event blocks.
pub fn iter_events<S: ByteSource>(
    source: &S,
    first_ev: u64,
) -> impl Iterator<Item = Result<EvBlock>> + '_ {
    LinkedBlockIterator {
        source,
        next_offset: first_ev,
        parse_fn: parse_ev_block,
        get_next: |ev: &EvBlock| ev.ev_next,
        chain: links::LinkChain::new(),
        chain_name: "ev_next",
    }
}

/// Creates an iterator over channel hierarchy blocks.
pub fn iter_hierarchy<S: ByteSource>(
    source: &S,
    first_ch: u64,
) -> impl Iterator<Item = Result<ChBlock>> + '_ {
    LinkedBlockIterator {
        source,
        next_offset: first_ch,
        parse_fn: parse_ch_block,
        get_next: |ch: &ChBlock| ch.ch_next,
        chain: links::LinkChain::new(),
        chain_name: "ch_next",
    }
}

/// Creates an iterator over sample reduction blocks.
pub fn iter_sample_reduction<S: ByteSource>(
    source: &S,
    first_sr: u64,
) -> impl Iterator<Item = Result<SrBlock>> + '_ {
    LinkedBlockIterator {
        source,
        next_offset: first_sr,
        parse_fn: parse_sr_block,
        get_next: |sr: &SrBlock| sr.sr_next,
        chain: links::LinkChain::new(),
        chain_name: "sr_next",
    }
}
