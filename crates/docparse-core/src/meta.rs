//! Document metadata: the `-f meta` projection and OOXML `docProps/core.xml`
//! parsing.
//!
//! [`Metadata`] lives on [`crate::ir::Document`] (filled by the backends that
//! have a metadata source — PDF Info / OOXML core.xml / HTML `<meta>`); this
//! module adds the two shared pieces:
//!
//! - [`report`] / [`to_json`] — the `-f meta` view (source, parser, page
//!   count, metadata), so callers don't have to walk the full document JSON.
//! - [`ooxml_core_metadata`] — `docProps/core.xml` → [`Metadata`], shared by
//!   the DOCX/PPTX/XLSX backends (same container convention, same fields).
//!
//! Honesty rules that hold here too: a field the container doesn't carry
//! stays `None`, dates pass through only when they're already ISO 8601
//! (OOXML's `dcterms:*` is), and a document from a format without any
//! metadata source has `metadata: None` (absent from JSON).

use serde::Serialize;

use crate::ir::{Document, Metadata};

/// The `-f meta` projection: what you'd want next to the parsed content when
/// ingesting into a corpus — identity, producer, size, and the metadata.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MetaReport {
    /// Same value as `Document.source`.
    pub source: String,
    /// Producing parser, e.g. "pdf" (from the provenance).
    pub parser: Option<String>,
    pub page_count: usize,
    /// Container metadata, `None` when the format has no metadata source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

/// Build the `-f meta` view of a parsed document.
pub fn report(doc: &Document) -> MetaReport {
    MetaReport {
        source: doc.source.clone(),
        parser: doc.provenance.as_ref().map(|p| p.parser.clone()),
        page_count: doc.pages.len(),
        metadata: doc.metadata.clone(),
    }
}

/// Serialize the report as pretty JSON (the `-f meta` body).
pub fn to_json(report: &MetaReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_default()
}

/// Read `docProps/core.xml` out of an OOXML zip container (DOCX/PPTX/XLSX)
/// and parse it into a [`Metadata`]. `None` when the archive can't open or
/// the part is missing — properties are best-effort, never fatal.
pub fn ooxml_metadata_from_zip(buf: &[u8]) -> Option<Metadata> {
    use std::io::Read as _;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(buf)).ok()?;
    let mut f = z.by_name("docProps/core.xml").ok()?;
    let mut xml = String::new();
    f.read_to_string(&mut xml).ok()?;
    Some(ooxml_core_metadata(xml.as_bytes()))
}

/// Parse OOXML `docProps/core.xml` (DOCX/PPTX/XLSX all carry the same file)
/// into a [`Metadata`]. Unparsable bytes yield an empty `Metadata` (all
/// `None`) rather than an error — metadata is best-effort by contract and a
/// document's parse must not fail over its properties part.
pub fn ooxml_core_metadata(xml: &[u8]) -> Metadata {
    use quick_xml::events::Event;

    let mut meta = Metadata::default();
    let mut current: Option<&'static str> = None;
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = e.name();
                let local = local.as_ref();
                let local = local.rsplit(|b| *b == b':').next().unwrap_or(local);
                current = match local {
                    b"title" => Some("title"),
                    b"subject" => Some("subject"),
                    b"creator" => Some("author"),
                    b"keywords" => Some("keywords"),
                    b"language" => Some("language"),
                    b"created" => Some("created"),
                    b"modified" => Some("modified"),
                    _ => None,
                };
            }
            Ok(Event::Text(t)) => {
                let Some(key) = current else { continue };
                let Ok(text) = t.unescape() else { continue };
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                let slot = match key {
                    "title" => &mut meta.title,
                    "subject" => &mut meta.subject,
                    "author" => &mut meta.author,
                    "keywords" => &mut meta.keywords,
                    "language" => &mut meta.language,
                    "created" => &mut meta.created,
                    "modified" => &mut meta.modified,
                    _ => unreachable!("current is only set to the keys above"),
                };
                *slot = Some(text.to_string());
            }
            Ok(Event::End(_)) => current = None,
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }
    meta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ooxml_core_xml_maps_all_fields() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"
                   xmlns:dc="http://purl.org/dc/elements/1.1/"
                   xmlns:dcterms="http://purl.org/dc/terms/">
  <dc:title>Quarterly Report</dc:title>
  <dc:subject>Finance</dc:subject>
  <dc:creator>Jane Chen</dc:creator>
  <cp:keywords>q3, budget</cp:keywords>
  <dc:language>en-US</dc:language>
  <dcterms:created>2026-08-01T09:00:00Z</dcterms:created>
  <dcterms:modified>2026-09-01T18:30:00Z</dcterms:modified>
</cp:coreProperties>"#;
        let m = ooxml_core_metadata(xml);
        assert_eq!(m.title.as_deref(), Some("Quarterly Report"));
        assert_eq!(m.subject.as_deref(), Some("Finance"));
        assert_eq!(
            m.author.as_deref(),
            Some("Jane Chen"),
            "dc:creator is the author"
        );
        assert_eq!(m.keywords.as_deref(), Some("q3, budget"));
        assert_eq!(m.language.as_deref(), Some("en-US"));
        assert_eq!(m.created.as_deref(), Some("2026-08-01T09:00:00Z"));
        assert_eq!(m.modified.as_deref(), Some("2026-09-01T18:30:00Z"));
        // No creating-app field in core.xml — stays None (app.xml is out of scope).
        assert_eq!(m.creator, None);
        assert_eq!(m.producer, None);
    }

    #[test]
    fn ooxml_partial_and_garbage_input() {
        let m = ooxml_core_metadata(
            b"<?xml?><cp:coreProperties><dc:title>Only</dc:title></cp:coreProperties>",
        );
        assert_eq!(m.title.as_deref(), Some("Only"));
        assert_eq!(m.author, None);
        // Garbage → empty metadata, never a panic (best-effort by contract).
        let m = ooxml_core_metadata(b"not xml at all <<<");
        assert_eq!(m, Metadata::default());
    }

    #[test]
    fn report_projection_carries_parser_and_count() {
        let doc = Document {
            source: "/tmp/a.pdf".into(),
            provenance: Some(crate::ir::Provenance::new("pdf", "0.1.0")),
            metadata: Some(Metadata {
                title: Some("T".into()),
                ..Default::default()
            }),
            pages: Vec::new(),
        };
        let r = report(&doc);
        assert_eq!(r.parser.as_deref(), Some("pdf"));
        assert_eq!(r.page_count, 0);
        assert_eq!(r.metadata.as_ref().unwrap().title.as_deref(), Some("T"));
        let json = to_json(&r);
        assert!(json.contains("\"parser\": \"pdf\""));
    }

    #[test]
    fn report_without_metadata_skips_the_field() {
        let doc = Document {
            source: "a.csv".into(),
            provenance: Some(crate::ir::Provenance::new("csv", "0.1.0")),
            metadata: None,
            pages: Vec::new(),
        };
        let json = to_json(&report(&doc));
        assert!(
            !json.contains("metadata"),
            "None must be absent, not null: {json}"
        );
    }
}
