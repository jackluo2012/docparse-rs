//! Output serializers: JSON (full IR), Markdown, and plain text.
//!
//! Markdown/text are built from [`crate::layout`] blocks: per-glyph chunks are
//! rebuilt into lines (word spaces by geometric gap), text inside a detected
//! table is excluded, running headers/footers dropped, and consecutive lines
//! grouped into paragraphs/headings. Tables render as their own blocks.

use crate::ir::{Document, Table};
use crate::layout::{self, PageItem};

/// Full IR as pretty JSON.
pub fn to_json(doc: &Document) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(doc)?)
}

/// Per-page reconstruction in **reading order**: text blocks (table content
/// excluded, headers/footers dropped, paragraphs grouped) with tables/images
/// spliced into their geometric position — a float is rendered where it sits
/// on the page, not after all text. Images worth rendering: exported to disk
/// (`file` set, referenced in Markdown) or carrying a caption (a VLM
/// description to surface).
struct PageContent<'a> {
    items: Vec<PageItem<'a>>,
}

fn document_content(doc: &Document) -> Vec<PageContent<'_>> {
    layout::page_items(doc)
        .into_iter()
        .zip(&doc.pages)
        .map(|(items, _page)| PageContent {
            items: items
                .into_iter()
                .filter(|it| match it {
                    PageItem::Image(i) => i.file.is_some() || i.caption.is_some(),
                    _ => true,
                })
                .collect(),
        })
        .collect()
}

/// Plain text: paragraphs one per line; tables as tab-separated rows.
pub fn to_text(doc: &Document) -> String {
    let mut s = String::new();
    let mut prev_header: Option<(usize, Vec<String>)> = None;
    for pc in document_content(doc) {
        for item in &pc.items {
            match item {
                PageItem::Block(b) => {
                    s.push_str(b.text.trim());
                    s.push('\n');
                }
                PageItem::Table(t) => {
                    if crate::table::looks_like_header_row(&t.rows[0]) {
                        prev_header = Some((
                            t.rows[0].len(),
                            t.rows[0]
                                .iter()
                                .map(|c| c.text.trim().to_string())
                                .collect(),
                        ));
                    } else if let Some((cols, hdr)) = &prev_header {
                        if *cols == t.rows[0].len() {
                            s.push_str(&format!("[continued table] {}\n", hdr.join("\t")));
                        } else {
                            s.push_str("[table without header]\n");
                        }
                    } else {
                        s.push_str("[table without header]\n");
                    }
                    for row in &t.rows {
                        let cells: Vec<&str> = row.iter().map(|c| c.text.trim()).collect();
                        s.push_str(&cells.join("\t"));
                        s.push('\n');
                    }
                    s.push('\n');
                }
                PageItem::Image(i) => {
                    if let Some(c) = &i.caption {
                        s.push_str(c.trim());
                        s.push('\n');
                    }
                }
            }
        }
        s.push('\n');
    }
    s
}

