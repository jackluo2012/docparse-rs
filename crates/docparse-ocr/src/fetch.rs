//! One-time model download for the optional neural tiers.
//!
//! The binary needs no models to run; OCR/layout/unirec are opt-in. Models
//! are pulled from their original HuggingFace repos (everything is
//! Apache-2.0; we redistribute nothing) and land under loader-matchable
//! names so the enhancers read them directly.
//!
//! Downloads go through the HF **tree API** (`/api/models/{repo}/tree/main`)
//! plus `resolve/main` URLs — no `hf` CLI, no Python, no shell scripts:
//! `docparse fetch-models` covers every tier, and `ensure_ocr_models`
//! (first-use OCR) reuses the same machinery. File selection is by glob
//! against the live repo listing, so a spec survives repo reorganizations
//! the way the old shell globs did.

use anyhow::{Context, Result};
use std::path::Path;

/// A single file to pull from a HuggingFace repo. `glob` is matched against
/// every file path in the repo (tree API). `dest` is the bare file name the
/// loader expects (`find_file` patterns in this crate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSpec {
    pub repo: &'static str,
    pub glob: &'static str,
    pub dest: &'static str,
    /// Direct download URL that bypasses the HF tree lookup. Needed when the
    /// file has no home on HuggingFace any more — the v4 dictionary was
    /// deleted from SWHL/RapidOCR and lives in the upstream PaddleOCR repo
    /// (GitHub) instead. `Some` makes `repo`/`glob` informational only.
    pub url: Option<&'static str>,
}

/// The optional neural model tiers `fetch-models` knows how to install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// PP-OCRv4 fallback OCR (~16 MB).
    Ocr,
    /// PP-OCRv6 tiny — the default OCR tier (~7 MB).
    Ppv6,
    /// DocLayout-YOLO (~75 MB).
    Layout,
    /// UniRec-0.1B table/formula/transcribe (~700 MB).
    Unirec,
    /// PP-DocLayoutV2 (~210 MB; static-ize with onnxsim after download).
    Ppv2,
}

