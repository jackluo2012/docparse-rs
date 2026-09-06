//! PDF Info-dictionary metadata: trailer `/Info` → [`Document.metadata`].
//!
//! The Info dictionary is the PDF's document-properties record (title,
//! author, dates, creating/producing applications). Everything here is
//! best-effort by contract: a missing or unreadable `/Info` yields `None`
//! (no `metadata` in the output), a missing field stays `None`, and a
//! malformed date string drops that one field rather than failing the parse.
//! Dates are normalized to ISO 8601 UTC (`D:20260906…` → `2026-09-06T…`),
//! applying the embedded UTC offset when present.
//!
//! Strings decode like bookmark titles (see `outlines::decode_pdf_string`):
//! UTF-16BE when they carry the BOM, else Latin-1-ish byte-per-char.

use lopdf::Document as PdfDocument;

use docparse_core::ir::Metadata;

/// Read the trailer's `/Info` dictionary into a [`Metadata`]. `None` when the
/// document has no Info dictionary at all.
pub fn info_metadata(doc: &PdfDocument) -> Option<Metadata> {
    let info_ref = doc.trailer.get(b"Info").ok()?;
    let info_obj = doc.get_object(info_ref.as_reference().ok()?).ok()?;
    let info = info_obj.as_dict().ok()?.clone();

    let mut meta = Metadata::default();
    for (key, slot) in [
        (b"Title" as &[u8], &mut meta.title as &mut Option<String>),
        (b"Author", &mut meta.author),
        (b"Subject", &mut meta.subject),
        (b"Keywords", &mut meta.keywords),
        (b"Creator", &mut meta.creator),
        (b"Producer", &mut meta.producer),
    ] {
        if let Some(text) = info_string(&info, key) {
            *slot = Some(text);
        }
    }
    if let Some(date) = info_string(&info, b"CreationDate")
        .as_deref()
        .and_then(pdf_date_to_iso)
    {
        meta.created = Some(date);
    }
    if let Some(date) = info_string(&info, b"ModDate")
        .as_deref()
        .and_then(pdf_date_to_iso)
    {
        meta.modified = Some(date);
    }
    Some(meta)
}

/// A string-valued Info entry, decoded (UTF-16BE / PDFDocEncoding). Empty
/// values read as absent.
fn info_string(info: &lopdf::Dictionary, key: &[u8]) -> Option<String> {
    let obj = info.get(key).ok()?;
    let bytes = obj.as_str().ok()?;
    let text = decode_pdf_string(bytes);
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Decode a PDF string: UTF-16BE when it carries a BOM, else one byte per
/// char (PDFDocEncoding ≈ Latin-1 for the common range) — same convention as
/// `outlines::decode_pdf_string`.
fn decode_pdf_string(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|p| u16::from_be_bytes([p[0], p[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        bytes.iter().map(|&b| b as char).collect()
    }
}

/// Normalize a PDF date string (`D:YYYYMMDDHHmmSSOHH'mm'`, all but the year
/// optional; `O` is `+`/`-`/`Z` with the UTC offset) to ISO 8601 UTC
/// (`2026-09-06T12:34:56Z`). Returns `None` for anything unparsable — the
/// field is simply absent, never guessed.
pub fn pdf_date_to_iso(raw: &str) -> Option<String> {
    let s = raw.trim();
    let s = s.strip_prefix("D:").unwrap_or(s);
    if s.len() < 4 || !s[..4].bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: i64 = s[..4].parse().ok()?;

    // Each subsequent pair (month, day, hour, minute, second) is optional and
    // must be two digits when present.
    let mut rest = &s[4..];
    let mut parts = [0i64; 5]; // month, day, hh, mm, ss
    let mut offset_minutes: i64 = 0;
    let mut i = 0;
    while i < 5 && !rest.is_empty() {
        let b = rest.as_bytes()[0];
        if !b.is_ascii_digit() {
            break;
        }
        if rest.len() < 2 {
            return None;
        }
        parts[i] = rest[..2].parse().ok()?;
        rest = &rest[2..];
        i += 1;
    }

    // Optional UTC offset: +HH'mm / -HH'mm / Z (or an apostrophe-terminated
    // trailing artifact). Applied so the timestamp is honest UTC.
    let mut chars = rest.chars();
    match chars.next() {
        Some('Z') | None | Some('\'') => {}
        Some(sign @ ('+' | '-')) => {
            let r = chars.as_str();
            let hh: i64 = r.get(0..2)?.parse().ok()?;
            let mm = r
                .get(3..5)
                .and_then(|m| m.parse().ok())
                .or_else(|| r.get(2..4).and_then(|m| m.parse().ok()))
                .unwrap_or(0);
            offset_minutes = hh * 60 + mm;
            if sign == '-' {
                offset_minutes = -offset_minutes;
            }
        }
        _ => {}
    }

    let (month, day, hh, mm, ss) = (parts[0], parts[1], parts[2], parts[3], parts[4]);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // civil date → seconds since epoch (Howard Hinnant's days_from_civil).
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    let secs = days * 86_400 + hh * 3600 + mm * 60 + ss - offset_minutes * 60;
    let rem = secs.rem_euclid(86_400);
    // civil_from_days: back to a Y/M/D for the (possibly offset-shifted) date.
    let z = secs.div_euclid(86_400) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let yy = yoe + era * 400;
    let ddd = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * ddd + 2) / 153;
    let dd = ddd - (153 * mp + 2) / 5 + 1;
    let mm2 = if mp < 10 { mp + 3 } else { mp - 9 };
    let yy = if mm2 <= 2 { yy + 1 } else { yy };
    Some(format!(
        "{yy:04}-{mm2:02}-{dd:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_normalize_to_iso() {
        assert_eq!(
            pdf_date_to_iso("D:20260906123456"),
            Some("2026-09-06T12:34:56Z".into())
        );
        assert_eq!(
            pdf_date_to_iso("D:20260906"),
            Some("2026-09-06T00:00:00Z".into())
        );
        assert_eq!(
            pdf_date_to_iso("20260906"),
            Some("2026-09-06T00:00:00Z".into())
        );
        // UTC offsets apply.
        assert_eq!(
            pdf_date_to_iso("D:20260906120000+02'00'"),
            Some("2026-09-06T10:00:00Z".into())
        );
        assert_eq!(
            pdf_date_to_iso("D:20260906230000-03'30'"),
            Some("2026-09-07T02:30:00Z".into()),
            "offset shift crosses the date boundary"
        );
        // Garbage → None, never guessed.
        assert_eq!(pdf_date_to_iso(""), None);
        assert_eq!(pdf_date_to_iso("D:notadate"), None);
        assert_eq!(pdf_date_to_iso("D:2026"), None);
        assert_eq!(
            pdf_date_to_iso("D:2026134000000"),
            None,
            "month 13 rejected"
        );
    }

    #[test]
    fn utf16be_strings_decode() {
        assert_eq!(
            decode_pdf_string(&[0xFE, 0xFF, 0x00, 0x48, 0x00, 0x69]),
            "Hi"
        );
        assert_eq!(decode_pdf_string(b"Hi"), "Hi");
    }
}
