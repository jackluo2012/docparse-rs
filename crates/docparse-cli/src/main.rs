//! `docparse` — parse a document into JSON / Markdown / text.

mod batch;
mod cache;
mod fetch_models;
mod input_source;
mod mcp;
mod progress;
mod resources;
mod schema;
mod server;

use clap::{Parser, Subcommand, ValueEnum};
use docparse_adoc::AdocParser;
use docparse_core::output;
use docparse_core::parser::DocumentParser;
use docparse_csv::CsvParser;
use docparse_docx::DocxParser;
use docparse_eml::EmlParser;
use docparse_html::HtmlParser;
use docparse_img::ImageParser;
use docparse_md::MarkdownParser;
use docparse_pdf::PdfParser;
use docparse_pptx::PptxParser;
use docparse_srt::SrtParser;
use docparse_tex::TexParser;
use docparse_xlsx::XlsxParser;
use std::path::PathBuf;

/// Parser registry — one line per format backend. Shared by the CLI path, the
/// MCP server, and the REST server. `decode_images` makes the PDF backend
/// materialize every embedded image's pixels (the image-export path).
pub(crate) fn parsers_with(
    decode_images: bool,
    password: Option<String>,
) -> Vec<Box<dyn DocumentParser>> {
    vec![
        Box::new(PdfParser {
            decode_images,
            password,
        }),
        Box::new(DocxParser),
        Box::new(HtmlParser),
        Box::new(XlsxParser),
        Box::new(PptxParser),
        Box::new(MarkdownParser),
        Box::new(CsvParser),
        Box::new(SrtParser),
        Box::new(TexParser),
        Box::new(EmlParser),
        Box::new(ImageParser),
        Box::new(AdocParser),
    ]
}

/// Pick the backend by path and parse — the shared entry for all interfaces.
pub(crate) fn parse_path_with(
    path: &std::path::Path,
    decode_images: bool,
    password: Option<String>,
) -> anyhow::Result<docparse_core::ir::Document> {
    let parser = parsers_with(decode_images, password)
        .into_iter()
        .find(|p| p.supports(path))
        .ok_or_else(|| anyhow::anyhow!("no parser supports {}", path.display()))?;
    parser.parse(path)
}

/// Resolve the PDF password from its three possible sources, in explicit >
/// file > env priority. Clap already makes the flags mutually exclusive, so
/// at most one source is ever non-empty; the priority only matters when a
/// caller passes them programmatically. An explicitly named env var or file
/// that cannot be read is an error — never a silent "no password".
pub(crate) fn resolve_password(
    explicit: Option<String>,
    env_var: Option<&str>,
    file: Option<&std::path::Path>,
) -> anyhow::Result<Option<String>> {
    if let Some(pw) = explicit {
        return Ok(Some(pw));
    }
    if let Some(path) = file {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("--password-file {}: {e}", path.display()))?;
        let trimmed = raw.strip_suffix('\n').unwrap_or(&raw);
        let trimmed = trimmed.strip_suffix('\r').unwrap_or(trimmed);
        return Ok(Some(trimmed.to_string()));
    }
    if let Some(var) = env_var {
        return match std::env::var(var) {
            Ok(v) => Ok(Some(v)),
            Err(e) => Err(anyhow::anyhow!(
                "--password-env {var}: cannot read environment variable ({e})"
            )),
        };
    }
    Ok(None)
}

