//! PDF-specific OCR input adapter.
//!
//! Embedded scan pixels remain the fast path. Some valid PDFs expose only
//! position-only images, while others paint their visible content with vector
//! paths and contain no page image at all. A page routed to OCR can therefore
//! lack pixels even when it is visually non-empty. This adapter renders only
//! those routed pages, injects one temporary full-page RGB image, delegates to
//! the existing OCR enhancer, then removes the temporary image from the IR.

use crate::PpOcrEnhancer;
use docparse_core::enhance::{self, Enhancer, PageRoute};
use docparse_core::ir::{BBox, Document, Element, ImageChunk, ImageKind, Page};
use docparse_raster::Rasterizer;
use std::sync::Mutex;

const RASTER_SCALE: f32 = 2.0;
const TEMP_IMAGE_MARKER: &str = "\0docparse-pdf-ocr-raster";

/// Apply quality-routed OCR to a PDF, rendering only routed pages that do not
/// already carry usable embedded pixels. Pages with machine-readable text are
/// filtered out by the shared quality router before this adapter is invoked.
/// Render failures degrade to the original OCR path so deterministic parsing
/// still succeeds.
pub fn apply_with(
    doc: &Document,
    pdf_bytes: Vec<u8>,
    ocr: &PpOcrEnhancer,
    on_page: Option<&(dyn Fn() + Sync)>,
) -> (Document, Vec<PageRoute>) {
    let rasterizer = match Rasterizer::new(pdf_bytes) {
        Ok(rasterizer) => rasterizer,
        Err(error) => {
            eprintln!("pdf-ocr: raster fallback unavailable: {error:#}");
            return enhance::apply_with(doc, &[ocr as &dyn Enhancer], on_page);
        }
    };
    let adapter = PdfRasterOcr {
        ocr,
        rasterizer: Mutex::new(rasterizer),
    };
    enhance::apply_with(doc, &[&adapter as &dyn Enhancer], on_page)
}

struct PdfRasterOcr<'a> {
    ocr: &'a PpOcrEnhancer,
    rasterizer: Mutex<Rasterizer>,
}

impl Enhancer for PdfRasterOcr<'_> {
    fn capability(&self) -> docparse_core::enhance::Capability {
        self.ocr.capability()
    }

    fn enhance_page(&self, page: &Page) -> Option<Page> {
        if page
            .elements
            .iter()
            .any(|element| matches!(element, Element::Image(image) if is_usable_ocr_image(image)))
        {
            return self.ocr.enhance_page(page);
        }

        let page_index = page.number.checked_sub(1)?;
        let rendered = self
            .rasterizer
            .lock()
            .ok()
            .and_then(|rasterizer| rasterizer.render_rgb(page_index, RASTER_SCALE).ok());
        let Some((width, height, rgb)) = rendered else {
            eprintln!("pdf-ocr: raster fallback failed on page {}", page.number);
            return None;
        };
        enhance_with_raster(self.ocr, page, width, height, rgb)
    }
}

pub(crate) fn is_usable_ocr_image(image: &ImageChunk) -> bool {
    let width = image.width_px as usize;
    let height = image.height_px as usize;
    if width == 0 || height == 0 {
        return false;
    }
    match image.kind {
        ImageKind::Rgb8 => image.data.len() == width.saturating_mul(height).saturating_mul(3),
        ImageKind::Gray8 => image.data.len() == width.saturating_mul(height),
        ImageKind::Jpeg => !image.data.is_empty(),
        ImageKind::None | ImageKind::Encoded => false,
    }
}

fn enhance_with_raster(
    enhancer: &dyn Enhancer,
    page: &Page,
    width: u32,
    height: u32,
    rgb: Vec<u8>,
) -> Option<Page> {
    let mut staged = page.clone();
    staged.elements.push(Element::Image(ImageChunk {
        bbox: BBox {
            x0: 0.0,
            y0: 0.0,
            x1: page.width,
            y1: page.height,
        },
        page: page.number,
        width_px: width,
        height_px: height,
        turns: 0,
        kind: ImageKind::Rgb8,
        data: rgb,
        file: Some(TEMP_IMAGE_MARKER.into()),
        data_base64: None,
        data_media_type: None,
        caption: None,
        caption_source: None,
    }));
    let mut enhanced = enhancer.enhance_page(&staged)?;
    enhanced.elements.retain(|element| {
        !matches!(element, Element::Image(image) if image.file.as_deref() == Some(TEMP_IMAGE_MARKER))
    });
    Some(enhanced)
}

#[cfg(test)]
mod tests {
    use super::*;
    use docparse_core::enhance::Capability;
    use docparse_core::ir::TextChunk;

    struct StubOcr;

    impl Enhancer for StubOcr {
        fn capability(&self) -> Capability {
            Capability {
                name: "stub-ocr".into(),
                version: "test".into(),
                handles_scanned: true,
                handles_garbled: false,
            }
        }

        fn enhance_page(&self, page: &Page) -> Option<Page> {
            let raster = page.elements.iter().find_map(|element| match element {
                Element::Image(image) if image.file.as_deref() == Some(TEMP_IMAGE_MARKER) => {
                    Some(image)
                }
                _ => None,
            })?;
            assert_eq!((raster.width_px, raster.height_px), (2, 3));
            assert_eq!(raster.data.len(), 18);

            let mut enhanced = page.clone();
            enhanced.elements.push(Element::Text(TextChunk {
                text: "recovered".into(),
                bbox: BBox {
                    x0: 1.0,
                    y0: 1.0,
                    x1: 10.0,
                    y1: 10.0,
                },
                font_size: 9.0,
                font: None,
                page: page.number,
                confidence: 0.8,
                bold: false,
                hidden: false,
                source: Some("ocr:test".into()),
                group: None,
                tag: None,
            }));
            Some(enhanced)
        }
    }

    fn empty_page() -> Page {
        Page {
            number: 1,
            width: 100.0,
            height: 200.0,
            elements: Vec::new(),
        }
    }

    #[test]
    fn temporary_raster_is_available_to_ocr_but_absent_from_output() {
        let enhanced = enhance_with_raster(&StubOcr, &empty_page(), 2, 3, vec![255; 18])
            .expect("stub should recover text");

        assert!(enhanced
            .elements
            .iter()
            .any(|element| { matches!(element, Element::Text(text) if text.text == "recovered") }));
        assert!(!enhanced.elements.iter().any(|element| {
            matches!(element, Element::Image(image) if image.file.as_deref() == Some(TEMP_IMAGE_MARKER))
        }));
    }

    #[test]
    fn position_only_image_is_not_a_usable_ocr_source() {
        let image = ImageChunk {
            bbox: BBox {
                x0: 0.0,
                y0: 0.0,
                x1: 100.0,
                y1: 200.0,
            },
            page: 1,
            width_px: 100,
            height_px: 200,
            turns: 0,
            kind: ImageKind::None,
            data: Vec::new(),
            file: None,
            data_base64: None,
            data_media_type: None,
            caption: None,
            caption_source: None,
        };

        assert!(!is_usable_ocr_image(&image));
    }
}