/// Markdown: blocks become paragraphs (`##` for headings); tables become pipe
/// tables (first row treated as the header).
pub fn to_markdown(doc: &Document) -> String {
    let mut md = format!("<!-- source: {} -->\n\n", doc.source);
    // Last header seen (columns, texts), inherited by continued tables that
    // have no header row of their own (e.g. a table split across pages).
    let mut prev_header: Option<(usize, Vec<String>)> = None;
    for pc in document_content(doc) {
        for item in &pc.items {
            match item {
                PageItem::Block(b) => {
                    let block = b;
                    let t = block.text.trim();
                    if t.is_empty() {
                        continue;
                    }
                    if block.code {
                        md.push_str("```\n");
                        md.push_str(&block.text);
                        md.push_str("\n```\n\n");
                        continue;
                    }
                    if block.list_item {
                        // Bullets normalize to "-"; ordinals keep their own numbering
                        // (Markdown renders both as lists).
                        let t = block.text.trim_start();
                        let rendered = match t.chars().next() {
                            Some('•' | '·' | '‣' | '▪' | '◦' | '○' | '–') => {
                                format!(
                                    "- {}",
                                    t[t.chars().next().unwrap().len_utf8()..].trim_start()
                                )
                            }
                            _ => t.to_string(),
                        };
                        md.push_str(&rendered);
                        md.push('\n');
                        continue;
                    }
                    if block.heading {
                        // Level 1 → "## " (single # reserved for a document title),
                        // deeper levels nest accordingly.
                        for _ in 0..(block.level.clamp(1, 4) + 1) {
                            md.push('#');
                        }
                        md.push(' ');
                    }
                    md.push_str(t);
                    md.push_str("\n\n");
                }
                PageItem::Table(t) => {
                    if crate::table::looks_like_header_row(&t.rows[0]) {
                        md.push_str(&markdown_table(t));
                        md.push('\n');
                        prev_header = Some((
                            t.rows[0].len(),
                            t.rows[0]
                                .iter()
                                .map(|c| c.text.trim().to_string())
                                .collect(),
                        ));
                    } else if let Some((cols, hdr)) = &prev_header {
                        if *cols == t.rows[0].len() {
                            md.push_str("<!-- continued table: header inherited from the previous table -->\n\n");
                            let header: Vec<crate::ir::Cell> = hdr
                                .iter()
                                .map(|t| crate::ir::Cell {
                                    text: t.clone(),
                                    bbox: crate::ir::BBox {
                                        x0: 0.0,
                                        y0: 0.0,
                                        x1: 0.0,
                                        y1: 0.0,
                                    },
                                    row_span: 1,
                                    col_span: 1,
                                    merged: false,
                                })
                                .collect();
                            md.push_str(&md_table_with_header(&header, &t.rows));
                            md.push('\n');
                        } else {
                            md.push_str("<!-- continued table without header (column count differs from previous) -->\n\n");
                            md.push_str(&md_table_datarows(&t.rows));
                            md.push('\n');
                        }
                    } else {
                        md.push_str("<!-- continued table without header -->\n\n");
                        md.push_str(&md_table_datarows(&t.rows));
                        md.push('\n');
                    }
                }
                PageItem::Image(i) => {
                    // Caption (e.g. a VLM description) becomes the image's alt
                    // text; a caption-only image (no exported file) still
                    // surfaces its text.
                    let alt = i
                        .caption
                        .as_deref()
                        .map(|c| c.replace(['\n', '\r'], " ").replace(']', ")"))
                        .unwrap_or_else(|| format!("image p{}", i.page));
                    match &i.file {
                        Some(f) => md.push_str(&format!("![{alt}]({f})\n\n")),
                        None => {
                            if i.caption.is_some() {
                                md.push_str(&format!("*{}*\n\n", alt.trim()));
                            }
                        }
                    }
                }
            }
        }
    }
    md
}

/// Render a table as a GitHub-flavored Markdown pipe table. A row that does
/// not look like a header (continued/fragmented table) renders as bare data
/// rows — no fake `---` header line is emitted.
fn markdown_table(table: &Table) -> String {
    let cols = table.rows.first().map(|r| r.len()).unwrap_or(0);
    if cols == 0 {
        return String::new();
    }
    if crate::table::looks_like_header_row(&table.rows[0]) {
        md_table_with_header(&table.rows[0], &table.rows[1..])
    } else {
        md_table_datarows(&table.rows)
    }
}

fn md_table_with_header(header: &[crate::ir::Cell], body: &[Vec<crate::ir::Cell>]) -> String {
    let mut s = String::new();
    let esc = |t: &str| t.replace('|', "\\|").replace('\n', " ");
    s.push('|');
    for cell in header {
        s.push(' ');
        s.push_str(esc(cell.text.trim()).trim());
        s.push_str(" |");
    }
    s.push('\n');
    s.push('|');
    for _ in 0..header.len() {
        s.push_str(" --- |");
    }
    s.push('\n');
    for row in body {
        s.push('|');
        for cell in row {
            s.push(' ');
            s.push_str(esc(cell.text.trim()).trim());
            s.push_str(" |");
        }
        s.push('\n');
    }
    s
}