#[derive(Parser)]
#[command(
    name = "docparse",
    version,
    about = "Efficient multi-format document parser (Rust)"
)]
#[command(args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Input document(s) and/or folder(s). One file → result to stdout (or
    /// -o). Multiple inputs, a folder, or --out-dir → batch mode: each input is
    /// parsed and an aggregate report is printed at the end.
    inputs: Vec<PathBuf>,

    /// Output format.
    #[arg(short, long, value_enum, default_value_t = Format::Json)]
    format: Format,

    /// Only keep these pages, given 1-based and endpoint-inclusive:
    /// "3" (single), "1-5" (range), "10-" (open-ended to the last page),
    /// comma-separated combos like "1-5,10". Filtering runs after parsing and
    /// before any enhancement, so OCR/layout models only ever see the kept
    /// pages. Page numbers keep their absolute document numbering — chunks
    /// and outline still cite the original pages. An explicit page beyond the
    /// document is an error, never a silent truncate.
    #[arg(long, value_name = "SPEC")]
    pages: Option<String>,

    /// Explicit format for a stdin (`-`) or URL input whose format can't be
    /// inferred (no %PDF- magic, no URL suffix, no Content-Type). One of:
    /// pdf|docx|html|xlsx|pptx|md|csv|srt|tex|eml|img|adoc. Ignored for
    /// regular file inputs.
    #[arg(long, value_enum)]
    input_format: Option<input_source::InputFormat>,

    /// Table rendering inside `-f chunks` text (tab=default, markdown=pipe table).
    #[arg(long, value_enum, default_value_t = TableFormat::Tab)]
    table_format: TableFormat,

    /// Write to this file instead of stdout. For `-f okf` this is the bundle
    /// *directory* (omit it to auto-derive `<stem>-okf/`).
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// `-f okf`: prefix for each concept's `resource` URI (e.g.
    /// `file:///data/docs/`). Default empty → the bare source basename, which
    /// keeps bundles byte-identical across machines.
    #[arg(long, value_name = "URI")]
    okf_resource_base: Option<String>,

    /// `-f okf`: overwrite the target bundle directory even if it exists and is
    /// non-empty (otherwise an auto-derived non-empty dir is refused).
    #[arg(long)]
    force: bool,

    /// `-f okf`: write the bundle as a deterministic tar archive to stdout
    /// (for `| tar x` / upload) instead of a directory. Ignores -o.
    #[arg(long)]
    okf_tar: bool,

    /// Print a parse-quality report (coverage/garble/flags) as JSON to stderr.
    #[arg(long)]
    quality: bool,

    /// Gate pages for human review before corpus ingestion: prints a JSON
    /// review list (page, chars, garbled ratio, flags, reasons) to stderr.
    /// A page is listed when it has no text layer, or its garbled-character
    /// ratio exceeds THRESHOLD (higher = stricter). Optional — without this
    /// flag no review list is produced and output bytes are unchanged.
    #[arg(long, value_name = "FLOAT")]
    quality_threshold: Option<f32>,

    /// Password for encrypted PDFs (standard security handler: RC4 / AES-128 /
    /// AES-256). Omit for unencrypted files; an encrypted PDF loaded without a
    /// password fails with a clear message.
    #[arg(long, value_name = "PASSWORD", conflicts_with_all = ["password_env", "password_file"])]
    password: Option<String>,

    /// Read the PDF password from environment variable VAR instead of the
    /// command line — keeps the secret out of the process list / shell
    /// history / CI logs. Errors if VAR is unset (never silently "no
    /// password").
    #[arg(long, value_name = "VAR", conflicts_with_all = ["password", "password_file"])]
    password_env: Option<String>,

    /// Read the PDF password from FILE (trailing newline stripped) instead of
    /// the command line — for `.secret` files / Docker secrets. Errors if the
    /// file is unreadable.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["password", "password_env"])]
    password_file: Option<PathBuf>,

    /// RAG chunk target size: accumulate consecutive paragraphs up to about
    /// this many characters before emitting a chunk (default 800). Smaller =
    /// finer-grained chunks for dense vector indexes; larger = fewer, longer
    /// chunks. Individual blocks stay atomic (headings / lists / code / tables
    /// are never split). Matches the core default, so omitting it keeps output
    /// bytes unchanged.
    #[arg(long, value_name = "N", default_value_t = 800)]
    chunk_target_chars: usize,

    /// Print the per-page enhancement routing plan (which pages a model would
    /// be escalated to) as JSON to stderr — demonstrates how few pages are hard.
    #[arg(long)]
    route_plan: bool,

    /// OCR quality-flagged pages (scans) with the embedded ONNX enhancer
    /// (PP-OCRv6 via tract). Digital pages never touch the model. Requires
    /// model files — see --ocr-models.
    #[arg(long)]
    ocr: bool,

    /// PP-OCR model dir (*det*.onnx / *rec*.onnx / *dict*.txt; any generation).
    /// Default models/ppocr-v6 (PP-OCRv6 tiny); pass models/ppocr for v4.
    #[arg(long, default_value = "models/ppocr-v6")]
    ocr_models: PathBuf,

    /// Print the per-page complexity profile (kind/image-coverage/tables) as
    /// JSON to stderr — the routing signal, observable.
    #[arg(long)]
    profile: bool,

    /// Re-derive macro reading order with the layout model (renders each page
    /// on demand — pure Rust, opt-in; PDF only). Heavier: ~2.4s/page.
    #[arg(long)]
    layout: bool,

    /// Path to the layout ONNX model. Backend is auto-detected: DocLayout-YOLO
    /// (default) or PP-DocLayoutV2 (pass models/layout-ppv2/PP-DoclayoutV2_simp.onnx
    /// — richer 25-class semantics + native reading order; ~3x YOLO on
    /// messy-layout table detection).
    #[arg(long, default_value = "models/layout/doclayout_yolo.onnx")]
    layout_model: PathBuf,

    /// Caption sizable figures with a VLM (renders figure regions on demand;
    /// PDF only). Requires --vlm-url and --vlm-model. Captions are injected
    /// as positioned text with source "vlm:<model>".
    #[arg(long)]
    vlm_describe: bool,

    /// Re-extract detected tables' structure with a VLM (renders each table
    /// region on demand; PDF only). Handles merged cells / multi-row headers
    /// the geometric detectors can't. Requires --vlm-url and --vlm-model;
    /// replaced tables carry source "vlm:<model>", failures keep the
    /// deterministic grid.
    #[arg(long)]
    vlm_tables: bool,

    /// OpenAI-compatible service base URL (vLLM / LM Studio / cloud),
    /// e.g. http://127.0.0.1:8000
    #[arg(long)]
    vlm_url: Option<String>,

    /// Vision model name as the service knows it.
    #[arg(long)]
    vlm_model: Option<String>,

    /// Bearer token, if the service requires one.
    #[arg(long)]
    vlm_api_key: Option<String>,

    /// Re-extract detected tables' structure with the embedded UniRec-0.1B
    /// model (renders each table region on demand; PDF only). Resolves
    /// merged cells / multi-row headers in-process — no service needed.
    /// Value: model dir holding encoder/decoder ONNX + tokenizer mapping.
    /// Replaced tables carry source "table:unirec-0.1b"; failures keep the
    /// deterministic grid.
    #[arg(long, value_name = "DIR")]
    table_model: Option<PathBuf>,

    /// Convert display formulas to LaTeX with the embedded UniRec-0.1B
    /// model (PDF only). Formula regions come from the DocLayout-YOLO
    /// layout model (--layout-model path); glyph-soup text inside each
    /// region is replaced by one LaTeX chunk tagged "Formula" with source
    /// "formula:unirec-0.1b". Value: UniRec model directory.
    #[arg(long, value_name = "DIR")]
    formula_model: Option<PathBuf>,

    /// Re-recognize whole pages with the embedded UniRec model (PDF only):
    /// layout regions (DocLayout-YOLO) read in order, replacing the page's
    /// text at region-level positions. The route for design/CJK layouts the
    /// deterministic geometry can't order — opt-in, line-level positions are
    /// traded away (chunks carry region bboxes). Value: UniRec model dir.
    #[arg(long, value_name = "DIR")]
    transcribe_model: Option<PathBuf>,

    /// Embed image payloads as base64 in JSON output (data_base64 +
    /// data_media_type on each image element) — ODL's "embedded" mode.
    /// Decodes all embedded images ≥16px a side (PDF) or the input image.
    #[arg(long)]
    image_embed: bool,

    /// Export embedded raster images (≥16px a side) to this directory as
    /// JPEG/PNG files; JSON image elements gain a "file" path and Markdown
    /// references them (PDF only). Mirrors ODL's external image output.
    #[arg(long)]
    image_dir: Option<PathBuf>,

    /// Progress & speed visualization on stderr: auto (interactive TTY only,
    /// the default), always (force, even when piped), never (off), json
    /// (machine-readable JSON-lines events for CI/wrappers — no bar/ANSI).
    /// Shows a per-phase spinner / page bar and an end-of-run pages/s · MB/s
    /// summary. Never touches stdout, so `-f json > out.json` stays clean.
    #[arg(long, value_enum, default_value_t = progress::ProgressMode::Auto)]
    progress: progress::ProgressMode,

    /// Silence progress visualization (alias for --progress never).
    #[arg(long)]
    quiet: bool,

    /// Print CPU & peak-memory usage for the run to stderr at the end: peak RSS,
    /// CPU time (user+sys), and average utilization (>100% = multi-core work).
    /// Under --progress json it's emitted as a "resources" event instead.
    #[arg(long)]
    stats: bool,

    /// Batch output directory: write one result file per input as
    /// <out-dir>/<stem>.<format-ext> (json/md/txt). Required to keep parsed
    /// content when processing more than one file. Created if missing.
    #[arg(long, value_name = "DIR")]
    out_dir: Option<PathBuf>,

    /// Cache directory: on a re-run, skip documents whose content and output
    /// options match a previous run and replay the stored result — the heavy
    /// work (OCR / layout / UniRec) is skipped entirely. Keyed by content
    /// SHA-256 + output signature; a content or flag change simply misses and
    /// re-parses. Batch requires --out-dir and does not apply to -f okf; with
    /// serve/mcp it caches the enhanced document behind every tool/format.
    #[arg(long, value_name = "DIR")]
    cache_dir: Option<PathBuf>,

    /// In batch mode, descend into sub-folders. Default: only the folder's top
    /// level. No effect on explicit file inputs.
    #[arg(short, long)]
    recursive: bool,

    /// In batch mode, process up to N files in parallel (default 1 = serial).
    /// Only applies to deterministic batches: when any model flag (--ocr,
    /// --layout, --table-model, --formula-model, --transcribe-model, --vlm-*)
    /// is set, jobs is forced to 1 to keep peak memory bounded (per-page scan
    /// buffers + ~700MB models would multiply across files). Capped at the
    /// core count.
    #[arg(long, value_name = "N", default_value_t = 1)]
    jobs: usize,

    /// In batch mode, also write the aggregate report as JSON to this file.
    #[arg(long, value_name = "FILE")]
    report_json: Option<PathBuf>,

    /// In batch mode, also write the aggregate report as CSV (one row per file)
    /// to this file.
    #[arg(long, value_name = "FILE")]
    report_csv: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the parser over MCP (newline-delimited JSON-RPC on stdio) so
    /// agents can call parse/chunk/locate directly.
    Mcp {
        /// Model dir for the optional `ocr: true` tool argument.
        #[arg(long, default_value = "models/ppocr-v6")]
        ocr_models: PathBuf,
        /// Layout ONNX path for `layout`/`formula_model` tool arguments
        /// (DocLayout-YOLO or PP-DocLayoutV2, auto-detected).
        #[arg(long, default_value = "models/layout/doclayout_yolo.onnx")]
        layout_model: PathBuf,
        /// UniRec model dir enabling `table_model`/`formula_model` arguments.
        #[arg(long)]
        unirec_models: Option<PathBuf>,
        /// OpenAI-compatible service URL enabling `vlm_describe`/`vlm_tables`.
        #[arg(long)]
        vlm_url: Option<String>,
        /// Vision model name for the VLM service.
        #[arg(long)]
        vlm_model: Option<String>,
        /// Bearer token for the VLM service.
        #[arg(long)]
        vlm_api_key: Option<String>,
        /// Document cache dir: repeated parses of the same file (same content
        /// and enhancement flags) replay the stored document instead of
        /// re-parsing — pure acceleration, tool outputs stay byte-identical.
        #[arg(long, value_name = "DIR")]
        cache_dir: Option<PathBuf>,
        /// Default PDF password: explicit value used when a tool call does not
        /// pass `password` (alternative: --password-env / --password-file).
        #[arg(long, value_name = "PASSWORD", conflicts_with_all = ["password_env", "password_file"])]
        password: Option<String>,
        /// Read the default PDF password from environment variable VAR.
        #[arg(long, value_name = "VAR", conflicts_with_all = ["password", "password_file"])]
        password_env: Option<String>,
        /// Read the default PDF password from FILE (trailing newline stripped).
        #[arg(long, value_name = "PATH", conflicts_with_all = ["password", "password_env"])]
        password_file: Option<PathBuf>,
    },
    /// Serve a REST API: POST /parse (multipart) + GET /healthz.
    Serve {
        /// Bind address. Default 127.0.0.1 (same-machine trust model); set
        /// 0.0.0.0 only behind a trusted network boundary (e.g. a container on
        /// a private compose network) — the API has no auth.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// TCP port to listen on.
        #[arg(long, default_value_t = 8642)]
        port: u16,
        /// Model dir for the optional `?ocr=true` query parameter.
        #[arg(long, default_value = "models/ppocr-v6")]
        ocr_models: PathBuf,
        /// Layout ONNX path for `?layout=true` / `?formula_model=true`
        /// (DocLayout-YOLO or PP-DocLayoutV2, auto-detected).
        #[arg(long, default_value = "models/layout/doclayout_yolo.onnx")]
        layout_model: PathBuf,
        /// UniRec model dir enabling `?table_model=true` / `?formula_model=true`.
        #[arg(long)]
        unirec_models: Option<PathBuf>,
        /// OpenAI-compatible service URL enabling `?vlm_describe=true` / `?vlm_tables=true`.
        #[arg(long)]
        vlm_url: Option<String>,
        /// Vision model name for the VLM service.
        #[arg(long)]
        vlm_model: Option<String>,
        /// Bearer token for the VLM service.
        #[arg(long)]
        vlm_api_key: Option<String>,
        /// Document cache dir: repeated uploads of the same content (same
        /// enhancement query flags) replay the stored document instead of
        /// re-parsing — responses stay byte-identical; hits add the
        /// `x-docparse-cache: hit` response header.
        #[arg(long, value_name = "DIR")]
        cache_dir: Option<PathBuf>,
        /// Default PDF password: explicit value used when a request does not
        /// pass `?password=` (alternative: --password-env / --password-file).
        #[arg(long, value_name = "PASSWORD", conflicts_with_all = ["password_env", "password_file"])]
        password: Option<String>,
        /// Read the default PDF password from environment variable VAR.
        #[arg(long, value_name = "VAR", conflicts_with_all = ["password", "password_file"])]
        password_env: Option<String>,
        /// Read the default PDF password from FILE (trailing newline stripped).
        #[arg(long, value_name = "PATH", conflicts_with_all = ["password", "password_env"])]
        password_file: Option<PathBuf>,
    },
    /// Install the optional neural model tiers (OCR / layout / UniRec) in pure
    /// Rust — no HuggingFace CLI, no Python, no shell scripts. Downloads from
    /// the original HuggingFace repos via the tree API (all Apache-2.0).
    FetchModels {
        /// Which tier to fetch: ocr, ppocr-v6, layout, unirec, ppv2, all.
        #[arg(value_enum)]
        tier: fetch_models::FetchTierArg,
        /// Models root directory; tier subdirs (ppocr/, ppocr-v6/, layout/,
        /// unirec/, layout-ppv2/) are created under it.
        #[arg(long, value_name = "DIR", default_value = "models")]
        dir: PathBuf,
    },
    /// Reverse citation lookup (the CLI face of the MCP `locate` tool):
    /// print the retrieval chunk covering the given point on a page.
    Locate {
        /// The document to search.
        file: PathBuf,
        /// 1-based page number.
        #[arg(long)]
        page: usize,
        /// X coordinate in PDF user space (pt, origin left).
        #[arg(long)]
        x: f32,
        /// Y coordinate: in PDF user space (origin bottom, y up) by default,
        /// or distance from the top edge with --top-left.
        #[arg(long)]
        y: f32,
        /// Take `--y` from the top edge (screenshot/annotation convention)
        /// and convert: y_user = page_height - y.
        #[arg(long)]
        top_left: bool,
        /// Output: the covering chunk's JSON (default) or just its text.
        #[arg(short, long, value_enum, default_value_t = LocateFormat::Json)]
        format: LocateFormat,
        /// PDF decryption password (encrypted PDFs only).
        #[arg(long, value_name = "PASSWORD")]
        password: Option<String>,
    },
    /// Print (or write) the machine-readable output contract: JSON Schema
    /// (draft 2020-12) for every output format, generated from the code.
    Schema {
        /// Print only this schema (one of: document, chunk, outline, quality,
        /// profile, okf-bundle). Default: a JSON object of all of them.
        #[arg(long)]
        name: Option<String>,
        /// Write the schemas to `schemas/<name>.json` (the committed contract
        /// files) instead of printing. Regenerates after a contract change.
        #[arg(long)]
        write: bool,
    },
}