impl Tier {
    /// Subdirectory under the models root where this tier installs.
    pub fn dir(self) -> &'static str {
        match self {
            Tier::Ocr => "ppocr",
            Tier::Ppv6 => "ppocr-v6",
            Tier::Layout => "layout",
            Tier::Unirec => "unirec",
            Tier::Ppv2 => "layout-ppv2",
        }
    }

    /// One-line description for the fetch banner.
    pub fn summary(self) -> &'static str {
        match self {
            Tier::Ocr => "OCR v4 fallback (PP-OCRv4, SWHL/RapidOCR, Apache-2.0, ~16 MB)",
            Tier::Ppv6 => "OCR v6 default (PP-OCRv6 tiny, PaddlePaddle, Apache-2.0, ~7 MB)",
            Tier::Layout => "layout (DocLayout-YOLO, DocStructBench, Apache-2.0, ~75 MB)",
            Tier::Unirec => "UniRec-0.1B table/formula/transcribe (topdu, Apache-2.0, ~700 MB)",
            Tier::Ppv2 => "layout PP-DocLayoutV2 (topdu, Apache-2.0, ~210 MB + static-ize prep)",
        }
    }

    /// The files this tier installs (whole-repo tiers return `None`).
    pub fn files(self) -> Option<&'static [FileSpec]> {
        Some(match self {
            Tier::Ocr => &[
                FileSpec {
                    repo: "SWHL/RapidOCR",
                    glob: "**/ch_PP-OCRv4_det_infer.onnx",
                    dest: "ch_PP-OCRv4_det_infer.onnx",
                    url: None,
                },
                FileSpec {
                    repo: "SWHL/RapidOCR",
                    glob: "**/ch_PP-OCRv4_rec_infer.onnx",
                    dest: "ch_PP-OCRv4_rec_infer.onnx",
                    url: None,
                },
                FileSpec {
                    repo: "SWHL/RapidOCR",
                    glob: "**/ch_ppocr_mobile_v2.0_cls_infer.onnx",
                    dest: "ch_ppocr_mobile_v2.0_cls_infer.onnx",
                    url: None,
                },
                FileSpec {
                    repo: "SWHL/RapidOCR",
                    // Deleted from SWHL/RapidOCR; the upstream PaddleOCR
                    // repo (GitHub) is the file's real home.
                    glob: "**/ppocr_keys_v1.txt",
                    dest: "ppocr_keys_v1.txt",
                    url: Some("https://raw.githubusercontent.com/PaddlePaddle/PaddleOCR/main/ppocr/utils/ppocr_keys_v1.txt"),
                },
            ],
            Tier::Ppv6 => &[
                FileSpec {
                    repo: "PaddlePaddle/PP-OCRv6_tiny_det_onnx",
                    glob: "**/inference.onnx",
                    dest: "PP-OCRv6_tiny_det.onnx",
                    url: None,
                },
                FileSpec {
                    repo: "PaddlePaddle/PP-OCRv6_tiny_rec_onnx",
                    glob: "**/inference.onnx",
                    dest: "PP-OCRv6_tiny_rec.onnx",
                    url: None,
                },
                FileSpec {
                    repo: "PaddlePaddle/PP-OCRv6_tiny_rec_onnx",
                    glob: "**/inference.yml",
                    dest: "PP-OCRv6_tiny_rec.yml",
                    url: None,
                },
                // v6 ships no new orientation classifier — reuse v4's.
                FileSpec {
                    repo: "SWHL/RapidOCR",
                    glob: "**/ch_ppocr_mobile_v2.0_cls_infer.onnx",
                    dest: "ch_ppocr_mobile_v2.0_cls_infer.onnx",
                    url: None,
                },
            ],
            Tier::Layout => &[FileSpec {
                repo: "wybxc/DocLayout-YOLO-DocStructBench-onnx",
                glob: "**/*.onnx",
                dest: "doclayout_yolo.onnx",
                url: None,
            }],
            Tier::Ppv2 => &[FileSpec {
                repo: "topdu/PP_DoclayoutV2_onnx",
                glob: "**/PP-DoclayoutV2.onnx",
                dest: "PP-DoclayoutV2.onnx",
                url: None,
            }],
            // Whole-repo tier — every file, repo-relative paths preserved.
            Tier::Unirec => return None,
        })
    }

    fn repo(self) -> &'static str {
        match self {
            Tier::Unirec => "topdu/unirec_0_1b_onnx",
            _ => unreachable!("repo() is only meaningful for the whole-repo tier"),
        }
    }
}

/// True when `dir` already holds a usable OCR model set (det + rec present).
/// The dict (txt or rec yml) and cls are resolved by the loader; det+rec are
/// the minimum that makes a download unnecessary.
pub fn models_present(dir: &Path) -> bool {
    crate::find_file(dir, &["ch_PP-OCRv4_det_infer.onnx"], "det", ".onnx").is_ok()
        && crate::find_file(dir, &["ch_PP-OCRv4_rec_infer.onnx"], "rec", ".onnx").is_ok()
}

/// Whether `dir` is the built-in PP-OCRv6 default — the only dir we know
/// download URLs for. A custom `--ocr-models` path we can't fetch for.
pub fn is_default_v6_dir(dir: &Path) -> bool {
    dir.file_name().and_then(|n| n.to_str()) == Some("ppocr-v6")
}

/// Download the PP-OCRv6 tiny files into `dir` (~7 MB). Kept as the
/// first-use-OCR entry point; identical to `fetch_tier(Tier::Ppv6, dir, …)`.
pub fn fetch_ppocr_v6(dir: &Path, progress: impl FnMut(&str)) -> Result<()> {
    fetch_tier(Tier::Ppv6, dir, progress)
}

