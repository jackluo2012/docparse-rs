//! Input materialization for the two non-file input shapes: stdin (`-`) and
//! `http(s)://` URLs.
//!
//! The parser backends consume `&Path` and pick their parser by extension,
//! so both shapes are materialized into a temp file with a meaningful
//! extension and the rest of the pipeline stays untouched (single-file path,
//! rendering, cache semantics). The caller owns the temp file's lifetime —
//! [`TempInput`] removes it on drop, best-effort.
//!
//! Format determination order (most explicit wins): `--input-format` →
//! the `%PDF-` magic (stdin only — the one sniff worth having, it's 5 exact
//! bytes) → URL path suffix → `Content-Type`. Anything else is an error
//! listing the valid formats — no heuristic text sniffing, per the project's
//! "don't guess silently" rule.

use anyhow::{anyhow, Context};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Explicit format override (`--input-format`), mirroring the parser names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum InputFormat {
    Pdf,
    Docx,
    Html,
    Xlsx,
    Pptx,
    Md,
    Csv,
    Srt,
    Tex,
    Eml,
    Img,
    Adoc,
}

impl InputFormat {
    /// The file extension the materialized temp file gets (it drives the
    /// backend's `supports()` dispatch).
    pub fn ext(self) -> &'static str {
        match self {
            InputFormat::Pdf => "pdf",
            InputFormat::Docx => "docx",
            InputFormat::Html => "html",
            InputFormat::Xlsx => "xlsx",
            InputFormat::Pptx => "pptx",
            InputFormat::Md => "md",
            InputFormat::Csv => "csv",
            InputFormat::Srt => "srt",
            InputFormat::Tex => "tex",
            InputFormat::Eml => "eml",
            InputFormat::Img => "png",
            InputFormat::Adoc => "adoc",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        // Accepts the clap kebab names (pdf/docx/html/xlsx/pptx/md/csv/srt/
        // tex/eml/img/adoc) — one source: iterate the ValueEnum set.
        use clap::ValueEnum as _;
        InputFormat::value_variants()
            .iter()
            .find(|f| {
                f.to_possible_value()
                    .map(|v| v.matches(name, false))
                    .unwrap_or(false)
            })
            .copied()
    }
}

/// The list of valid `--input-format` names, for error messages.
fn format_names() -> String {
    use clap::ValueEnum as _;
    InputFormat::value_variants()
        .iter()
        .filter_map(|f| f.to_possible_value())
        .map(|v| v.get_name().to_string())
        .collect::<Vec<_>>()
        .join("|")
}

/// Decide the temp-file extension for a non-file input.
/// `url_path` is the URL's path (empty for stdin); `content_type` is the
/// response's Content-Type (None for stdin).
pub fn pick_extension(
    url_path: &str,
    content_type: Option<&str>,
    explicit: Option<InputFormat>,
) -> anyhow::Result<String> {
    if let Some(f) = explicit {
        return Ok(f.ext().to_string());
    }
    // The reliable stdin sniff: real PDFs begin with the 5 exact bytes.
    if url_path.is_empty() && content_type.is_none() {
        // Handled by the caller reading the first bytes; here just fall
        // through to an error message.
    }
    let suffix = Path::new(url_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    if let Some(ext) = suffix {
        if InputFormat::from_name(&ext).is_some() {
            return Ok(ext.to_string());
        }
    }
    if let Some(ct) = content_type {
        let ct = ct
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let ext = match ct.as_str() {
            "application/pdf" => "pdf",
            "text/html" | "application/xhtml+xml" => "html",
            "text/plain" => "txt",
            "text/markdown" => "md",
            "text/csv" | "application/csv" => "csv",
            "application/json" => "json",
            _ => "",
        };
        if InputFormat::from_name(ext).is_some() {
            return Ok(ext.to_string());
        }
    }
    anyhow::bail!(
        "cannot determine the input format{}{} — pass --input-format <FMT> \
         (one of: {})",
        if url_path.is_empty() {
            String::new()
        } else {
            format!(" for {url_path}")
        },
        content_type
            .map(|c| format!(" (content-type: {c})"))
            .unwrap_or_default(),
        format_names()
    )
}

/// True when the first 5 bytes are the `%PDF-` magic.
pub fn has_pdf_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF-")
}

/// A temp file backing a stdin/URL input; removes it on drop (best-effort —
/// a leaked temp file on an abrupt exit is a missed cleanup, never a
/// correctness issue).
pub struct TempInput(pub PathBuf);