/// Lazily-loaded OCR enhancer shared by the serving faces: models are read on
/// the first request that asks for OCR, never at startup, so serving digital
/// documents stays model-free. The load outcome (ok or a stable error string)
/// is cached — broken setups fail fast on every call instead of re-reading.
pub(crate) struct OcrState {
    dir: PathBuf,
    cell: std::sync::OnceLock<Result<docparse_ocr::PpOcrEnhancer, String>>,
}

impl OcrState {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            cell: std::sync::OnceLock::new(),
        }
    }

    pub(crate) fn get(&self) -> Result<&docparse_ocr::PpOcrEnhancer, String> {
        self.cell
            .get_or_init(|| {
                ensure_ocr_models(&self.dir).map_err(|e| format!("{e:#}"))?;
                docparse_ocr::PpOcrEnhancer::new(&self.dir).map_err(|e| format!("{e:#}"))
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

/// A UniRec model loaded on first use and cached for the rest of the run — the
/// CLI-path analogue of `EnhanceState`'s server-lifetime UniRec. The model is
/// ~700 MB to load, so a batch over many files (`--table-model` / `--formula-model`
/// / `--transcribe-model`) must read it once, not once per file.
pub(crate) struct LazyUniRec {
    cell: std::sync::OnceLock<Result<docparse_ocr::unirec::UniRec, String>>,
}

impl LazyUniRec {
    fn new() -> Self {
        Self {
            cell: std::sync::OnceLock::new(),
        }
    }

    /// The model dir is fixed for a run (one CLI flag), so `dir` is the same on
    /// every call; the first load wins and is reused.
    fn get(&self, dir: &std::path::Path) -> anyhow::Result<&docparse_ocr::unirec::UniRec> {
        self.cell
            .get_or_init(|| docparse_ocr::unirec::UniRec::new(dir).map_err(|e| format!("{e:#}")))
            .as_ref()
            .map_err(|e| anyhow::anyhow!("unirec models unavailable: {e}"))
    }
}

/// A layout (DocLayout-YOLO / PP-DocLayoutV2) model loaded on first use and
/// cached — `--layout`/`--formula-model`/`--transcribe-model` all need it, so a
/// batch (or a single file using several of them) reads it once.
pub(crate) struct LazyLayout {
    cell: std::sync::OnceLock<Result<docparse_ocr::layout::LayoutModel, String>>,
}

impl LazyLayout {
    fn new() -> Self {
        Self {
            cell: std::sync::OnceLock::new(),
        }
    }

    fn get(&self, path: &std::path::Path) -> anyhow::Result<&docparse_ocr::layout::LayoutModel> {
        self.cell
            .get_or_init(|| {
                docparse_ocr::layout::LayoutModel::new(path).map_err(|e| format!("{e:#}"))
            })
            .as_ref()
            .map_err(|e| anyhow::anyhow!("layout model unavailable: {e}"))
    }
}

/// Models loaded once per CLI run and reused across every input. In single-file
/// mode this is just the one file; in batch mode it's the whole folder — so the
/// heavy OCR / UniRec / layout models are read at most once, not per file. All
/// fields are lazy (interior-mutable `OnceLock`): a digital-only `--ocr` batch
/// still never touches a model, preserving the "digital stays model-free"
/// invariant.
pub(crate) struct RunModels {
    ocr: OcrState,
    layout: LazyLayout,
    table: LazyUniRec,
    formula: LazyUniRec,
    transcribe: LazyUniRec,
}

impl RunModels {
    fn from_cli(cli: &Cli) -> Self {
        Self {
            ocr: OcrState::new(cli.ocr_models.clone()),
            layout: LazyLayout::new(),
            table: LazyUniRec::new(),
            formula: LazyUniRec::new(),
            transcribe: LazyUniRec::new(),
        }
    }
}

/// Make sure the OCR model dir is populated before the enhancer reads it.
///
/// For the built-in PP-OCRv6 default we can fetch the ~7 MB model set on first
/// use. Downloading is a network action, so it's gated on an interactive y/N
/// confirm; non-interactive faces (MCP/REST servers, pipes, CI) aren't a TTY
/// and get a clear error with the fetch command instead. `DOCPARSE_OCR_DOWNLOAD=1`
/// pre-confirms for automation that explicitly opts in.
fn ensure_ocr_models(dir: &std::path::Path) -> anyhow::Result<()> {
    use anyhow::Context as _;
    use std::io::{IsTerminal, Write};
    if docparse_ocr::fetch::models_present(dir) {
        return Ok(());
    }
    let fetch_cmd = "docparse fetch-models ppocr-v6";
    if !docparse_ocr::fetch::is_default_v6_dir(dir) {
        anyhow::bail!(
            "OCR models not found in {}\n  download them with: {fetch_cmd}",
            dir.display()
        );
    }
    let preconfirmed = std::env::var_os("DOCPARSE_OCR_DOWNLOAD").is_some();
    if !preconfirmed {
        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "OCR models not found at {}\n  run: {fetch_cmd}\n  \
                 or set DOCPARSE_OCR_DOWNLOAD=1 to fetch non-interactively (~7 MB, Apache-2.0)",
                dir.display()
            );
        }
        eprint!(
            "OCR models missing. Download PP-OCRv6 tiny (~7 MB, PaddlePaddle, Apache-2.0) \
             to {}? [y/N] ",
            dir.display()
        );
        std::io::stderr().flush().ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            anyhow::bail!("declined — fetch later with: {fetch_cmd}");
        }
    }
    docparse_ocr::fetch::fetch_ppocr_v6(dir, |name| eprintln!("  ↓ {name}"))
        .context("downloading PP-OCRv6 models")?;
    eprintln!("  ✓ OCR models ready at {}", dir.display());
    Ok(())
}

/// Run quality-routed enhancement over a parsed document (shared by all faces).
/// Remove empty-row table placeholders (a `--layout`-seeded table region no
/// model filled, or `--table-model` not run). Called after ALL enhancers so
/// every output face is consistent — including `-f json`, which serializes
/// `page.elements` directly and otherwise leaks `{"type":"table","rows":[]}`.
fn drop_empty_table_placeholders(doc: &mut docparse_core::ir::Document) {
    for page in &mut doc.pages {
        page.elements
            .retain(|e| !matches!(e, docparse_core::ir::Element::Table(t) if t.rows.is_empty()));
    }
}

pub(crate) fn apply_ocr(
    doc: docparse_core::ir::Document,
    path: &std::path::Path,
    ocr: &docparse_ocr::PpOcrEnhancer,
) -> anyhow::Result<docparse_core::ir::Document> {
    Ok(apply_ocr_with(doc, path, ocr, None)?.0)
}

fn apply_ocr_with(
    doc: docparse_core::ir::Document,
    path: &std::path::Path,
    ocr: &docparse_ocr::PpOcrEnhancer,
    on_page: Option<&(dyn Fn() + Sync)>,
) -> anyhow::Result<(
    docparse_core::ir::Document,
    Vec<docparse_core::enhance::PageRoute>,
)> {
    let is_pdf = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
    if is_pdf {
        return Ok(docparse_ocr::pdf_ocr::apply_with(
            &doc,
            std::fs::read(path)?,
            ocr,
            on_page,
        ));
    }
    Ok(docparse_core::enhance::apply_with(
        &doc,
        &[ocr as &dyn docparse_core::enhance::Enhancer],
        on_page,
    ))
}

fn vlm_config(
    url: Option<String>,
    model: Option<String>,
    api_key: Option<String>,
) -> Option<docparse_vlm::VlmConfig> {
    match (url, model) {
        (Some(url), Some(model)) => Some(docparse_vlm::VlmConfig {
            url,
            model,
            api_key,
        }),
        _ => None,
    }
}

/// Per-request enhancement switches for the serving faces (MCP tool args /
/// REST query params). Everything defaults off — the deterministic result.
#[derive(Default, Clone, Copy)]
pub(crate) struct EnhanceOpts {
    pub ocr: bool,
    /// Embed image payloads as base64 in the JSON output (serving counterpart
    /// of --image-dir; ODL's image_output="embedded").
    pub images_embedded: bool,
    pub layout: bool,
    pub table_model: bool,
    pub formula_model: bool,
    pub vlm_describe: bool,
    pub vlm_tables: bool,
}

