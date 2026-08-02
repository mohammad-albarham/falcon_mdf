//! Parsed MD (metadata) block contents.
//!
//! MF4 stores descriptive information as XML in MD blocks, in a shape fixed by
//! the standard: a `<TX>` element holding the human-readable comment, and an
//! optional `<common_properties>` element holding named values, optionally
//! grouped into named trees. A file header's metadata typically looks like:
//!
//! ```xml
//! <HDcomment>
//!     <TX>Recording of a test drive</TX>
//!     <common_properties>
//!         <tree name="Device Information">
//!             <e name="serial number">0BFD7754</e>
//!         </tree>
//!     </common_properties>
//! </HDcomment>
//! ```
//!
//! Handing that back as a string means every caller writes its own XML parsing.
//! [`Metadata`] extracts the comment and flattens the properties to
//! `"Device Information/serial number"` → `"0BFD7754"`, while keeping the
//! original XML for anything this does not model.

use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::BTreeMap;

/// The contents of an MD block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    text: String,
    properties: BTreeMap<String, String>,
    xml: String,
}

impl Metadata {
    /// Parses MD block XML.
    ///
    /// Never fails: metadata is descriptive, so malformed or unexpected markup
    /// yields whatever could be read rather than failing the file. The original
    /// text is always available from [`Metadata::xml`].
    pub fn parse(xml: &str) -> Self {
        let mut meta = Metadata {
            xml: xml.to_string(),
            ..Default::default()
        };

        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        // Names of the `<tree>` elements currently open, which give a property
        // its path.
        let mut trees: Vec<String> = Vec::new();
        let mut in_text = false;
        let mut pending_key: Option<String> = None;

        // Names of the elements currently open, so a leaf carrying text — a
        // file-history block's `<tool_id>`, say — can be recorded under its own
        // tag. Such elements sit outside `common_properties` but are still the
        // information the block exists to convey.
        let mut open: Vec<String> = Vec::new();
        let mut depth_at_last_start = 0usize;

        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    match e.name().as_ref() {
                        b"TX" => in_text = true,
                        b"tree" => trees.push(attribute(&e, b"name").unwrap_or_default()),
                        b"e" => pending_key = Some(attribute(&e, b"name").unwrap_or_default()),
                        _ => {}
                    }
                    open.push(name);
                    depth_at_last_start = open.len();
                }
                Ok(Event::End(e)) => {
                    match e.name().as_ref() {
                        b"TX" => in_text = false,
                        b"tree" => {
                            trees.pop();
                        }
                        b"e" => {
                            // An `<e/>` with no text still names a property;
                            // record it as empty rather than dropping it.
                            if let Some(key) = pending_key.take() {
                                meta.insert(&trees, key, String::new());
                            }
                        }
                        _ => {}
                    }
                    open.pop();
                }
                Ok(Event::Empty(e)) => {
                    if e.name().as_ref() == b"e" {
                        if let Some(key) = attribute(&e, b"name") {
                            meta.insert(&trees, key, String::new());
                        }
                    }
                }
                Ok(Event::Text(t)) => {
                    let value = t
                        .unescape()
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    if let Some(key) = pending_key.take() {
                        meta.insert(&trees, key, value);
                    } else if in_text && !value.is_empty() {
                        if !meta.text.is_empty() {
                            meta.text.push('\n');
                        }
                        meta.text.push_str(&value);
                    } else if !value.is_empty() && open.len() == depth_at_last_start {
                        // A leaf element's own text, outside any tree — record
                        // it under the tag name. Skipped when the element has
                        // children, whose text belongs to them.
                        if let Some(tag) = open.last() {
                            if tag != "TX" && !meta.properties.contains_key(tag) {
                                meta.properties.insert(tag.clone(), value);
                            }
                        }
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }

        meta
    }

    fn insert(&mut self, trees: &[String], key: String, value: String) {
        let path = if trees.is_empty() {
            key
        } else {
            format!("{}/{}", trees.join("/"), key)
        };
        self.properties.insert(path, value);
    }

