//! Text (TX) and Metadata (MD) block parsing.
//!
//! TX blocks contain plain text strings (names, units, comments).
//! MD blocks contain XML metadata.

use crate::blocks::common::{BlockHeader, ParseBlock, BLOCK_HEADER_SIZE};
use crate::error::{Mf4Error, Result};

/// The Text (TX) block.
///
/// Contains a UTF-8 encoded string, typically used for channel names,
/// units, or comments.
#[derive(Debug, Clone)]
pub struct TxBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// The text content (UTF-8).
    pub text: String,
}

impl ParseBlock for TxBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##TX", offset)?;

        let data_start = BLOCK_HEADER_SIZE;
        let data_len = (header.length as usize).saturating_sub(data_start);

        if data.len() < data_start + data_len {
            return Err(Mf4Error::truncated(
                offset,
                data_start + data_len,
                data.len(),
            ));
        }

        let text_bytes = &data[data_start..data_start + data_len];

        // Find null terminator
        let end = text_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(text_bytes.len());
        let text = String::from_utf8_lossy(&text_bytes[..end]).to_string();

        Ok(TxBlock { header, text })
    }
}

impl TxBlock {
    /// Returns the text content trimmed of whitespace.
    pub fn text_trimmed(&self) -> &str {
        self.text.trim()
    }
}

/// The Metadata (MD) block.
///
/// Contains XML-formatted metadata with additional information
/// about channels, sources, etc.
#[derive(Debug, Clone)]
pub struct MdBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// The XML content.
    pub xml: String,
}

impl MdBlock {
    /// Parses the XML into its comment and named properties.
    pub fn metadata(&self) -> crate::model::Metadata {
        crate::model::Metadata::parse(&self.xml)
    }
}

impl ParseBlock for MdBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##MD", offset)?;

        let data_start = BLOCK_HEADER_SIZE;
        let data_len = (header.length as usize).saturating_sub(data_start);

        if data.len() < data_start + data_len {
            return Err(Mf4Error::truncated(
                offset,
                data_start + data_len,
                data.len(),
            ));
        }

        let xml_bytes = &data[data_start..data_start + data_len];

        // Find null terminator
        let end = xml_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(xml_bytes.len());
        let xml = String::from_utf8_lossy(&xml_bytes[..end]).to_string();

        Ok(MdBlock { header, xml })
    }
}

/// Represents either a TX (text) or MD (metadata) block.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TextOrMetadata {
    /// Plain text block.
    Text(TxBlock),
    /// XML metadata block.
    Metadata(MdBlock),
}

impl TextOrMetadata {
    /// Parses a text or metadata block, auto-detecting the type.
    pub fn parse(data: &[u8], offset: u64) -> Result<Self> {
        if data.len() < 4 {
            return Err(Mf4Error::truncated(offset, 4, data.len()));
        }

        let block_id = &data[0..4];
        match block_id {
            b"##TX" => Ok(TextOrMetadata::Text(TxBlock::parse(data, offset)?)),
            b"##MD" => Ok(TextOrMetadata::Metadata(MdBlock::parse(data, offset)?)),
            _ => Err(Mf4Error::invalid_block_id(
                offset,
                "##TX/##MD",
                String::from_utf8_lossy(block_id).to_string(),
            )),
        }
    }

    /// Returns the text content regardless of block type.
    ///
    /// For a metadata block this is the raw XML. Callers wanting the comment a
    /// human wrote should use [`TextOrMetadata::comment`] instead — the XML is a
    /// container, not the message.
    pub fn as_str(&self) -> &str {
        match self {
            TextOrMetadata::Text(tx) => &tx.text,
            TextOrMetadata::Metadata(md) => &md.xml,
        }
    }