impl EnhanceOpts {
    fn any_pdf_only(&self) -> bool {
        self.layout
            || self.table_model
            || self.formula_model
            || self.vlm_describe
            || self.vlm_tables
    }
}

/// Server-lifetime enhancement state: capability config from startup flags +
/// lazily-loaded models shared across requests (UniRec is ~700MB — loading
/// once per server is the point). A capability whose config is absent yields
/// a clear per-request error naming the startup flag, never a crash.
pub(crate) struct EnhanceState {
    pub ocr: OcrState,
    layout_model: PathBuf,
    unirec_dir: Option<PathBuf>,
    vlm: Option<docparse_vlm::VlmConfig>,
    /// Optional document cache dir (`serve`/`mcp --cache-dir`): when set,
    /// repeated parses of the same content + flags replay the stored document
    /// instead of re-running parsing / OCR / layout.
    pub(crate) cache_dir: Option<PathBuf>,
    /// Default encrypted-PDF password configured at startup
    /// (`serve`/`mcp --password/--password-env/--password-file`). Used when a
    /// request / tool call does not pass one explicitly; the *resolved* value
    /// still flows through the document-cache signature, so per-password
    /// cache isolation holds for defaults too.
    pub(crate) default_password: Option<String>,
    unirec: std::sync::OnceLock<Result<std::sync::Arc<docparse_ocr::unirec::UniRec>, String>>,
    layout: std::sync::OnceLock<Result<std::sync::Arc<docparse_ocr::layout::LayoutModel>, String>>,
}

impl EnhanceState {
    pub(crate) fn new(
        ocr_models: PathBuf,
        layout_model: PathBuf,
        unirec_dir: Option<PathBuf>,
        vlm: Option<docparse_vlm::VlmConfig>,
    ) -> Self {
        Self {
            ocr: OcrState::new(ocr_models),
            layout_model,
            unirec_dir,
            vlm,
            cache_dir: None,
            default_password: None,
            unirec: std::sync::OnceLock::new(),
            layout: std::sync::OnceLock::new(),
        }
    }

    /// Attach the `--cache-dir` the serving face was started with (`None` = no
    /// caching). Consumes `self` so callers keep a single expression.
    pub(crate) fn with_cache_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.cache_dir = dir;
        self
    }

    /// Attach the startup default PDF password (see `default_password`).
    /// Consumes `self` so callers keep a single expression.
    pub(crate) fn with_default_password(mut self, password: Option<String>) -> Self {
        self.default_password = password;
        self
    }

    /// Output-affecting signature for the document cache. Every `EnhanceOpts`
    /// field changes what the enhanced document contains, and the configured
    /// model paths are included too — a `--cache-dir` can outlive the process,
    /// and a different model set must never replay another server's entries.
    /// Render-level options (format / envelope / table_markdown / tool
    /// arguments) are deliberately excluded: they change the rendering, not
    /// the document, and the faces render after the lookup. The `srv1;` prefix
    /// keeps serving entries distinct from batch entries sharing an identity.
    /// **Any new output-affecting serving option MUST be added here**, or a
    /// changed option would replay stale bytes.
    pub(crate) fn cache_signature(&self, opts: &EnhanceOpts, password: Option<&str>) -> String {
        use std::fmt::Write as _;
        let mut s = String::new();
        let _ = write!(s, "srv1;");
        let _ = write!(s, "ocr={};ocr_models={}", opts.ocr, self.ocr.dir.display());
        let _ = write!(s, ";images={}", opts.images_embedded);
        let _ = write!(
            s,
            ";layout={};layout_model={}",
            opts.layout,
            self.layout_model.display()
        );
        let _ = write!(
            s,
            ";table_model={};formula_model={}",
            opts.table_model, opts.formula_model
        );
        if let Some(d) = &self.unirec_dir {
            let _ = write!(s, ";unirec={}", d.display());
        }
        let _ = write!(
            s,
            ";vlm_describe={};vlm_tables={}",
            opts.vlm_describe, opts.vlm_tables
        );
        if let Some(v) = &self.vlm {
            let _ = write!(s, ";vlm_url={:?};vlm_model={:?}", v.url, v.model);
        }
        // The password changes what the document contains — a different
        // password (or none) must never replay another password's parse.
        let _ = write!(s, ";password={:?}", password);
        s
    }

    fn unirec(&self) -> anyhow::Result<std::sync::Arc<docparse_ocr::unirec::UniRec>> {
        let dir = self.unirec_dir.as_ref().ok_or_else(|| {
            anyhow::anyhow!("table/formula model not configured (start with --unirec-models <dir>)")
        })?;
        self.unirec
            .get_or_init(|| {
                docparse_ocr::unirec::UniRec::new(dir)
                    .map(std::sync::Arc::new)
                    .map_err(|e| format!("{e:#}"))
            })
            .clone()
            .map_err(|e| anyhow::anyhow!("unirec models unavailable: {e}"))
    }

    /// Layout model loaded once per server lifetime (lazy), shared across
    /// requests — the serving counterpart of the CLI's `RunModels.layout`.
    fn loaded_layout(&self) -> anyhow::Result<std::sync::Arc<docparse_ocr::layout::LayoutModel>> {
        self.layout
            .get_or_init(|| {
                docparse_ocr::layout::LayoutModel::new(&self.layout_model)
                    .map(std::sync::Arc::new)
                    .map_err(|e| format!("{e:#}"))
            })
            .clone()
            .map_err(|e| anyhow::anyhow!("layout model unavailable: {e}"))
    }

    /// Apply the requested enhancements in the CLI's order. PDF-only
    /// enhancements are skipped for other formats (documented in the tool
    /// descriptions); unconfigured capabilities error with the startup flag
    /// to set.
    pub(crate) fn apply(
        &self,
        mut doc: docparse_core::ir::Document,
        path: &std::path::Path,
        o: EnhanceOpts,
    ) -> anyhow::Result<docparse_core::ir::Document> {
        if o.ocr {
            let enhancer = self
                .ocr
                .get()
                .map_err(|e| anyhow::anyhow!("ocr models unavailable: {e}"))?;
            doc = apply_ocr(doc, path, enhancer)?;
        }
        if o.images_embedded {
            embed_images(&mut doc);
        }
        let is_pdf = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false);
        if !o.any_pdf_only() || !is_pdf {
            return Ok(doc);
        }
        if o.layout {
            let bytes = std::fs::read(path)?;
            let layout = self.loaded_layout()?;
            docparse_ocr::layout::enhance_document(&mut doc, bytes, &layout, 2.0)?;
        }
        if o.table_model {
            let model = self.unirec()?;
            docparse_ocr::table_model::refine_tables(&mut doc, std::fs::read(path)?, &model)?;
        }
        if o.formula_model {
            let layout = self.loaded_layout()?;
            let model = self.unirec()?;
            docparse_ocr::formula::enhance_formulas(
                &mut doc,
                std::fs::read(path)?,
                &layout,
                &model,
            )?;
        }
        if o.vlm_describe || o.vlm_tables {
            let cfg = self.vlm.clone().ok_or_else(|| {
                anyhow::anyhow!("vlm not configured (start with --vlm-url and --vlm-model)")
            })?;
            let client = docparse_vlm::VlmClient::new(cfg);
            if o.vlm_describe {
                docparse_vlm::annotate_pictures(&mut doc, std::fs::read(path)?, &client)?;
            }
            if o.vlm_tables {
                docparse_vlm::refine_tables(&mut doc, std::fs::read(path)?, &client)?;
            }
        }
        drop_empty_table_placeholders(&mut doc);
        Ok(doc)
    }
}

/// Parse + enhance one document for the serving faces, replaying the cached
/// document when `--cache-dir` is set and the (identity, content, flags, model
/// set) triple matches.
///
/// Returns `(document, cache_status)` where `cache_status` is `None` when
/// caching is disabled, `Some(true)` on a hit, `Some(false)` on a miss that
/// stored. `source_name` replaces the document's source annotation (REST: the
/// sanitized upload name — it appears in the output, so it is part of the key
/// identity); `None` keeps the parser's default (MCP: the path). `password`
/// decrypts encrypted PDFs (`None` for everything else) — it is part of the
/// cache signature, so a different password never replays another password's
/// parse.
///
/// The cached unit is the enhanced document — the shared heavy step behind
/// every format and tool. Rendering happens after the lookup, so format/tool
/// arguments never fragment the cache. Store failures are best-effort: they
/// degrade to a re-parse, never an error.
pub(crate) fn parse_enhanced_cached(
    path: &std::path::Path,
    source_name: Option<&str>,
    opts: EnhanceOpts,
    password: Option<String>,
    state: &EnhanceState,
) -> anyhow::Result<(docparse_core::ir::Document, Option<bool>)> {
    // `EnhanceOpts` is Copy: the same opts can be handed to every branch.
    let parse_fresh = |opts: EnhanceOpts,
                       password: Option<&str>|
     -> anyhow::Result<docparse_core::ir::Document> {
        let mut doc = parse_path_with(path, opts.images_embedded, password.map(str::to_string))?;
        doc = state.apply(doc, path, opts)?;
        if let Some(name) = source_name {
            doc.source = name.to_string();
        }
        Ok(doc)
    };
    let Some(cache_dir) = &state.cache_dir else {
        return Ok((parse_fresh(opts, password.as_deref())?, None));
    };
    let sha = crate::cache::content_sha(path)?;
    let sig = state.cache_signature(&opts, password.as_deref());
    let identity = source_name
        .map(str::to_string)
        .unwrap_or_else(|| path.display().to_string());
    if let Some(entry) = crate::cache::lookup_identity(cache_dir, &identity, &sha, &sig) {
        // Corruption / version skew is a miss, not an error: re-parse and
        // overwrite, same as the batch face.
        if let Ok(doc) = serde_json::from_str::<docparse_core::ir::Document>(&entry.output) {
            return Ok((doc, Some(true)));
        }
    }
    let doc = parse_fresh(opts, password.as_deref())?;
    let json = serde_json::to_string(&doc)?;
    let entry = crate::cache::CacheEntry::new(sha.clone(), doc.pages.len(), json);
    let _ = crate::cache::store_identity(cache_dir, &identity, &sha, &sig, &entry);
    Ok((doc, Some(false)))
}