impl Drop for TempInput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A unique-ish temp path with the given extension (pid + nanos; the repo
/// convention of no tempfile crate).
pub fn temp_path(ext: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "docparse-input-{}-{nanos}.{ext}",
        std::process::id()
    ))
}

/// Materialize stdin into a temp file, sniffing `%PDF-` for the common
/// `curl … | docparse -` case and otherwise requiring an explicit format.
pub fn from_stdin(explicit: Option<InputFormat>) -> anyhow::Result<TempInput> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .lock()
        .read_to_end(&mut bytes)
        .context("reading stdin")?;
    let ext = if has_pdf_magic(&bytes) {
        "pdf".to_string()
    } else {
        pick_extension("", None, explicit)?
    };
    let path = temp_path(&ext);
    std::fs::write(&path, &bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(TempInput(path))
}

/// Download a URL into a temp file. Extension: `--input-format` → URL path
/// suffix → Content-Type → error. 30s timeout, up to 5 redirects (ureq
/// default) — a local-CLI trust model, deliberately not exposed as a server
/// feature (SSRF).
pub fn from_url(url: &str, explicit: Option<InputFormat>) -> anyhow::Result<TempInput> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build();
    let resp = agent
        .get(url)
        .call()
        .map_err(|e| anyhow!("download {url}: {e}"))?;
    let content_type = resp.header("content-type").map(str::to_string);
    let ext = pick_extension(url::path_of(url), content_type.as_deref(), explicit)?;
    let path = temp_path(&ext);
    let mut reader = resp.into_reader();
    let mut file =
        std::fs::File::create(&path).with_context(|| format!("creating {}", path.display()))?;
    std::io::copy(&mut reader, &mut file).with_context(|| format!("downloading {url}"))?;
    Ok(TempInput(path))
}

/// URL helpers (host part of the std lib has none — keep it minimal).
mod url {
    /// The URL's percent-encoded path ("" when absent), for suffix sniffing.
    pub fn path_of(url: &str) -> &str {
        let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
        let rest = rest.split_once('/').map(|(_, p)| p).unwrap_or("");
        let path = rest.split(['?', '#']).next().unwrap_or("");
        // Strip a trailing slash: "…/report/" has no extension to read.
        path.trim_end_matches('/')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ext(path: &str, ct: Option<&str>, f: Option<InputFormat>) -> String {
        pick_extension(path, ct, f).unwrap()
    }

    #[test]
    fn explicit_format_wins_over_everything() {
        assert_eq!(
            ext("a.xlsx", Some("application/pdf"), Some(InputFormat::Md)),
            "md"
        );
        assert_eq!(ext("", None, Some(InputFormat::Csv)), "csv");
    }

    #[test]
    fn url_suffix_and_content_type_fallbacks() {
        assert_eq!(ext("https://x.io/r/report.pdf", None, None), "pdf");
        assert_eq!(ext("/tmp/a/notes.MD", None, None), "md", "case-insensitive");
        assert_eq!(
            ext("https://x.io/get", Some("application/pdf"), None),
            "pdf"
        );
        assert_eq!(
            ext("https://x.io/", Some("text/html; charset=utf-8"), None),
            "html"
        );
    }

    #[test]
    fn undeterminable_is_a_clear_error_naming_the_flag() {
        for (path, ct) in [
            ("https://x.io/get", None),
            ("https://x.io/get", Some("application/octet-stream")),
            ("", None),
        ] {
            let err = pick_extension(path, ct, None).unwrap_err().to_string();
            assert!(err.contains("--input-format"), "{err}");
            assert!(err.contains("pdf|docx"), "{err}");
        }
    }

    #[test]
    fn pdf_magic_sniff() {
        assert!(has_pdf_magic(b"%PDF-1.7\n..."));
        assert!(!has_pdf_magic(b"<html>"));
        assert!(!has_pdf_magic(b"%PD"));
    }

    #[test]
    fn url_path_extraction() {
        assert_eq!(
            url::path_of("https://x.io/a/b/report.pdf"),
            "a/b/report.pdf"
        );
        assert_eq!(url::path_of("https://x.io/get?q=1"), "get");
        assert_eq!(url::path_of("https://x.io/"), "");
        assert_eq!(url::path_of("/plain.pdf"), "plain.pdf");
    }
}
