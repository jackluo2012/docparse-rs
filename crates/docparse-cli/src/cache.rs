//! Incremental parse cache (`--cache-dir <DIR>`), shared by the batch CLI and
//! the serving faces (REST `/parse`, MCP tools).
//!
//! The RAG-corpus workflow re-runs the same documents repeatedly; files rarely
//! change between runs, yet every run re-pays the heavy cost (OCR / layout /
//! UniRec inference, page rasterization). The cache keys a stored payload by
//! `(identity, content SHA-256, output signature)` and, on a hit, replays it —
//! parsing is skipped entirely.
//!
//! The payload differs by face: the batch CLI stores the **rendered output**
//! (the whole per-file result); the serving faces store the **enhanced
//! document** (the shared heavy step behind every format and tool — rendering
//! happens after the lookup, so format/tool arguments never fragment the
//! cache). Both reuse [`CacheEntry`]; the signatures differ, so keys never
//! collide across faces.
//!
//! Correctness contract: **the key is the whole story.** A content change or
//! an output-affecting flag change mints a fresh cache file name, so a hit can
//! only ever replay bytes that are provably right for the current
//! identity+content+options triple. Stale entries linger on disk but are never
//! consulted again; no index, no invalidation pass.
//!
//! Cache writes are best-effort: a failed store degrades to a re-parse next
//! run and never fails the batch or the request.

use anyhow::Context as _;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::Cli;

/// One cache entry: the stored payload plus the metadata the caller needs
/// without re-parsing.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct CacheEntry {
    /// Entry format version (bump to invalidate every entry).
    pub v: u32,
    /// Content SHA-256 this entry was built from — checked on read.
    pub sha256: String,
    /// Page count (the batch report shows it; re-parsing just for that is
    /// silly).
    pub pages: usize,
    /// The stored payload (UTF-8): the rendered output for batch, the
    /// enhanced document JSON for the serving faces.
    pub output: String,
}

const VERSION: u32 = 1;

impl CacheEntry {
    pub fn new(sha256: String, pages: usize, output: String) -> Self {
        Self {
            v: VERSION,
            sha256,
            pages,
            output,
        }
    }
}