/// Write each decoded image to `dir` (JPEG passthrough as-is; raw Gray8/Rgb8
/// bitmaps as PNG) and record the path on the element so JSON/Markdown can
/// reference it. Returns the number of files written. Position-only images
/// (unsupported encodings, below the size gate) are skipped — they keep their
/// bbox in JSON for audit, same as before.
/// File extension for an already-encoded image's MIME type (default `bin`).
fn mime_ext(mime: Option<&str>) -> &'static str {
    match mime {
        Some("image/png") => "png",
        Some("image/jpeg") => "jpg",
        Some("image/gif") => "gif",
        Some("image/bmp") => "bmp",
        Some("image/tiff") => "tiff",
        Some("image/webp") => "webp",
        Some("image/x-emf") | Some("image/emf") => "emf",
        Some("image/x-wmf") | Some("image/wmf") => "wmf",
        _ => "bin",
    }
}

fn export_images(
    doc: &mut docparse_core::ir::Document,
    dir: &std::path::Path,
) -> anyhow::Result<usize> {
    use docparse_core::ir::{Element, ImageKind};
    std::fs::create_dir_all(dir)?;
    let mut written = 0usize;
    for page in &mut doc.pages {
        let mut idx = 0usize;
        for el in &mut page.elements {
            let Element::Image(img) = el else { continue };
            if img.data.is_empty() {
                continue;
            }
            let (ext, bytes) = match img.kind {
                ImageKind::Jpeg => ("jpg", std::mem::take(&mut img.data)),
                // Already-encoded media (DOCX/PPTX/HTML): write bytes verbatim,
                // deriving the extension from the source MIME type.
                ImageKind::Encoded => (
                    mime_ext(img.data_media_type.as_deref()),
                    std::mem::take(&mut img.data),
                ),
                ImageKind::Rgb8 => (
                    "png",
                    docparse_vlm::encode_png_rgb(&img.data, img.width_px, img.height_px),
                ),
                ImageKind::Gray8 => {
                    let rgb: Vec<u8> = img.data.iter().flat_map(|&g| [g, g, g]).collect();
                    (
                        "png",
                        docparse_vlm::encode_png_rgb(&rgb, img.width_px, img.height_px),
                    )
                }
                ImageKind::None => continue,
            };
            idx += 1;
            let name = format!("p{}-{}.{}", page.number, idx, ext);
            let path = dir.join(&name);
            std::fs::write(&path, bytes)?;
            img.file = Some(path.display().to_string());
            written += 1;
        }
    }
    Ok(written)
}

/// Fill `data_base64`/`data_media_type` on every image that carries pixels
/// (JPEG passthrough as-is; raw bitmaps re-encoded as PNG) — the embedded
/// counterpart of `--image-dir` (ODL `image_output="embedded"`). Returns the
/// number of images embedded.
pub(crate) fn embed_images(doc: &mut docparse_core::ir::Document) -> usize {
    use base64::Engine;
    use docparse_core::ir::{Element, ImageKind};
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut n = 0usize;
    for page in &mut doc.pages {
        for el in &mut page.elements {
            let Element::Image(img) = el else { continue };
            if img.data.is_empty() {
                continue;
            }
            let (mime, bytes) = match img.kind {
                ImageKind::Jpeg => ("image/jpeg".to_string(), img.data.clone()),
                // Already-encoded media: embed bytes as-is under their source MIME.
                ImageKind::Encoded => (
                    img.data_media_type
                        .clone()
                        .unwrap_or_else(|| "application/octet-stream".to_string()),
                    img.data.clone(),
                ),
                ImageKind::Rgb8 => (
                    "image/png".to_string(),
                    docparse_vlm::encode_png_rgb(&img.data, img.width_px, img.height_px),
                ),
                ImageKind::Gray8 => {
                    let rgb: Vec<u8> = img.data.iter().flat_map(|&g| [g, g, g]).collect();
                    (
                        "image/png".to_string(),
                        docparse_vlm::encode_png_rgb(&rgb, img.width_px, img.height_px),
                    )
                }
                ImageKind::None => continue,
            };
            img.data_base64 = Some(b64.encode(bytes));
            img.data_media_type = Some(mime);
            n += 1;
        }
    }
    n
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Format {
    Json,
    Markdown,
    Text,
    /// Retrieval chunks with source page+bbox and heading breadcrumb (JSON).
    Chunks,
    /// Document structure tree: nested sections (title/level/page/bbox) for
    /// agentic navigation — list the table of contents, drill into a section (JSON).
    Outline,
    /// Document metadata report: source, parser, page count, and the
    /// container's metadata (PDF Info / OOXML core.xml / HTML <meta>) as JSON.
    Meta,
    /// Open Knowledge Format bundle: a directory of Markdown + YAML-frontmatter
    /// "concept" files mirroring the structure tree (git-native, citable RAG
    /// delivery). Writes a directory (`-o <dir>`, else auto-derived `<stem>-okf/`).
    Okf,
}

/// Output style for `docparse locate`: the covering chunk's JSON (same shape
/// as a `-f chunks` element) or just its text.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum LocateFormat {
    Json,
    Text,
}

/// Table cell rendering inside `chunks` text.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum TableFormat {
    /// Tab/newline separated (default, compact).
    Tab,
    /// GitHub pipe table (markdown-native consumers).
    Markdown,
}