/// Install `tier` into `dir` (the tier's own directory; `models_present`
/// semantics apply per tier). Each file streams to a temp sibling, is
/// size-checked, then atomically renamed — an interrupted fetch never leaves
/// a half-written model the loader would choke on. Each file is retried a few
/// times: HF's CDN occasionally drops a connection mid-stream, which a retry
/// clears. `progress` is called once per file with a human-readable label
/// before its download starts.
pub fn fetch_tier(tier: Tier, dir: &Path, mut progress: impl FnMut(&str)) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let agent = agent();
    if let Tier::Unirec = tier {
        return fetch_repo(&agent, tier.repo(), dir, &mut progress);
    }
    let files = tier.files().expect("non-repo tier has file specs");
    for spec in files {
        progress(spec.dest);
        let url = match spec.url {
            Some(u) => u.to_string(),
            None => {
                let remote = find_in_repo(&agent, spec.repo, spec.glob)?;
                format!(
                    "https://huggingface.co/{}/resolve/main/{}",
                    spec.repo, remote
                )
            }
        };
        let dest = dir.join(spec.dest);
        let tmp = dest.with_file_name(format!(".{}.partial", spec.dest));
        download_one(&agent, &url, spec.dest, &tmp)
            .with_context(|| format!("download {}", spec.dest))?;
        std::fs::rename(&tmp, &dest).with_context(|| format!("install {}", dest.display()))?;
    }
    Ok(())
}

/// Install a whole repo (`Tier::Unirec`): every file, repo-relative path
/// preserved, so the loader's substring+ext lookup works on the original
/// names.
/// Whole-repo tiers download this many files concurrently: HF throttles per
/// connection (measured ~5-7 MB/min each), so 3 parallel streams ≈ 3× on the
/// ~700 MB UniRec tier without hammering the CDN.
const PARALLEL_DOWNLOADS: usize = 3;

fn fetch_repo(
    agent: &ureq::Agent,
    repo: &str,
    dir: &Path,
    progress: &mut impl FnMut(&str),
) -> Result<()> {
    let files = list_repo_files(agent, repo)?;
    // Announce every file up front — `progress` stays on the calling thread;
    // the actual downloads run `PARALLEL_DOWNLOADS`-wide below and only their
    // failures come back.
    for (path, _size) in &files {
        progress(path);
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failures: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        let workers = PARALLEL_DOWNLOADS.min(files.len()).max(1);
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((path, _size)) = files.get(i) else {
                    break;
                };
                let url = format!("https://huggingface.co/{repo}/resolve/main/{path}");
                let dest = dir.join(path);
                let tmp = dest.with_file_name(format!(".{}.partial", path.replace('/', "__")));
                if let Some(parent) = dest.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        failures
                            .lock()
                            .unwrap()
                            .push(format!("{path}: create {}: {e}", parent.display()));
                        continue;
                    }
                }
                match download_one(agent, &url, path, &tmp) {
                    Ok(()) => {
                        if let Err(e) = std::fs::rename(&tmp, &dest) {
                            failures
                                .lock()
                                .unwrap()
                                .push(format!("{path}: install {}: {e}", dest.display()));
                        }
                    }
                    Err(e) => failures.lock().unwrap().push(format!("{path}: {e:#}")),
                }
            });
        }
    });
    let failures = failures.into_inner().unwrap();
    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "{} file(s) failed:
  {}",
            failures.len(),
            failures.join(
                "
  "
            )
        )
    }
}