fn md_table_datarows(rows: &[Vec<crate::ir::Cell>]) -> String {
    let mut s = String::new();
    let esc = |t: &str| t.replace('|', "\\|").replace('\n', " ");
    for row in rows {
        s.push('|');
        for cell in row {
            s.push(' ');
            s.push_str(esc(cell.text.trim()).trim());
            s.push_str(" |");
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{BBox, Element, ImageChunk, ImageKind, Page};

    fn img(file: Option<&str>, caption: Option<&str>) -> Element {
        Element::Image(ImageChunk {
            bbox: BBox {
                x0: 72.0,
                y0: 400.0,
                x1: 500.0,
                y1: 700.0,
            },
            page: 1,
            width_px: 100,
            height_px: 100,
            turns: 0,
            kind: ImageKind::None,
            data: Vec::new(),
            file: file.map(Into::into),
            data_base64: None,
            data_media_type: None,
            caption: caption.map(Into::into),
            caption_source: caption.map(|_| "vlm:test".into()),
        })
    }

    fn doc(elements: Vec<Element>) -> Document {
        Document {
            source: "t".into(),
            provenance: None,
            metadata: None,
            pages: vec![Page {
                number: 1,
                width: 612.0,
                height: 792.0,
                elements,
            }],
        }
    }

    #[test]
    fn markdown_uses_caption_as_alt_text() {
        let md = to_markdown(&doc(vec![img(
            Some("assets/fig.png"),
            Some("A bar chart of revenue."),
        )]));
        assert!(
            md.contains("![A bar chart of revenue.](assets/fig.png)"),
            "got: {md}"
        );
    }

    #[test]
    fn caption_only_image_renders_as_italic_line() {
        // No exported file, but a VLM caption — surface it rather than drop it.
        let md = to_markdown(&doc(vec![img(None, Some("A flow diagram."))]));
        assert!(md.contains("*A flow diagram.*"), "got: {md}");
        let txt = to_text(&doc(vec![img(None, Some("A flow diagram."))]));
        assert!(txt.contains("A flow diagram."), "got: {txt}");
    }

    #[test]
    fn image_without_file_or_caption_is_not_rendered() {
        let md = to_markdown(&doc(vec![img(None, None)]));
        assert!(!md.contains("!["), "no image syntax: {md}");
    }

    #[test]
    fn table_renders_between_paragraphs_in_markdown_and_text() {
        let d = doc(vec![
            crate::ir::Element::Text(crate::ir::TextChunk {
                text: "A paragraph above the table.".into(),
                bbox: BBox {
                    x0: 10.0,
                    y0: 700.0,
                    x1: 300.0,
                    y1: 720.0,
                },
                font_size: 10.0,
                font: None,
                page: 1,
                confidence: 1.0,
                bold: false,
                hidden: false,
                source: None,
                group: None,
                tag: None,
            }),
            crate::ir::Element::Table(crate::ir::Table {
                bbox: BBox {
                    x0: 10.0,
                    y0: 500.0,
                    x1: 300.0,
                    y1: 600.0,
                },
                page: 1,
                rows: vec![vec![crate::ir::Cell {
                    text: "head".into(),
                    bbox: BBox {
                        x0: 10.0,
                        y0: 590.0,
                        x1: 50.0,
                        y1: 600.0,
                    },
                    row_span: 1,
                    col_span: 1,
                    merged: false,
                }]],
                source: None,
            }),
            crate::ir::Element::Text(crate::ir::TextChunk {
                text: "B paragraph below the table.".into(),
                bbox: BBox {
                    x0: 10.0,
                    y0: 400.0,
                    x1: 300.0,
                    y1: 420.0,
                },
                font_size: 10.0,
                font: None,
                page: 1,
                confidence: 1.0,
                bold: false,
                hidden: false,
                source: None,
                group: None,
                tag: None,
            }),
        ]);
        let md = to_markdown(&d);
        let a = md.find("A paragraph").expect("A in md");
        let t = md.find("| head |").expect("table row in md");
        let b = md.find("B paragraph").expect("B in md");
        assert!(a < t && t < b, "table must sit between paragraphs: {md}");

        let txt = to_text(&d);
        let ta = txt.find("A paragraph").expect("A in text");
        let tt = txt.find("head").expect("table cell in text");
        let tb = txt.find("B paragraph").expect("B in text");
        assert!(
            ta < tt && tt < tb,
            "table must sit between paragraphs: {txt}"
        );
    }

    #[test]
    fn no_floats_output_keeps_block_order() {
        let d = doc(vec![
            crate::ir::Element::Text(crate::ir::TextChunk {
                text: "First paragraph.".into(),
                bbox: BBox {
                    x0: 10.0,
                    y0: 700.0,
                    x1: 300.0,
                    y1: 720.0,
                },
                font_size: 10.0,
                font: None,
                page: 1,
                confidence: 1.0,
                bold: false,
                hidden: false,
                source: None,
                group: None,
                tag: None,
            }),
            crate::ir::Element::Text(crate::ir::TextChunk {
                text: "Second paragraph.".into(),
                bbox: BBox {
                    x0: 10.0,
                    y0: 600.0,
                    x1: 300.0,
                    y1: 620.0,
                },
                font_size: 10.0,
                font: None,
                page: 1,
                confidence: 1.0,
                bold: false,
                hidden: false,
                source: None,
                group: None,
                tag: None,
            }),
        ]);
        let md = to_markdown(&d);
        let a = md.find("First paragraph").unwrap();
        let b = md.find("Second paragraph").unwrap();
        assert!(a < b, "plain text keeps block order: {md}");
    }

    fn tbl(cells: Vec<Vec<&str>>, y0: f32, y1: f32) -> Element {
        Element::Table(crate::ir::Table {
            bbox: BBox {
                x0: 10.0,
                y0,
                x1: 300.0,
                y1,
            },
            page: 1,
            rows: cells
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|t| crate::ir::Cell {
                            text: t.into(),
                            bbox: BBox {
                                x0: 10.0,
                                y0,
                                x1: 50.0,
                                y1,
                            },
                            row_span: 1,
                            col_span: 1,
                            merged: false,
                        })
                        .collect()
                })
                .collect(),
            source: None,
        })
    }

    #[test]
    fn headerless_table_renders_without_fake_header() {
        let d = doc(vec![tbl(
            vec![vec!["", "32", "5.01"], vec!["2", "", "6.11"]],
            400.0,
            500.0,
        )]);
        let md = to_markdown(&d);
        assert!(!md.contains("| --- |"), "no fake header line: {md}");
        assert!(
            md.contains("continued table without header"),
            "comment present: {md}"
        );
        assert!(md.contains("|  | 32 | 5.01 |"), "data row present: {md}");
        let txt = to_text(&d);
        assert!(
            txt.contains("[table without header]"),
            "text marks table: {txt}"
        );
    }

    #[test]
    fn headerless_table_inherits_previous_header() {
        // Two adjacent tables, same column count; the second has no header.
        let d = doc(vec![
            tbl(
                vec![vec!["Model", "BLEU"], vec!["base", "27.3"]],
                500.0,
                600.0,
            ),
            tbl(vec![vec!["2", "6.11"], vec!["4", "5.19"]], 400.0, 450.0),
        ]);
        let md = to_markdown(&d);
        assert!(
            md.contains("continued table: header inherited"),
            "inheritance comment: {md}"
        );
        assert!(md.contains("| Model | BLEU |"), "inherited header: {md}");
        assert!(md.contains("| --- | --- |"), "separator after header: {md}");
        assert!(md.contains("| 2 | 6.11 |"), "continued rows: {md}");
        let txt = to_text(&d);
        assert!(
            txt.contains("[continued table] Model\tBLEU"),
            "text inheritance: {txt}"
        );
    }

    #[test]
    fn mismatched_columns_do_not_inherit() {
        let d = doc(vec![
            tbl(
                vec![vec!["Model", "BLEU"], vec!["base", "27.3"]],
                500.0,
                600.0,
            ),
            tbl(
                vec![vec!["2", "6.11", "x"], vec!["4", "5.19", "y"]],
                400.0,
                450.0,
            ),
        ]);
        let md = to_markdown(&d);
        assert!(
            md.contains("column count differs"),
            "mismatch comment: {md}"
        );
        // The first table's own header appears exactly once — the second
        // table does not inherit it.
        assert_eq!(
            md.matches("| Model | BLEU |").count(),
            1,
            "no inherited header: {md}"
        );
    }
}