/// SHA-256 (hex) of a file's contents — the source identity. Content, not
/// mtime/size: a touch must not masquerade as "unchanged", and a content
/// change must never replay an old parse.
pub fn content_sha(path: &Path) -> anyhow::Result<String> {
    let mut file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .with_context(|| format!("read {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Canonical string of every option that can change a rendered output. **Any
/// new output-affecting CLI flag MUST be added here**, or a changed flag
/// would replay stale bytes. Stderr-only observability flags (--quality /
/// --profile / --route-plan / --progress / --stats / …) and batch mechanics
/// (--out-dir / --jobs / --report-* / --recursive) are deliberately excluded:
/// they change nothing in the cached output.
pub fn output_signature(cli: &Cli) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = write!(s, "format={:?}", cli.format);
    let _ = write!(s, ";table={:?}", cli.table_format);
    let _ = write!(s, ";chunk_target={}", cli.chunk_target_chars);
    let _ = write!(s, ";password={:?}", cli.password);
    // --pages: appended only when set, so signatures minted before the flag
    // existed stay valid (no gratuitous whole-cache invalidation).
    if let Some(pages) = &cli.pages {
        let _ = write!(s, ";pages={pages}");
    }
    let _ = write!(
        s,
        ";ocr={};ocr_models={}",
        cli.ocr,
        cli.ocr_models.display()
    );
    let _ = write!(
        s,
        ";layout={};layout_model={}",
        cli.layout,
        cli.layout_model.display()
    );
    let _ = write!(
        s,
        ";vlm_describe={};vlm_tables={};vlm_url={:?};vlm_model={:?}",
        cli.vlm_describe, cli.vlm_tables, cli.vlm_url, cli.vlm_model
    );
    let _ = write!(s, ";table_model={:?}", cli.table_model);
    let _ = write!(s, ";formula_model={:?}", cli.formula_model);
    let _ = write!(s, ";transcribe_model={:?}", cli.transcribe_model);
    let _ = write!(
        s,
        ";image_embed={};image_dir={:?}",
        cli.image_embed, cli.image_dir
    );
    s
}

/// Cache file name for a key triple: SHA-256 of `identity|sha|sig`. Any
/// change mints a fresh name. The identity is the caller's notion of "which
/// document this is": the absolute path for the CLI/MCP faces (stable,
/// user-supplied), or the sanitized upload name for REST (the staging temp
/// path is random per request and would poison the key).
fn cache_file(cache_dir: &Path, identity: &str, sha: &str, sig: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(identity.as_bytes());
    hasher.update(b"\n");
    hasher.update(sha.as_bytes());
    hasher.update(b"\n");
    hasher.update(sig.as_bytes());
    cache_dir.join(format!("{}.json", hex(&hasher.finalize())))
}

/// Read a cache entry, verifying the format version and that it was built
/// from the same content hash (defense in depth — the file name already
/// encodes it). Any mismatch (missing file, truncated JSON, wrong version,
/// wrong hash) is a miss: `None`.
pub fn lookup(cache_dir: &Path, path: &Path, sha: &str, sig: &str) -> Option<CacheEntry> {
    lookup_identity(cache_dir, &path.display().to_string(), sha, sig)
}

/// Path-free variant for the serving faces: `identity` is whatever stays
/// stable across requests for the same logical document (the sanitized upload
/// name for REST — it appears in the output — or the caller-provided path for
/// MCP).
pub fn lookup_identity(
    cache_dir: &Path,
    identity: &str,
    sha: &str,
    sig: &str,
) -> Option<CacheEntry> {
    let file = cache_file(cache_dir, identity, sha, sig);
    let bytes = std::fs::read(&file).ok()?;
    let entry: CacheEntry = serde_json::from_slice(&bytes).ok()?;
    if entry.v != VERSION || entry.sha256 != sha {
        return None;
    }
    Some(entry)
}

/// Write a cache entry atomically (temp + rename). Concurrent writers (--jobs,
/// or parallel serving requests) race only on the rename, which is atomic: one
/// wins, the other's bytes are dropped whole — never a torn file. Best-effort
/// at the call site.
pub fn store(
    cache_dir: &Path,
    path: &Path,
    sha: &str,
    sig: &str,
    entry: &CacheEntry,
) -> anyhow::Result<()> {
    store_identity(cache_dir, &path.display().to_string(), sha, sig, entry)
}

/// Path-free variant of [`store`]; see [`lookup_identity`] for the identity
/// contract.
pub fn store_identity(
    cache_dir: &Path,
    identity: &str,
    sha: &str,
    sig: &str,
    entry: &CacheEntry,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("create {}", cache_dir.display()))?;
    let target = cache_file(cache_dir, identity, sha, sig);
    let tmp = cache_dir.join(format!(
        ".{}.partial",
        target.file_name().unwrap_or_default().to_string_lossy()
    ));
    let bytes = serde_json::to_vec(entry)?;
    std::fs::write(&tmp, &bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, &target).with_context(|| format!("install {}", target.display()))?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;
    use std::process::id;

    /// A unique scratch dir under the system temp, pid-suffixed (repo
    /// convention — no tempfile crate in the tree).
    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("docparse-cache-{}-{tag}", id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn cli(args: &[&str]) -> Cli {
        let mut full = vec!["docparse", "in.pdf"];
        full.extend_from_slice(args);
        Cli::try_parse_from(full).expect("cli parses")
    }

    #[test]
    fn content_sha_is_stable_and_content_sensitive() {
        let d = scratch("sha");
        let p = d.join("a.txt");
        std::fs::write(&p, "hello").unwrap();
        let s1 = content_sha(&p).unwrap();
        let s2 = content_sha(&p).unwrap();
        assert_eq!(s1, s2, "deterministic");
        std::fs::write(&p, "hello!").unwrap();
        assert_ne!(s1, content_sha(&p).unwrap(), "content change -> new hash");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn signature_reacts_to_output_flags() {
        let base = cli(&[]);
        assert_ne!(
            output_signature(&base),
            output_signature(&cli(&["-f", "markdown"]))
        );
        assert_ne!(
            output_signature(&base),
            output_signature(&cli(&["--chunk-target-chars", "400"]))
        );
        assert_ne!(output_signature(&base), output_signature(&cli(&["--ocr"])));
        assert_ne!(
            output_signature(&base),
            output_signature(&cli(&["--layout"]))
        );
        assert_ne!(
            output_signature(&base),
            output_signature(&cli(&["--image-embed"]))
        );
        assert_ne!(
            output_signature(&base),
            output_signature(&cli(&["--password", "x"]))
        );
        // --pages changes the rendered output, so it must change the signature…
        assert_ne!(
            output_signature(&base),
            output_signature(&cli(&["--pages", "1-5"]))
        );
        // …and two different specs must not collide.
        assert_ne!(
            output_signature(&cli(&["--pages", "1-5"])),
            output_signature(&cli(&["--pages", "2-5"]))
        );
        // Stderr-only / batch-mechanics flags must NOT change the signature.
        assert_eq!(
            output_signature(&base),
            output_signature(&cli(&["--quality"]))
        );
        assert_eq!(
            output_signature(&base),
            output_signature(&cli(&["--profile"]))
        );
        assert_eq!(
            output_signature(&base),
            output_signature(&cli(&["--jobs", "8"]))
        );
        assert_eq!(
            output_signature(&base),
            output_signature(&cli(&["--out-dir", "o"]))
        );
    }

    #[test]
    fn store_lookup_roundtrip_and_misses() {
        let d = scratch("rt");
        let p = Path::new("/tmp/some/report.pdf");
        let sha = "a".repeat(64);
        let sig = "format=Json";
        let entry = CacheEntry::new(sha.clone(), 7, "{\"pages\":7}".to_string());
        store(&d, p, &sha, sig, &entry).unwrap();

        // Exact triple hits.
        let hit = lookup(&d, p, &sha, sig).expect("roundtrip hit");
        assert_eq!(hit.pages, 7);
        assert_eq!(hit.output, "{\"pages\":7}");
        assert_eq!(hit.sha256, sha);

        // Content change -> miss (new name, old entry untouched).
        let sha2 = "b".repeat(64);
        assert!(lookup(&d, p, &sha2, sig).is_none());
        // Different output signature -> miss.
        assert!(lookup(&d, p, &sha, "format=Markdown").is_none());
        // Missing cache dir -> miss, never an error.
        assert!(lookup(&d.join("nope"), p, &sha, sig).is_none());

        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn identity_variants_share_one_mechanism() {
        let d = scratch("idv");
        let identity = "report.pdf";
        let sha = "c".repeat(64);
        let sig = "srv1;ocr=false";
        let entry = CacheEntry::new(sha.clone(), 3, "{\"pages\":3}".to_string());
        store_identity(&d, identity, &sha, sig, &entry).unwrap();

        // Same identity + sha + sig hits.
        assert_eq!(
            lookup_identity(&d, identity, &sha, sig).expect("hit").pages,
            3
        );
        // A different identity (e.g. another upload name for the same bytes)
        // misses — the serving faces key on it because it appears in output.
        assert!(lookup_identity(&d, "other.pdf", &sha, sig).is_none());
        // The path-based wrapper agrees with its identity expansion.
        let p = Path::new("/tmp/some/report.pdf");
        store(&d, p, &sha, sig, &entry).unwrap();
        assert!(lookup(&d, p, &sha, sig).is_some());
        assert!(
            lookup(&d, Path::new("/tmp/some/other.pdf"), &sha, sig).is_none(),
            "path wrapper keys on the absolute path"
        );

        let _ = std::fs::remove_dir_all(&d);
    }
}
