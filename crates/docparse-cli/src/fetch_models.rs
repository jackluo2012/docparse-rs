//! `docparse fetch-models` — install the optional neural model tiers without
//! the HuggingFace CLI or shell scripts.
//!
//! The actual downloads live in `docparse_ocr::fetch` (HF tree API + resolve
//! URLs, pure Rust); this module is the CLI face: tier selection, the models
//! root dir, banner/progress to stderr, and the PP-DocLayoutV2 static-ize
//! hint (that prep step still needs `onnxsim` — Python, one-time, documented).

use anyhow::Context as _;
use clap::ValueEnum;
use std::path::Path;

/// CLI-facing tier names (`fetch-models all` included).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FetchTierArg {
    /// PP-OCRv4 fallback OCR (~16 MB).
    Ocr,
    /// PP-OCRv6 tiny — default OCR tier (~7 MB).
    #[value(name = "ppocr-v6")]
    Ppv6,
    /// DocLayout-YOLO (~75 MB).
    Layout,
    /// UniRec-0.1B table/formula/transcribe (~700 MB).
    Unirec,
    /// PP-DocLayoutV2 (~210 MB + a one-time static-ize prep).
    Ppv2,
    /// Everything.
    All,
}

const ALL_TIERS: &[docparse_ocr::fetch::Tier] = &[
    docparse_ocr::fetch::Tier::Ppv6,
    docparse_ocr::fetch::Tier::Ocr,
    docparse_ocr::fetch::Tier::Layout,
    docparse_ocr::fetch::Tier::Unirec,
    docparse_ocr::fetch::Tier::Ppv2,
];

impl FetchTierArg {
    fn tiers(self) -> &'static [docparse_ocr::fetch::Tier] {
        match self {
            FetchTierArg::All => ALL_TIERS,
            FetchTierArg::Ocr => &[docparse_ocr::fetch::Tier::Ocr],
            FetchTierArg::Ppv6 => &[docparse_ocr::fetch::Tier::Ppv6],
            FetchTierArg::Layout => &[docparse_ocr::fetch::Tier::Layout],
            FetchTierArg::Unirec => &[docparse_ocr::fetch::Tier::Unirec],
            FetchTierArg::Ppv2 => &[docparse_ocr::fetch::Tier::Ppv2],
        }
    }
}

/// Run `fetch-models TIER [--dir ROOT]`. Exit nonzero on any failed download;
/// installed files are already on disk (atomic rename), so re-running resumes
/// by overwriting.
pub fn run(tier: FetchTierArg, root: &Path) -> anyhow::Result<()> {
    let tiers = tier.tiers();
    for t in tiers {
        let dir = root.join(t.dir());
        eprintln!("{} → {}", t.summary(), dir.display());
        docparse_ocr::fetch::fetch_tier(*t, &dir, |name| eprintln!("  ↓ {name}"))
            .with_context(|| format!("fetch {}", t.dir()))?;
        match t.files() {
            Some(files) => eprintln!("  ✓ {} file(s) → {}", files.len(), dir.display()),
            None => eprintln!("  ✓ all files → {}", dir.display()),
        }
    }
    if tiers.contains(&docparse_ocr::fetch::Tier::Ppv2) {
        eprintln!();
        eprintln!("  PP-DocLayoutV2's official export has a dynamic graph tract can't shape-infer.");
        eprintln!("  Static-ize it once (needs a venv with onnx + onnxsim):");
        eprintln!();
        eprintln!("      pip install onnx onnxsim");
        eprintln!("      python scripts/spike/ppv2/prepare.py");
        eprintln!();
        eprintln!("  → produces {}/layout-ppv2/PP-DoclayoutV2_simp.onnx, then run with",
            root.display());
        eprintln!("      --layout --layout-model {}/layout-ppv2/PP-DoclayoutV2_simp.onnx",
            root.display());
    }
    eprintln!("done.");
    Ok(())
}