fn main() -> anyhow::Result<()> {
    let mut cli = Cli::parse();

    // Resolve the PDF password from the mutually-exclusive sources
    // (--password / --password-env / --password-file) exactly once; the
    // resolved value feeds the single-file path and the batch runner.
    // Serve/Mcp re-resolve inside their arms to build the server default.
    let password = resolve_password(
        cli.password.clone(),
        cli.password_env.as_deref(),
        cli.password_file.as_deref(),
    )?;

    // Borrow (not move) the subcommand so the whole `cli` stays available to the
    // file-processing path below — server fields are cheap to clone.
    if let Some(cmd) = &cli.command {
        match cmd {
            Command::Mcp {
                ocr_models,
                layout_model,
                unirec_models,
                vlm_url,
                vlm_model,
                vlm_api_key,
                cache_dir,
                password,
                password_env,
                password_file,
            } => {
                let default_password = resolve_password(
                    password.clone(),
                    password_env.as_deref(),
                    password_file.as_deref(),
                )?;
                return mcp::serve(
                    EnhanceState::new(
                        ocr_models.clone(),
                        layout_model.clone(),
                        unirec_models.clone(),
                        vlm_config(vlm_url.clone(), vlm_model.clone(), vlm_api_key.clone()),
                    )
                    .with_cache_dir(cache_dir.clone())
                    .with_default_password(default_password),
                );
            }
            Command::Serve {
                host,
                port,
                ocr_models,
                layout_model,
                unirec_models,
                vlm_url,
                vlm_model,
                vlm_api_key,
                cache_dir,
                password,
                password_env,
                password_file,
            } => {
                let default_password = resolve_password(
                    password.clone(),
                    password_env.as_deref(),
                    password_file.as_deref(),
                )?;
                return server::serve(
                    host,
                    *port,
                    EnhanceState::new(
                        ocr_models.clone(),
                        layout_model.clone(),
                        unirec_models.clone(),
                        vlm_config(vlm_url.clone(), vlm_model.clone(), vlm_api_key.clone()),
                    )
                    .with_cache_dir(cache_dir.clone())
                    .with_default_password(default_password),
                );
            }
            Command::Locate {
                file,
                page,
                x,
                y,
                top_left,
                format,
                password,
            } => return run_locate(file, *page, *x, *y, *top_left, *format, password.clone()),
            Command::FetchModels { tier, dir } => return fetch_models::run(*tier, dir),
            Command::Schema { name, write } => return schema::run(name.as_deref(), *write),
        }
    }
    if cli.inputs.is_empty() {
        anyhow::bail!("missing input file or folder (see --help)");
    }

    // Speed visualization (stderr-only, TTY-gated). The clock starts now so the
    // end-of-run summary (and --stats wall time) covers parse + every phase.
    let run_start = std::time::Instant::now();
    let reporter = progress::Reporter::new(cli.progress, cli.quiet);

    // Materialize non-file inputs (stdin "-", URLs) into temp files so the
    // extension-driven backend dispatch sees a normal path. Both shapes are
    // deliberately single-input: batching a folder mixes local files with a
    // stream/download, and --out-dir naming needs a real stem.
    let is_special = |p: &PathBuf| {
        p.to_str() == Some("-")
            || p.to_str()
                .map(|s| s.starts_with("http://") || s.starts_with("https://"))
                .unwrap_or(false)
    };
    let mut temp_input: Option<input_source::TempInput> = None;
    if cli.inputs.iter().any(is_special) {
        if cli.inputs.len() != 1 {
            anyhow::bail!("- (stdin) and URL inputs must be the only input — folder/multi-input batching doesn't apply to them");
        }
        if cli.out_dir.is_some() {
            anyhow::bail!("--out-dir does not apply to - (stdin) or URL inputs");
        }
        let spec = cli.input_format;
        if cli.inputs[0].to_str() == Some("-") {
            let _g = reporter.spinner("stdin");
            temp_input = Some(input_source::from_stdin(spec)?);
        } else {
            let url = cli.inputs[0].to_str().unwrap_or_default().to_string();
            let _g = reporter.spinner("download");
            temp_input = Some(input_source::from_url(&url, spec)?);
        }
        cli.inputs[0] = temp_input.as_ref().unwrap().0.clone();
    }

    // Batch when given a folder, several inputs, or an explicit --out-dir;
    // otherwise the classic single-file path (result to stdout or -o).
    let single = cli.inputs.len() == 1 && cli.inputs[0].is_file() && cli.out_dir.is_none();
    if !single {
        batch::run(&cli, &reporter, password.clone())?;
        if cli.stats {
            resources::report(&reporter, run_start.elapsed());
        }
        return Ok(());
    }

    let input = &cli.inputs[0];
    if let Some(dir) = &cli.cache_dir {
        eprintln!(
            "note: --cache-dir {} applies to batch runs (folder / multiple inputs / --out-dir); ignoring",
            dir.display()
        );
    }
    let input_bytes = std::fs::metadata(input).map(|m| m.len()).unwrap_or(0);

    let models = RunModels::from_cli(&cli);
    let doc = parse_and_enhance(input, &cli, &models, Some(&reporter), password.clone())?;

    if cli.quality {
        eprintln!("{}", docparse_core::quality::analyze(&doc).to_json());
    }
    if cli.profile {
        eprintln!(
            "{}",
            docparse_core::quality::profile_json(&docparse_core::quality::profile(&doc))
        );
    }
    if cli.route_plan {
        // No enhancers registered in the CLI; the plan shows which pages WOULD
        // need a model — on a digital document this is empty (cost stays low).
        let plan = docparse_core::enhance::plan(&doc, &[]);
        eprintln!(
            "{{\"hard_pages\": {}, \"total_pages\": {}, \"routes\": {}}}",
            plan.len(),
            doc.pages.len(),
            docparse_core::enhance::report_json(&plan)
        );
    }

    if let Some(t) = cli.quality_threshold {
        eprintln!(
            "{}",
            serde_json::to_string_pretty(&docparse_core::quality::review_pages(&doc, t))
                .unwrap_or_default()
        );
    }

    // End-of-run speed summary (pages · MB · wall · pages/s · MB/s). No-op when
    // progress is disabled; printed to stderr so stdout stays pure data.
    reporter.finish(
        &input.file_name().map_or_else(
            || input.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        ),
        doc.pages.len(),
        input_bytes,
    );

    // OKF is a directory bundle, not a stream — handle it before render_doc.
    if matches!(cli.format, Format::Okf) {
        if cli.okf_tar {
            // Deterministic tar to stdout (for `| tar x` / upload).
            use std::io::Write;
            let bundle = docparse_core::okf::build(&doc, &okf_options(&cli, input));
            std::io::stdout().write_all(&bundle.to_tar())?;
        } else {
            let dir = cli.out.clone().unwrap_or_else(|| derived_okf_dir(input));
            let explicit = cli.out.is_some();
            write_okf_bundle(&doc, &cli, input, &dir, explicit)?;
        }
    } else {
        let rendered = render_doc(&doc, &cli)?;
        match &cli.out {
            Some(path) => std::fs::write(path, rendered)?,
            None => println!("{rendered}"),
        }
    }
    if cli.stats {
        resources::report(&reporter, run_start.elapsed());
    }
    Ok(())
}

/// Parse one input and apply every enabled enhancement phase, returning the
/// finished document (empty-table placeholders dropped). Shared by the
/// single-file path and the batch runner.
///
/// `reporter`: `Some` shows a per-phase spinner / OCR page bar and emits the
/// per-phase JSON count lines on stderr (single-file behavior); `None` runs
/// quiet — batch mode's file bar + aggregate report stand in. A phase that
/// doesn't apply to the input's format is skipped; failures propagate so the
/// caller can record them.
fn parse_and_enhance(
    input: &std::path::Path,
    cli: &Cli,
    models: &RunModels,
    reporter: Option<&progress::Reporter>,
    password: Option<String>,
) -> anyhow::Result<docparse_core::ir::Document> {
    let is_pdf = input
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false);
    let log = reporter.is_some();

    let mut doc = {
        let _g = reporter.map(|r| r.spinner("parse"));
        parse_path_with(
            input,
            cli.image_dir.is_some() || cli.image_embed,
            password.clone(),
        )?
    };

    // --pages: filter after parsing, before every enhancement — the models
    // (OCR/layout/UniRec) only ever see the kept pages. Page numbers stay
    // absolute (see core::pages), so downstream citations are unaffected.
    if let Some(spec) = &cli.pages {
        docparse_core::pages::retain_pages_spec(&mut doc, spec)?;
    }

    if let Some(dir) = &cli.image_dir {
        let n = export_images(&mut doc, dir)?;
        if log {
            eprintln!("{{\"images_exported\": {n}}}");
        }
    }
    if cli.image_embed {
        let n = embed_images(&mut doc);
        if log {
            eprintln!("{{\"images_embedded\": {n}}}");
        }
    }

    if cli.ocr {
        // Load models only when some page actually needs enhancement — a
        // fully digital document with --ocr must stay zero-cost (and must not
        // fail on a missing model dir it would never use).
        let needs = docparse_core::quality::assess_pages(&doc)
            .iter()
            .any(|a| a.needs_enhancement);
        if needs {
            // Loaded once per run and cached (lazy): a batch of scans reads the
            // model on the first scanned page, not once per file.
            let ocr = models
                .ocr
                .get()
                .map_err(|e| anyhow::anyhow!("ocr models unavailable: {e}"))?;
            let (enhanced, report) = match reporter {
                Some(r) => {
                    let (bar, _g) = r.page_bar("ocr", doc.pages.len() as u64);
                    match &bar {
                        Some(b) => {
                            let b = b.clone();
                            let on_page = move || b.inc(1);
                            apply_ocr_with(doc, input, ocr, Some(&on_page))?
                        }
                        None => apply_ocr_with(doc, input, ocr, None)?,
                    }
                }
                None => apply_ocr_with(doc, input, ocr, None)?,
            };
            doc = enhanced;
            if log {
                eprintln!("{}", docparse_core::enhance::report_json(&report));
            }
        } else if log {
            eprintln!("[]");
        }
    }

    if cli.layout {
        if is_pdf {
            let pdf_bytes = std::fs::read(input)?;
            let layout = models.layout.get(&cli.layout_model)?;
            let n = {
                let _g = reporter.map(|r| r.spinner("layout"));
                docparse_ocr::layout::enhance_document(&mut doc, pdf_bytes, layout, 2.0)?
            };
            if log {
                eprintln!("{{\"layout_enhanced_pages\": {n}}}");
            }
        } else if log {
            eprintln!("--layout currently supports PDF inputs only; skipped");
        }
    }

    if let Some(dir) = &cli.table_model {
        if !is_pdf {
            if log {
                eprintln!("--table-model currently supports PDF inputs only; skipped");
            }
        } else {
            let model = models.table.get(dir)?;
            let n = {
                let _g = reporter.map(|r| r.spinner("table"));
                docparse_ocr::table_model::refine_tables(&mut doc, std::fs::read(input)?, model)?
            };
            if log {
                eprintln!("{{\"table_model_refined\": {n}}}");
            }
        }
    }

    if let Some(dir) = &cli.formula_model {
        if !is_pdf {
            if log {
                eprintln!("--formula-model currently supports PDF inputs only; skipped");
            }
        } else {
            let layout = models.layout.get(&cli.layout_model)?;
            let model = models.formula.get(dir)?;
            let n = {
                let _g = reporter.map(|r| r.spinner("formula"));
                docparse_ocr::formula::enhance_formulas(
                    &mut doc,
                    std::fs::read(input)?,
                    layout,
                    model,
                )?
            };
            if log {
                eprintln!("{{\"formula_model_replaced\": {n}}}");
            }
        }
    }

    if let Some(dir) = &cli.transcribe_model {
        if !is_pdf {
            if log {
                eprintln!("--transcribe-model currently supports PDF inputs only; skipped");
            }
        } else {
            let layout = models.layout.get(&cli.layout_model)?;
            let model = models.transcribe.get(dir)?;
            let n = {
                let _g = reporter.map(|r| r.spinner("transcribe"));
                docparse_ocr::transcribe::transcribe_pages(
                    &mut doc,
                    std::fs::read(input)?,
                    layout,
                    model,
                )?
            };
            if log {
                eprintln!("{{\"transcribed_pages\": {n}}}");
            }
        }
    }

    if cli.vlm_describe || cli.vlm_tables {
        if !is_pdf {
            if log {
                eprintln!("--vlm-describe/--vlm-tables currently support PDF inputs only; skipped");
            }
        } else {
            let (url, model) = match (cli.vlm_url.clone(), cli.vlm_model.clone()) {
                (Some(u), Some(m)) => (u, m),
                _ => anyhow::bail!("--vlm-describe/--vlm-tables require --vlm-url and --vlm-model"),
            };
            let client = docparse_vlm::VlmClient::new(docparse_vlm::VlmConfig {
                url,
                model,
                api_key: cli.vlm_api_key.clone(),
            });
            if cli.vlm_describe {
                let n = {
                    let _g = reporter.map(|r| r.spinner("vlm-describe"));
                    docparse_vlm::annotate_pictures(&mut doc, std::fs::read(input)?, &client)?
                };
                if log {
                    eprintln!("{{\"vlm_described_figures\": {n}}}");
                }
            }
            if cli.vlm_tables {
                let n = {
                    let _g = reporter.map(|r| r.spinner("vlm-tables"));
                    docparse_vlm::refine_tables(&mut doc, std::fs::read(input)?, &client)?
                };
                if log {
                    eprintln!("{{\"vlm_refined_tables\": {n}}}");
                }
            }
        }
    }

    // After all enhancers: drop empty-row table placeholders before any output
    // or quality/profile pass sees them.
    drop_empty_table_placeholders(&mut doc);
    Ok(doc)
}