    /// Returns the human-readable comment, from the `<TX>` element.
    ///
    /// Empty when the metadata carries only properties, which is common.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Looks up a property by its path, e.g. `"Device Information/serial number"`.
    pub fn get(&self, path: &str) -> Option<&str> {
        self.properties.get(path).map(|s| s.as_str())
    }

    /// Returns every property as `(path, value)`, ordered by path.
    pub fn properties(&self) -> impl Iterator<Item = (&str, &str)> {
        self.properties
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Returns the number of properties.
    ///
    /// Named for what it counts rather than `len`, because a metadata block
    /// holds a comment as well: `len() == 0` alongside `is_empty() == false`
    /// would be a contradiction for a block carrying only a comment.
    pub fn property_count(&self) -> usize {
        self.properties.len()
    }

    /// Returns true if there is neither comment text nor any property.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.properties.is_empty()
    }

    /// Returns the original XML, for anything this type does not model.
    pub fn xml(&self) -> &str {
        &self.xml
    }
}

/// Reads one attribute's value as a string.
fn attribute(e: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.unescape_value().ok())
        .map(|v| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_comment_text() {
        let m = Metadata::parse("<HDcomment><TX>a test drive</TX></HDcomment>");
        assert_eq!(m.text(), "a test drive");
    }

    #[test]
    fn an_empty_tx_element_yields_no_text() {
        let m = Metadata::parse("<HDcomment><TX/></HDcomment>");
        assert_eq!(m.text(), "");
    }

    #[test]
    fn flattens_named_trees_into_property_paths() {
        let xml = r#"
            <HDcomment>
                <TX/>
                <common_properties>
                    <tree name="Device Information">
                        <e name="serial number" ro="true">0BFD7754</e>
                        <e name="firmware version">01.07.03</e>
                    </tree>
                </common_properties>
            </HDcomment>"#;
        let m = Metadata::parse(xml);
        assert_eq!(m.get("Device Information/serial number"), Some("0BFD7754"));
        assert_eq!(
            m.get("Device Information/firmware version"),
            Some("01.07.03")
        );
        assert_eq!(m.property_count(), 2);
    }

    #[test]
    fn handles_properties_outside_any_tree() {
        let xml = r#"<SIcomment><common_properties>
                       <e name="bus">CAN</e>
                     </common_properties></SIcomment>"#;
        assert_eq!(Metadata::parse(xml).get("bus"), Some("CAN"));
    }

    #[test]
    fn handles_nested_trees() {
        let xml = r#"
            <SIcomment><common_properties>
                <tree name="ASAM Measurement Environment">
                    <tree name="node">
                        <e name="type">Device</e>
                    </tree>
                </tree>
            </common_properties></SIcomment>"#;
        let m = Metadata::parse(xml);
        assert_eq!(
            m.get("ASAM Measurement Environment/node/type"),
            Some("Device")
        );
    }

    #[test]
    fn a_valueless_property_is_recorded_as_empty_not_dropped() {
        let xml = r#"<c><common_properties><e name="vendor"></e></common_properties></c>"#;
        let m = Metadata::parse(xml);
        assert_eq!(m.get("vendor"), Some(""), "the property exists, empty");
    }

    #[test]
    fn unescapes_entities() {
        let m = Metadata::parse("<c><TX>a &amp; b &lt;c&gt;</TX></c>");
        assert_eq!(m.text(), "a & b <c>");
    }

    #[test]
    fn malformed_xml_yields_what_could_be_read_rather_than_failing() {
        // Metadata is descriptive; losing it must not lose the file.
        let m = Metadata::parse("<c><TX>kept</TX><unclosed>");
        assert_eq!(m.text(), "kept");
        assert!(!m.xml().is_empty(), "the original is always retained");
    }

    #[test]
    fn plain_text_that_is_not_xml_is_survivable() {
        let m = Metadata::parse("just a comment");
        assert!(m.properties().next().is_none());
    }

    #[test]
    fn reports_emptiness() {
        assert!(Metadata::parse("<c/>").is_empty());
        assert!(!Metadata::parse("<c><TX>x</TX></c>").is_empty());
    }
}