    /// Returns the human-readable comment.
    ///
    /// A text block is its own comment. A metadata block wraps one in XML, so
    /// the `<TX>` element is extracted; returning the markup would make every
    /// caller parse it.
    pub fn comment(&self) -> String {
        match self {
            TextOrMetadata::Text(tx) => tx.text.trim().to_string(),
            TextOrMetadata::Metadata(md) => md.metadata().text().to_string(),
        }
    }

    /// Returns the text content trimmed.
    pub fn as_str_trimmed(&self) -> &str {
        self.as_str().trim()
    }
}

/// Reads a text string from a TX or MD block at the given link.
///
/// If the link is 0, returns an empty string.
pub fn read_text_at_link(data: &[u8], file_len: u64, link: u64) -> Result<String> {
    if link == 0 {
        return Ok(String::new());
    }

    if link >= file_len {
        return Err(Mf4Error::InvalidLink {
            offset: 0,
            target: link,
        });
    }

    let offset = link as usize;
    let remaining = data.len().saturating_sub(offset);

    if remaining < BLOCK_HEADER_SIZE {
        return Err(Mf4Error::truncated(link, BLOCK_HEADER_SIZE, remaining));
    }

    let block_data = &data[offset..];
    let text_or_md = TextOrMetadata::parse(block_data, link)?;
    Ok(text_or_md.as_str_trimmed().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_tx_block(text: &str) -> Vec<u8> {
        let text_bytes = text.as_bytes();
        let total_len = BLOCK_HEADER_SIZE + text_bytes.len() + 1; // +1 for null terminator
        let mut data = vec![0u8; total_len];

        // Header
        data[0..4].copy_from_slice(b"##TX");
        data[8..16].copy_from_slice(&(total_len as u64).to_le_bytes());
        data[16..24].copy_from_slice(&0u64.to_le_bytes()); // link_count

        // Text content
        data[BLOCK_HEADER_SIZE..BLOCK_HEADER_SIZE + text_bytes.len()].copy_from_slice(text_bytes);
        // Null terminator is already 0

        data
    }

    #[test]
    fn test_tx_block_parse() {
        let data = create_test_tx_block("TestChannel");
        let tx = TxBlock::parse(&data, 0).unwrap();

        assert_eq!(tx.text, "TestChannel");
        assert_eq!(tx.text_trimmed(), "TestChannel");
    }

    #[test]
    fn test_tx_block_with_whitespace() {
        let data = create_test_tx_block("  Test  ");
        let tx = TxBlock::parse(&data, 0).unwrap();

        assert_eq!(tx.text, "  Test  ");
        assert_eq!(tx.text_trimmed(), "Test");
    }

    fn create_test_md_block(xml: &str) -> Vec<u8> {
        let xml_bytes = xml.as_bytes();
        let total_len = BLOCK_HEADER_SIZE + xml_bytes.len() + 1;
        let mut data = vec![0u8; total_len];

        data[0..4].copy_from_slice(b"##MD");
        data[8..16].copy_from_slice(&(total_len as u64).to_le_bytes());
        data[16..24].copy_from_slice(&0u64.to_le_bytes());
        data[BLOCK_HEADER_SIZE..BLOCK_HEADER_SIZE + xml_bytes.len()].copy_from_slice(xml_bytes);

        data
    }

    #[test]
    fn test_md_block_parse() {
        let data = create_test_md_block("<root><item>value</item></root>");
        let md = MdBlock::parse(&data, 0).unwrap();

        assert!(md.xml.contains("<root>"));
    }

    #[test]
    fn test_text_or_metadata_auto_detect() {
        let tx_data = create_test_tx_block("Hello");
        let result = TextOrMetadata::parse(&tx_data, 0).unwrap();
        assert!(matches!(result, TextOrMetadata::Text(_)));
        assert_eq!(result.as_str_trimmed(), "Hello");

        let md_data = create_test_md_block("<xml/>");
        let result = TextOrMetadata::parse(&md_data, 0).unwrap();
        assert!(matches!(result, TextOrMetadata::Metadata(_)));
    }
}
