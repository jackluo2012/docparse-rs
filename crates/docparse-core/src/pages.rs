//! Page-range selection (`--pages "1-5,10,20-"`), shared by all interfaces.
//!
//! The spec is 1-based, endpoint-inclusive, comma-separated; `N`, `N-M` and
//! the open-ended `N-` are supported. Two-phase by design: [`PageSpec::parse`]
//! checks syntax only (no document bounds known yet — REST validates before
//! the heavy work), [`PageSpec::resolve`] bounds-checks against the real page
//! count (an explicit page beyond the document is an **error**, never a silent
//! truncate — the project's "don't swallow data" rule).
//!
//! [`retain_pages`] filters a parsed `Document` *after parsing, before
//! enhancement*: every format backend benefits for free, and the model
//! enhancers (OCR / layout / UniRec) only ever see the kept pages — saving
//! model cost is the point of a page range. Page numbers keep their absolute
//! document numbering (no remap): chunks and outline keep citing "page 37"
//! so downstream references survive across differently-ranged calls.

use std::collections::BTreeSet;

use anyhow::{anyhow, bail};

use crate::ir::Document;

/// One syntactic piece of a `--pages` spec: `N`, `N-M`, or the open `N-`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PageRange {
    Single(usize),
    Through(usize, usize),
    From(usize),
}

/// A parsed `--pages` spec: syntax checked, not yet bounds-checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSpec {
    ranges: Vec<PageRange>,
}

impl PageSpec {
    /// Parse the spec syntax. 1-based and endpoint-inclusive; `0` is invalid
    /// (page numbers start at 1) and so is `start > end` or any non-numeric
    /// junk — all with a message that echoes the offending spec.
    pub fn parse(spec: &str) -> anyhow::Result<Self> {
        let mut ranges = Vec::new();
        for part in spec.split(',') {
            let part = part.trim();
            if part.is_empty() {
                bail!("--pages \"{spec}\": empty range item");
            }
            if let Some(start) = part.strip_suffix('-') {
                let start = parse_page(start, spec)?;
                ranges.push(PageRange::From(start));
            } else if let Some((a, b)) = part.split_once('-') {
                let start = parse_page(a, spec)?;
                let end = parse_page(b, spec)?;
                if start > end {
                    bail!("--pages \"{spec}\": range {start}-{end} starts after it ends");
                }
                ranges.push(PageRange::Through(start, end));
            } else {
                ranges.push(PageRange::Single(parse_page(part, spec)?));
            }
        }
        Ok(Self { ranges })
    }

    /// Bounds-check against a real page count (1-based) and expand to the
    /// concrete set of kept page numbers. Explicit pages beyond `total` are
    /// an error; only the open-ended `N-` clamps to the document end.
    pub fn resolve(&self, total: usize) -> anyhow::Result<BTreeSet<usize>> {
        let mut keep = BTreeSet::new();
        for range in &self.ranges {
            match *range {
                PageRange::Single(n) => {
                    if n > total {
                        bail!("--pages: page {n} is out of range (document has {total} pages)");
                    }
                    keep.insert(n);
                }
                PageRange::Through(start, end) => {
                    if end > total {
                        bail!("--pages: page {end} is out of range (document has {total} pages)");
                    }
                    keep.extend(start..=end);
                }
                PageRange::From(start) => {
                    if start > total {
                        bail!("--pages: page {start} is out of range (document has {total} pages)");
                    }
                    keep.extend(start..=total);
                }
            }
        }
        Ok(keep)
    }
}

fn parse_page(s: &str, spec: &str) -> anyhow::Result<usize> {
    s.trim()
        .parse::<usize>()
        .ok()
        .filter(|n| *n >= 1)
        .ok_or_else(|| anyhow!("--pages \"{spec}\": \"{s}\" is not a positive page number"))
}

/// Filter a document to the given absolute page numbers, in document order.
/// Page numbers are **not** remapped — chunks/outline keep citing the original
/// numbering, so references stay valid across differently-ranged calls. The
/// caller owns the ordering decision (parse → filter → enhance).
pub fn retain_pages(doc: &mut Document, keep: &BTreeSet<usize>) {
    doc.pages.retain(|p| keep.contains(&p.number));
}