/// Render a finished document into the requested output format. Shared by the
/// single-file path and the batch runner.
/// `docparse locate`: parse → chunk → point lookup, printing the covering
/// chunk (JSON or text). A miss prints `null` / an empty line and still exits
/// 0 — a miss is a query result, not an error (same semantics as the MCP
/// `locate` tool). Deterministic only: no model flags (the enhanced lookup
/// lives in the MCP face).
fn run_locate(
    file: &std::path::Path,
    page: usize,
    x: f32,
    y: f32,
    top_left: bool,
    format: LocateFormat,
    password: Option<String>,
) -> anyhow::Result<()> {
    let doc = parse_path_with(file, false, password)?;
    let page_ref = doc.pages.iter().find(|p| p.number == page).ok_or_else(|| {
        anyhow::anyhow!("no page {page} (document has {} pages)", doc.pages.len())
    })?;
    // --top-left flips the image/screenshot convention into PDF user space.
    let y_user = if top_left { page_ref.height - y } else { y };
    let chunks = docparse_core::chunk::chunk_document(&doc);
    let hit = docparse_core::chunk::locate(&chunks, page, x, y_user);
    match (format, hit) {
        (LocateFormat::Json, Some(chunk)) => {
            println!("{}", serde_json::to_string_pretty(chunk)?)
        }
        (LocateFormat::Json, None) => println!("null"),
        (LocateFormat::Text, Some(chunk)) => println!("{}", chunk.text),
        (LocateFormat::Text, None) => println!(),
    }
    Ok(())
}

fn render_doc(doc: &docparse_core::ir::Document, cli: &Cli) -> anyhow::Result<String> {
    Ok(match cli.format {
        Format::Json => output::to_json(doc)?,
        Format::Markdown => output::to_markdown(doc),
        Format::Text => output::to_text(doc),
        Format::Chunks => {
            let opts = docparse_core::chunk::ChunkOptions {
                target_chars: cli.chunk_target_chars,
                table_markdown: matches!(cli.table_format, TableFormat::Markdown),
            };
            docparse_core::chunk::to_json(&docparse_core::chunk::chunk_document_with(doc, opts))
        }
        Format::Outline => docparse_core::outline::to_json(&docparse_core::outline::build(doc)),
        Format::Meta => docparse_core::meta::to_json(&docparse_core::meta::report(doc)),
        // OKF writes a directory bundle, never a string — handled out-of-band.
        Format::Okf => unreachable!("okf is written via write_okf_bundle, not render_doc"),
    })
}

/// Auto-derived OKF bundle directory for `input`: `<stem>-okf/` in the cwd.
fn derived_okf_dir(input: &std::path::Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".into());
    PathBuf::from(format!("{stem}-okf"))
}

/// Build the OKF options for `input` (basename + mtime + resource base) and
/// write the bundle under `dir`. An auto-derived (`!explicit`) non-empty dir is
/// refused unless `--force`; an explicit `-o` dir is trusted.
fn write_okf_bundle(
    doc: &docparse_core::ir::Document,
    cli: &Cli,
    input: &std::path::Path,
    dir: &std::path::Path,
    explicit: bool,
) -> anyhow::Result<()> {
    if !explicit && !cli.force && dir_nonempty(dir) {
        anyhow::bail!(
            "{} exists and is not empty; use -o to target it or --force to overwrite",
            dir.display()
        );
    }
    let opts = okf_options(cli, input);
    let bundle = docparse_core::okf::build(doc, &opts);
    let concepts = bundle
        .files
        .iter()
        .filter(|(p, _)| p.file_name().and_then(|n| n.to_str()) != Some("index.md"))
        .count();
    bundle.write_to(dir)?;
    eprintln!(
        "wrote OKF bundle to {}/ ({concepts} concept(s))",
        dir.display()
    );
    Ok(())
}

/// Assemble [`docparse_core::okf::OkfOptions`] from the CLI + source file: the
/// basename for `resource` URIs and the file's mtime as a deterministic
/// ISO 8601 timestamp (never the wall clock).
pub(crate) fn okf_options(cli: &Cli, input: &std::path::Path) -> docparse_core::okf::OkfOptions {
    okf_options_for(
        input,
        cli.okf_resource_base.clone().unwrap_or_default(),
        matches!(cli.table_format, TableFormat::Markdown),
    )
}

/// `OkfOptions` from a source path alone (basename + mtime timestamp) — shared
/// by the CLI and the MCP/REST `okf` surfaces, which have no `Cli`.
pub(crate) fn okf_options_for(
    input: &std::path::Path,
    resource_base: String,
    table_markdown: bool,
) -> docparse_core::okf::OkfOptions {
    let source_name = input
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let timestamp = std::fs::metadata(input)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| iso8601_utc(d.as_secs()));
    docparse_core::okf::OkfOptions {
        resource_base,
        source_name,
        timestamp,
        table_markdown,
    }
}

/// True if `dir` exists and contains at least one entry.
fn dir_nonempty(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
}

/// Format Unix seconds as `YYYY-MM-DDTHH:MM:SSZ` (UTC), dependency-free via the
/// days-from-civil algorithm — deterministic, so bundles stay byte-identical.
fn iso8601_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant's civil_from_days (epoch 1970-01-01).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

#[cfg(test)]
mod password_tests {
    use super::resolve_password;

    #[test]
    fn explicit_wins_over_file_and_env() {
        assert_eq!(
            resolve_password(Some("a".into()), Some("DOCPARSE_UNSET_VAR"), None)
                .unwrap()
                .as_deref(),
            Some("a")
        );
    }

    #[test]
    fn file_strips_trailing_newline() {
        let dir = std::env::temp_dir().join(format!("docparse-pwfile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("secret.txt");
        std::fs::write(&f, "hunter2\n").unwrap();
        assert_eq!(
            resolve_password(None, None, Some(&f)).unwrap().as_deref(),
            Some("hunter2")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_reads_and_missing_env_is_an_error_not_silent_none() {
        unsafe {
            std::env::set_var("DOCPARSE_TEST_PW_22", "envpw");
        }
        assert_eq!(
            resolve_password(None, Some("DOCPARSE_TEST_PW_22"), None)
                .unwrap()
                .as_deref(),
            Some("envpw")
        );
        let err = resolve_password(None, Some("DOCPARSE_TEST_UNSET_22"), None).unwrap_err();
        assert!(
            format!("{err}").contains("DOCPARSE_TEST_UNSET_22"),
            "got: {err}"
        );
    }

    #[test]
    fn unreadable_file_is_an_error() {
        let err =
            resolve_password(None, None, Some(std::path::Path::new("/nonexistent/x"))).unwrap_err();
        assert!(format!("{err}").contains("--password-file"), "got: {err}");
    }

    #[test]
    fn none_when_no_source_configured() {
        assert!(resolve_password(None, None, None).unwrap().is_none());
    }
}

#[cfg(test)]
pub(crate) mod test_fixtures {
    /// Minimal encrypted PDF: Standard security handler, Revision 2, 40-bit
    /// RC4 keys (PDF 1.4). Produced through lopdf's *own public encrypt API*
    /// — the same library docparse uses to decrypt — so the fixture can never
    /// drift from what the parser actually supports. One page, Helvetica
    /// Type1 (a standard-14 font: ASCII text extracts without any embedded
    /// font), content stream "Secret".
    pub(crate) fn encrypted_pdf(password: &str) -> Vec<u8> {
        use lopdf::{
            dictionary, Document, EncryptionState, EncryptionVersion, Object, Permissions, Stream,
        };

        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content_id = doc.add_object(Stream::new(
            dictionary! {},
            b"BT /F1 24 Tf 72 720 Td (Secret) Tj ET".to_vec(),
        ));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        doc.trailer.set(
            "ID",
            Object::Array(vec![
                Object::string_literal(b"docparse-test-id"),
                Object::string_literal(b"docparse-test-id"),
            ]),
        );

        let version = EncryptionVersion::V2 {
            document: &doc,
            owner_password: "docparse-owner",
            user_password: password,
            key_length: 40,
            permissions: Permissions::all(),
        };
        let state = EncryptionState::try_from(version).expect("valid v2 encryption state");
        doc.encrypt(&state).expect("encrypt fixture");

        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("save fixture");
        bytes
    }
}

#[cfg(test)]
mod pages_tests {
    use super::*;

    /// Minimal N-page PDF (same fixture style as [`test_fixtures::encrypted_pdf`],
    /// minus encryption): one Helvetica page per n, content "Page <i>".
    pub(crate) fn multi_page_pdf(n: usize) -> Vec<u8> {
        use lopdf::{dictionary, Document, Object, Stream};
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let mut kids = Vec::new();
        for i in 1..=n {
            // Vary y per page: identical text at an identical position on
            // every page is precisely a "running header" pattern, and the
            // layout layer would correctly drop it from body output — the
            // fixture must look like real body text, not a header.
            let y = 720 - (i as i64 - 1) * 60;
            let content_id = doc.add_object(Stream::new(
                dictionary! {},
                format!("BT /F1 24 Tf 72 {y} Td (Page {i} lorem docparse) Tj ET").into_bytes(),
            ));
            let page_id = doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content_id,
            });
            kids.push(page_id.into());
        }
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => Object::Integer(n as i64),
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("save fixture");
        bytes
    }

    fn temp_pdf(name: &str, n: usize) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, multi_page_pdf(n)).unwrap();
        path
    }

