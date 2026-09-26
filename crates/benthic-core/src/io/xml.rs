//! A minimal XML tree, shared by the XML-based importers.
//!
//! We only need the subset of XML that dive-log formats emit, so a
//! hand-rolled, stack-based tree is plenty and avoids pulling in a full DOM.

use quick_xml::events::Event as XmlEvent;
use quick_xml::Reader;

use crate::{Error, Result};

/// An XML element with its attributes, text and children.
#[derive(Debug, Default)]
pub(crate) struct Node {
    pub(crate) name: String,
    pub(crate) attrs: Vec<(String, String)>,
    pub(crate) text: String,
    pub(crate) children: Vec<Node>,
}

impl Node {
    pub(crate) fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub(crate) fn text_trimmed(&self) -> &str {
        self.text.trim()
    }

    pub(crate) fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    pub(crate) fn children_named<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
}

/// Parse XML text into a tree.
pub(crate) fn parse_document(xml: &str) -> Result<Node> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;

    loop {
        match reader.read_event_into(&mut buf)? {
            XmlEvent::Start(e) => {
                stack.push(node_from_start(&e)?);
            }
            XmlEvent::Empty(e) => {
                let node = node_from_start(&e)?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root = Some(node),
                }
            }
            XmlEvent::Text(e) => {
                if let Some(parent) = stack.last_mut() {
                    parent.text.push_str(&e.unescape()?);
                }
            }
            XmlEvent::CData(e) => {
                if let Some(parent) = stack.last_mut() {
                    parent.text.push_str(&String::from_utf8_lossy(e.as_ref()));
                }
            }
            XmlEvent::End(_) => {
                if let Some(node) = stack.pop() {
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(node),
                        None => root = Some(node),
                    }
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    root.ok_or_else(|| Error::Parse {
        what: "XML document",
        value: "empty input".into(),
    })
}

fn node_from_start(e: &quick_xml::events::BytesStart<'_>) -> Result<Node> {
    let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
    let mut attrs = Vec::new();
    for attr in e.attributes() {
        let attr = attr?;
        let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
        let value = attr.unescape_value()?.into_owned();
        attrs.push((key, value));
    }
    Ok(Node {
        name,
        attrs,
        text: String::new(),
        children: Vec::new(),
    })
}