/// Convenience for the CLI/batch path: parse + resolve + filter in one call.
pub fn retain_pages_spec(doc: &mut Document, spec: &str) -> anyhow::Result<()> {
    let keep = PageSpec::parse(spec)?.resolve(doc.pages.len())?;
    retain_pages(doc, &keep);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{BBox, Element, Page, TextChunk};

    fn doc_with_pages(n: usize) -> Document {
        Document {
            source: "test.pdf".into(),
            provenance: None,
            metadata: None,
            pages: (1..=n)
                .map(|i| Page {
                    number: i,
                    width: 612.0,
                    height: 792.0,
                    elements: vec![Element::Text(TextChunk {
                        text: format!("p{i}"),
                        bbox: BBox {
                            x0: 0.0,
                            y0: 0.0,
                            x1: 10.0,
                            y1: 10.0,
                        },
                        font_size: 12.0,
                        font: None,
                        page: i,
                        confidence: 1.0,
                        bold: false,
                        hidden: false,
                        source: None,
                        group: None,
                        tag: None,
                    })],
                })
                .collect(),
        }
    }

    fn kept(spec: &str, total: usize) -> Vec<usize> {
        PageSpec::parse(spec)
            .unwrap()
            .resolve(total)
            .unwrap()
            .into_iter()
            .collect()
    }

    fn err_of(spec: &str, total: usize) -> String {
        PageSpec::parse(spec)
            .unwrap()
            .resolve(total)
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn spec_syntax_basics() {
        assert_eq!(kept("3", 10), vec![3]);
        assert_eq!(kept("1-3", 10), vec![1, 2, 3]);
        assert_eq!(kept("1-3,7", 10), vec![1, 2, 3, 7]);
        assert_eq!(kept("5-", 10), vec![5, 6, 7, 8, 9, 10]);
        assert_eq!(
            kept("7,1-3", 10),
            vec![1, 2, 3, 7],
            "set order, not spec order"
        );
        assert_eq!(
            kept(" 2 , 4 - 6 ", 10),
            vec![2, 4, 5, 6],
            "whitespace tolerated"
        );
    }

    #[test]
    fn spec_rejects_bad_syntax() {
        for spec in ["0", "3-1", "abc", "", "1,,2", "1-2-3", "-4", "1.5"] {
            assert!(PageSpec::parse(spec).is_err(), "\"{spec}\" must not parse");
        }
        // Messages echo the offending spec.
        assert!(PageSpec::parse("3-1")
            .unwrap_err()
            .to_string()
            .contains("3-1"));
        // Open-ended with no start is the `-4` case — rejected, not "from 1".
        assert!(PageSpec::parse("-4").is_err());
    }

    #[test]
    fn resolve_bounds_check() {
        assert_eq!(kept("5-", 5), vec![5], "open start == last page keeps it");
        assert_eq!(
            kept("5-", 8),
            vec![5, 6, 7, 8],
            "open end clamps to the document"
        );
        // An explicit page (or open start) beyond the document errors — never
        // a silent truncate to an empty output.
        let msg = err_of("20-", 5);
        assert!(msg.contains("20") && msg.contains("5"), "{msg}");
        let msg = err_of("999", 12);
        assert!(msg.contains("999") && msg.contains("12"), "{msg}");
        assert!(err_of("1-999", 12).contains("out of range"));
    }

    #[test]
    fn retain_keeps_absolute_numbering() {
        let mut doc = doc_with_pages(10);
        retain_pages_spec(&mut doc, "4-5,9").unwrap();
        assert_eq!(
            doc.pages.iter().map(|p| p.number).collect::<Vec<_>>(),
            vec![4, 5, 9]
        );
        // Chunks still cite the original page — no remap.
        let pages: Vec<usize> = doc.pages[0]
            .elements
            .iter()
            .filter_map(|e| match e {
                Element::Text(t) => Some(t.page),
                _ => None,
            })
            .collect();
        assert_eq!(pages, vec![4]);
    }

    #[test]
    fn retain_errors_do_not_touch_the_document() {
        let mut doc = doc_with_pages(3);
        let before = doc.pages.len();
        assert!(retain_pages_spec(&mut doc, "99").is_err());
        assert_eq!(
            doc.pages.len(),
            before,
            "failed spec leaves the doc untouched"
        );
    }
}