    fn cli_for(args: &[&str]) -> Cli {
        let mut full = vec!["docparse"];
        full.extend_from_slice(args);
        Cli::try_parse_from(full).expect("cli parses")
    }

    #[test]
    fn pages_flag_filters_with_absolute_numbering() {
        let path = temp_pdf("docparse-pages-3.pdf", 3);
        let models = RunModels::from_cli(&cli_for(&["x.pdf"]));
        let cli = cli_for(&["x.pdf", "--pages", "2"]);

        let doc = parse_and_enhance(&path, &cli, &models, None, None).expect("parses");
        assert_eq!(doc.pages.len(), 1, "only the requested page survives");
        assert_eq!(doc.pages[0].number, 2, "absolute numbering, not remapped");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn pages_out_of_range_is_an_error_not_a_truncate() {
        let path = temp_pdf("docparse-pages-range.pdf", 3);
        let models = RunModels::from_cli(&cli_for(&["x.pdf"]));
        let cli = cli_for(&["x.pdf", "--pages", "9"]);

        let err = parse_and_enhance(&path, &cli, &models, None, None)
            .expect_err("beyond-the-end page must error");
        assert!(err.to_string().contains("out of range"), "{err}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn pages_spec_syntax_errors_surface_from_the_cli_path() {
        let path = temp_pdf("docparse-pages-syntax.pdf", 3);
        let models = RunModels::from_cli(&cli_for(&["x.pdf"]));
        let cli = cli_for(&["x.pdf", "--pages", "3-1"]);

        let err = parse_and_enhance(&path, &cli, &models, None, None).expect_err("bad spec");
        assert!(err.to_string().contains("starts after it ends"), "{err}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn pages_arg_is_visible_in_help() {
        // The flag must exist on the *main* Cli (agent-facing contract), not
        // just be tolerated.
        let err = Cli::try_parse_from(["docparse", "--help"])
            .err()
            .expect("help exits");
        assert!(err.to_string().contains("--pages"), "{err}");
    }
}

#[cfg(test)]
mod pages_render_regression {
    use super::*;

    /// `--pages` must not disturb rendering: the same text comes out of a
    /// ranged document as from the full one (minus the dropped pages), and a
    /// well-formed multi-page PDF renders every page's text via `-f text`.
    #[test]
    fn pages_filter_keeps_text_rendering_intact() {
        use docparse_core::output;
        let path = std::env::temp_dir().join("docparse-pages-render.pdf");
        std::fs::write(&path, pages_tests::multi_page_pdf(3)).unwrap();
        let models = RunModels::from_cli(&cli_for_static());

        let mut cli = cli_for_static();
        let full = parse_and_enhance(&path, &cli, &models, None, None).unwrap();
        cli.pages = Some("2".into());
        let ranged = parse_and_enhance(&path, &cli, &models, None, None).unwrap();

        let full_text = output::to_text(&full);
        assert!(
            full_text.contains("Page 1"),
            "multi-page text renders: {full_text:?}"
        );
        assert!(full_text.contains("Page 2") && full_text.contains("Page 3"));
        let ranged_text = output::to_text(&ranged);
        assert_eq!(
            ranged_text,
            output::to_text(&{
                let mut d = full.clone();
                d.pages.retain(|p| p.number == 2);
                d
            }),
            "--pages output must equal manually dropping the same pages"
        );
        std::fs::remove_file(&path).ok();
    }

    fn cli_for_static() -> Cli {
        Cli::try_parse_from(["docparse", "x.pdf"]).expect("cli parses")
    }
}

#[cfg(test)]
mod meta_tests {
    use super::*;
    use docparse_core::ir::Metadata;

    /// Minimal 1-page PDF with a populated Info dictionary (same fixture
    /// style as [`pages_tests`], plus trailer /Info).
    fn pdf_with_info() -> Vec<u8> {
        use lopdf::{dictionary, Document, Object, Stream};
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content_id = doc.add_object(Stream::new(
            dictionary! {},
            b"BT /F1 24 Tf 72 700 Td (Meta fixture) Tj ET".to_vec(),
        ));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => Object::Integer(1),
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        // Real PDFs carry /Info as an *indirect reference* — inline it and the
        // metadata reader (which resolves the reference, like real files) sees
        // nothing.
        // NB: lopdf's `From<&str>` yields a *Name* object — Info strings must
        // be built with `string_literal` (or `Object::String`) or they come
        // back as names, not strings.
        let info_id = doc.add_object(dictionary! {
            "Title" => Object::string_literal("Quarterly Report"),
            "Author" => Object::string_literal("Jane Chen"),
            "CreationDate" => Object::string_literal("D:20260906093000+02'00'"),
        });
        doc.trailer.set("Info", info_id);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("save fixture");
        bytes
    }

    fn parse_fixture(bytes: &[u8], name: &str, fmt: Format) -> anyhow::Result<String> {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, bytes).unwrap();
        let cli = Cli::try_parse_from(["docparse", "x", "-f", "meta"]).unwrap();
        let mut cli = cli;
        cli.format = fmt;
        let models = RunModels::from_cli(&cli);
        let doc = parse_and_enhance(&path, &cli, &models, None, None)?;
        let rendered = render_doc(&doc, &cli)?;
        std::fs::remove_file(&path).ok();
        Ok(rendered)
    }

    #[test]
    fn pdf_info_flows_into_meta_and_json() {
        let bytes = pdf_with_info();
        let meta = parse_fixture(&bytes, "docparse-meta-info.pdf", Format::Meta).unwrap();
        assert!(meta.contains("\"title\": \"Quarterly Report\""), "{meta}");
        assert!(meta.contains("\"author\": \"Jane Chen\""), "{meta}");
        // D:20260906093000+02'00' → 07:30 UTC.
        assert!(
            meta.contains("\"created\": \"2026-09-06T07:30:00Z\""),
            "{meta}"
        );
        assert!(meta.contains("\"parser\": \"pdf\""), "{meta}");

        let json = parse_fixture(&bytes, "docparse-meta-info.json.pdf", Format::Json).unwrap();
        assert!(
            json.contains("\"metadata\""),
            "json carries the metadata field"
        );
        assert!(json.contains("Quarterly Report"));
    }

    #[test]
    fn no_metadata_source_keeps_json_bytes_free_of_the_field() {
        let csv = b"name,score\nada,99\n";
        let json = parse_fixture(csv, "docparse-meta-plain.csv", Format::Json).unwrap();
        assert!(
            !json.contains("\"metadata\""),
            "metadata: None must be skipped so legacy consumers see identical bytes"
        );
        let meta = parse_fixture(csv, "docparse-meta-plain.meta.csv", Format::Meta).unwrap();
        assert!(!meta.contains("\"metadata\""), "{meta}");
        assert!(meta.contains("\"parser\": \"csv\""), "{meta}");
    }

    #[test]
    fn meta_schema_is_registered() {
        let s = docparse_core::schema::by_name("meta").expect("meta schema");
        assert!(s["properties"].get("metadata").is_some());
        assert!(s["properties"].get("page_count").is_some());
    }

    // Silence the unused-import lint when only some fixtures run.
    #[allow(dead_code)]
    fn _meta_type_witness(_: Option<Metadata>) {}
}

#[cfg(test)]
mod locate_tests {
    use super::*;

    #[test]
    fn locate_hits_in_user_space_and_top_left() {
        let path = std::env::temp_dir().join("docparse-locate.pdf");
        std::fs::write(&path, pages_tests::multi_page_pdf(3)).unwrap();

        // Page 2's text sits at y=660..684 (fixture varies y per page).
        // The same physical point expressed in top-left coordinates must hit
        // the same chunk: 792 - 670 = 122.
        for (label, args) in [
            ("user space", vec!["docparse", "locate"]),
            ("top-left", vec!["docparse", "locate", "--top-left"]),
        ] {
            let _ = label;
            let mut full = args;
            full.extend([
                path.to_str().unwrap(),
                "--page",
                "2",
                "--x",
                "100",
                "--y",
                if full.contains(&"--top-left") {
                    "122"
                } else {
                    "670"
                },
            ]);
            let cli = Cli::try_parse_from(&full).expect("cli parses");
            let Command::Locate {
                file,
                page,
                x,
                y,
                top_left,
                ..
            } = cli.command.expect("subcommand")
            else {
                panic!("expected locate subcommand");
            };
            let doc = parse_path_with(&file, false, None).unwrap();
            let page_ref = doc.pages.iter().find(|p| p.number == page).unwrap();
            let y_user = if top_left { page_ref.height - y } else { y };
            let chunks = docparse_core::chunk::chunk_document(&doc);
            let hit = docparse_core::chunk::locate(&chunks, page, x, y_user)
                .unwrap_or_else(|| panic!("must hit in {label:?}"));
            assert!(hit.text.contains("Page 2"), "{hit:?}");
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn locate_page_out_of_range_is_an_error() {
        let path = std::env::temp_dir().join("docparse-locate-range.pdf");
        std::fs::write(&path, pages_tests::multi_page_pdf(2)).unwrap();
        let doc = parse_path_with(&path, false, None).unwrap();
        let missing = doc.pages.iter().find(|p| p.number == 9);
        assert!(missing.is_none(), "fixture must not have page 9");
        std::fs::remove_file(&path).ok();
    }
}