/// List every file path in `repo` (recursive) via the HF tree API, with sizes.
fn list_repo_files(agent: &ureq::Agent, repo: &str) -> Result<Vec<(String, u64)>> {
    let url = format!("https://huggingface.co/api/models/{repo}/tree/main?recursive=true");
    let resp = agent
        .get(&url)
        .call()
        .with_context(|| format!("list {repo} (tree API)"))?;
    let body = resp
        .into_string()
        .with_context(|| format!("read tree listing for {repo}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&body).with_context(|| format!("parse tree listing for {repo}"))?;
    let arr = v
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("unexpected tree API response for {repo}"))?;
    let mut out = Vec::new();
    for entry in arr {
        if entry.get("type").and_then(|t| t.as_str()) != Some("file") {
            continue;
        }
        let Some(path) = entry.get("path").and_then(|p| p.as_str()) else {
            continue;
        };
        let size = entry.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
        out.push((path.to_string(), size));
    }
    out.sort();
    Ok(out)
}

/// The file path in `repo` matching `glob` (first in sorted order — several
/// specs use `**/*.onnx`-style patterns where the repo holds extra files).
fn find_in_repo(agent: &ureq::Agent, repo: &str, glob: &str) -> Result<String> {
    let files = list_repo_files(agent, repo)?;
    files
        .into_iter()
        .map(|(p, _)| p)
        .find(|p| glob_match(glob, p))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no file matching {glob:?} in {repo} (repo may have moved; \
                 check huggingface.co/{repo})"
            )
        })
}

/// Minimal glob for the model specs: `**/` matches any number of leading path
/// components (including zero), `*` matches any run of chars within one
/// component; everything else is literal. This covers every pattern the specs
/// use (`**/inference.onnx`, `**/*.onnx`, `**/PP-DoclayoutV2.onnx`, …).
fn glob_match(pattern: &str, path: &str) -> bool {
    if pattern.is_empty() {
        return path.is_empty();
    }
    if let Some(rest) = pattern.strip_prefix("**/") {
        // **/ matches zero or more components: try the rest against the whole
        // path, then against the path with one component cut off, and so on.
        if glob_match(rest, path) {
            return true;
        }
        if let Some(i) = path.find('/') {
            return glob_match(pattern, &path[i + 1..]);
        }
        return false;
    }
    let mut pat = pattern;
    let mut s = path;
    while let Some(star) = pat.find('*') {
        if !s.starts_with(&pat[..star]) {
            return false;
        }
        s = &s[star..];
        pat = &pat[star..];
        while let Some(rest) = pat.strip_prefix('*') {
            pat = rest; // collapse consecutive stars
        }
        if pat.is_empty() {
            return true;
        }
        // `*` greedily skips to the next literal char.
        let c = pat.chars().next().expect("pat non-empty");
        let Some(i) = s.find(c) else {
            return false;
        };
        s = &s[i..];
    }
    s == pat
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(20))
        .user_agent(concat!("docparse-rs/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// Stream `url` to `tmp`, retrying a flaky CDN up to 3 times. The temp file is
/// recreated each attempt so a truncated body never carries over.
fn download_one(agent: &ureq::Agent, url: &str, name: &str, tmp: &Path) -> Result<()> {
    let mut last_err = None;
    for attempt in 1..=3 {
        let result = download_attempt(agent, url, name, tmp);
        match result {
            Ok(_) => return Ok(()),
            Err(e) => {
                // Keep the .partial on mid-stream failures: the next attempt
                // (or the next deploy.sh run — interrupted big models were the
                // old pain) resumes from it via a Range request.
                last_err = Some(e);
                if attempt < 3 {
                    std::thread::sleep(std::time::Duration::from_millis(500 * attempt));
                }
            }
        }
    }
    Err(last_err.unwrap()).context("3 attempts failed")
}

/// One GET that resumes where a previous `.partial` left off: an existing
/// partial triggers a `Range:` request; a `206` appends, anything else (200 =
/// server ignored the range, 4xx/5xx bubbles as the error) restarts cleanly.
fn download_attempt(agent: &ureq::Agent, url: &str, name: &str, tmp: &Path) -> Result<u64> {
    let already = std::fs::metadata(tmp).map(|m| m.len()).unwrap_or(0);
    let mut req = agent.get(url);
    let mut resume = false;
    if already > 0 {
        req = agent.get(url).set("Range", &format!("bytes={already}-"));
        resume = true;
    }
    let resp = req.call().with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    let (mut file, base) = if resume && status == 206 {
        (
            std::fs::OpenOptions::new()
                .append(true)
                .open(tmp)
                .with_context(|| format!("append {}", tmp.display()))?,
            already,
        )
    } else {
        (
            std::fs::File::create(tmp).with_context(|| format!("create {}", tmp.display()))?,
            0,
        )
    };
    let n = std::io::copy(&mut resp.into_reader(), &mut file)?;
    let total = base + n;
    // A "too small" file is only a failure when it's an un-dereferenced
    // git-LFS *pointer* (the source moved / LFS broke) — a tiny real file (a
    // 31-byte README.md, say) is legitimate content. Whole-repo tiers must
    // not die on their own metadata files.
    if total <= 1024 && looks_like_lfs_pointer(tmp) {
        anyhow::bail!("{name} is a git-LFS pointer ({total} bytes) — source moved?");
    }
    Ok(total)
}

/// True when the first bytes of `path` are the standard git-LFS pointer
/// prelude (`version https://git-lfs.github.com/spec/v1`). Reads only the
/// file head — never the (potentially huge) body.
fn looks_like_lfs_pointer(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 24];
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(n) = f.read(&mut head) else {
        return false;
    };
    head[..n].starts_with(b"version https://git-lfs")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lfs_pointer_detection() {
        let d = std::env::temp_dir().join(format!("docparse-lfs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let ptr = d.join("ptr");
        std::fs::write(
            &ptr,
            b"version https://git-lfs.github.com/spec/v1\noid sha256:...\n",
        )
        .unwrap();
        assert!(looks_like_lfs_pointer(&ptr));
        let tiny = d.join("tiny");
        std::fs::write(&tiny, b"# just a small readme\n").unwrap();
        assert!(
            !looks_like_lfs_pointer(&tiny),
            "a small real file is not a pointer"
        );
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn glob_matches_starstar_prefix() {
        assert!(glob_match("**/inference.onnx", "inference.onnx"));
        assert!(glob_match("**/inference.onnx", "onnx/inference.onnx"));
        assert!(glob_match("**/inference.onnx", "a/b/c/inference.onnx"));
        assert!(!glob_match("**/inference.onnx", "a/b/inference.yml"));
        assert!(!glob_match("**/inference.onnx", "inference.yml"));
    }

    #[test]
    fn glob_matches_bare_star() {
        assert!(glob_match("**/*.onnx", "model.onnx"));
        assert!(glob_match("**/*.onnx", "x/y/model.onnx"));
        assert!(!glob_match("**/*.onnx", "model.yml"));
        assert!(glob_match("**/PP-DoclayoutV2.onnx", "PP-DoclayoutV2.onnx"));
        assert!(!glob_match("**/PP-DoclayoutV2.onnx", "pp-doclayoutv2.onnx"));
    }

    #[test]
    fn tier_specs_are_sane() {
        // Every spec must target a well-formed repo and carry a dest name the
        // loader can find; no duplicate dests within a tier.
        for tier in [Tier::Ocr, Tier::Ppv6, Tier::Layout, Tier::Ppv2] {
            let files = tier.files().expect("non-repo tier");
            assert!(!files.is_empty(), "{tier:?} has no files");
            for spec in files {
                assert!(spec.repo.contains('/'), "bad repo in {tier:?}: {spec:?}");
                assert!(
                    !spec.dest.contains('/'),
                    "dest must be a bare name in {tier:?}: {spec:?}"
                );
                assert!(!spec.glob.is_empty(), "empty glob in {tier:?}: {spec:?}");
            }
            let mut dests: Vec<&str> = files.iter().map(|f| f.dest).collect();
            dests.sort_unstable();
            dests.dedup();
            assert_eq!(dests.len(), files.len(), "duplicate dests in {tier:?}");
        }
    }
}
