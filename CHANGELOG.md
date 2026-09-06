# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Continued/headerless tables in Markdown & text: a table row that does not
  look like a header (empty cell, or every cell numeric) no longer renders as
  a fake header with a `---` separator — the data rows render bare with an
  explanatory comment. When such a table follows a same-column-count table,
  the previous table's header is inherited (`<!-- continued table: header
  inherited from the previous table -->`, also across pages), so a table split
  across pages or fragmented by detection keeps its column names. Text output
  mirrors this with `[continued table]` / `[table without header]` prefixes.
  New `looks_like_header_row` predicate in `table.rs`; e2e on a real paper
  shows the Table-3 continuation no longer mislabels `32 / 5.01` as headers.
- Unified reading order across every output format: tables and images are
  spliced back into the page's reading position instead of being dumped at the
  page bottom. The chunk layer's geometry splice (`follows`: horizontal overlap
  + float below a block) moved up into a shared `layout::page_items` /
  `page_items_chunk` ordering consumed by markdown, text and RAG chunks alike,
  so the three consumers now see the same order (previously RAG spliced
  mid-text while markdown/text parked tables at the page end). Multi-column
  pages stay column-safe: a right-column float never jumps ahead of a
  left-column paragraph. Caption helpers and constants moved from `chunk.rs`
  into `layout.rs` (`IMAGE_ADJ_GAP`, `MIN_IMAGE_COVERAGE`, `find_caption_idx`).
  e2e on a real two-column PDF with a ruled middle table: markdown/text/chunks
  all place the table between the column text and the bottom paragraph.
- `--password-env <VAR>` / `--password-file <PATH>` — safe password injection
  for the CLI (`--password` counterpart, mutually exclusive): read the
  encrypted-PDF password from an environment variable or a secrets file instead
  of the process list. A missing env var is an error (never silently "no
  password"); a trailing newline on the file (Docker secrets / `.secret`) is
  stripped. `serve`/`mcp` gain the same three sources as a **startup default**:
  REST requests without `?password=` and MCP calls without a `password`
  argument fall back to it (OpenAPI documents this), and the default still keys
  the server document cache per-password. Error messages now list all three
  injection options.
- `serve`/`mcp --password <PW>` — encrypted-PDF password end-to-end: REST
  `?password=` (documented in OpenAPI, with the localhost/LAN caveat) and an
  optional `password` argument on all five MCP tools. The password is part of
  the document-cache signature, so a different password (or none) never
  replays another password's parse — entries are keyed per (content,
  password) and verified e2e (miss→hit byte-identical, per-password entries).
  Absent/wrong passwords keep their actionable errors (`--password` hint /
  invalid-password). Test fixture uses lopdf's own public encrypt API
  (EncryptionVersion::V2) to mint a real R2/RC4-40 PDF.
- `serve`/`mcp --cache-dir <DIR>` — server-side document cache: repeated
  uploads (REST) or agent calls (MCP) of the same content + enhancement flags
  replay the stored **enhanced document** instead of re-parsing (the shared
  heavy step behind every format/tool). Key = (source name or path, content
  SHA-256, enhancement + model-set signature); format/tool arguments don't
  fragment the cache because rendering happens after the lookup. REST reports
  `x-docparse-cache: hit|miss` (the body stays byte-identical); MCP is pure
  acceleration. `format=okf` rides the same document cache (the tar itself is
  still rebuilt per request).
- `--cache-dir <DIR>` — incremental batch cache: a re-run of the same corpus
  folder skips files whose content (SHA-256) and output options match a
  previous run, replaying the stored output — the heavy work (OCR / layout /
  UniRec inference) is skipped entirely. Keyed by path+content-hash+output
  signature; a content or flag change mints a fresh key and simply re-parses
  (stale entries are never consulted). Needs --out-dir; not applied to -f
  okf. Batch report marks cached files (`cached` column / JSON field).
- `fetch-models <tier>` — install the optional neural model tiers in pure
  Rust over the HuggingFace tree API: `ocr` / `ppocr-v6` (default OCR) /
  `layout` / `unirec` / `ppv2` / `all`, into `--dir` (default `models/`, tier
  subdirs like the old script). No `hf` CLI, Python or shell required; the
  first-use OCR prompt and `scripts/fetch-models.sh` (now a thin wrapper)
  both point here. Globs match against the live repo listing, so specs
  survive repo reorganizations.
- `--password <pw>` — parse encrypted PDFs (standard security handler via
  lopdf: RC4 R2-R4, AES-128 V4, AES-256 V5/R5/R6). Loading an encrypted PDF
  without a password now fails with an actionable message instead of parsing
  undecrypted garbage; wrong passwords report "invalid password". Additive:
  unencrypted files are unaffected and the four interfaces stay byte-identical
  (MCP/REST still default to no password).
- `--chunk-target-chars <n>` — wire the existing core chunk-size knob into the
  CLI: consecutive paragraphs accumulate up to `n` chars per RAG chunk (default
  800, byte-identical when omitted). Smaller values yield finer-grained chunks
  for dense vector indexes; headings / lists / code / tables stay atomic.
- `--quality-threshold <float>` — review gate for corpus ingestion: pages with
  no text layer, or a garbled-character ratio above the threshold, are listed
  as JSON on stderr (page / chars / garbled ratio / flags / reasons) for human
  review before RAG ingestion. Deterministic, model-free, additive — without
  the flag, output bytes are unchanged.
- `scripts/fetch-models.sh` — per-tier downloader for the optional neural models
  (`ocr` / `layout` / `unirec` / `ppv2`), pulled from their original Apache-2.0
  repos. Models are never bundled in the repo or binary.
- `LICENSE` (Apache-2.0) and `NOTICE` (third-party attributions: vendored tract
  patches, veraPDF algorithmic reference, optional models, Rust dependencies).
- `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, and this changelog.

## [0.1.0]

First public release. Pure-Rust, multi-format document parser optimized for
speed and deterministic output.

### Added
- **Formats** — PDF, DOCX, HTML, XLSX, PPTX, Markdown, CSV, SRT/VTT, LaTeX,
  EML, PNG/JPEG, AsciiDoc. Each is a `DocumentParser` over a shared,
  format-agnostic core (reading order + output).
- **PDF engine** — self-built content-stream interpreter (graphics/text matrix
  state machine emitting positioned chunks) and font layer (ToUnicode CMap /
  AFM / Encoding), independently implemented with veraPDF as the *algorithmic*
  reference (no veraPDF code).
- **Layout** — paragraph aggregation, header/footer detection, XY-cut +
  multi-column reading order; bordered/ruled/cluster/borderless table detection.
- **Optional neural enhancers** (opt-in, external models) — `--ocr` (PP-OCRv4),
  `--layout` (DocLayout-YOLO default; PP-DocLayoutV2 second backend),
  `--table-model` / `--formula-model` / `--transcribe-model` (UniRec-0.1B), and
  VLM-based description over an OpenAI-compatible protocol.
- **Outputs** — JSON / Markdown / Text / RAG chunks with per-chunk provenance
  (bbox / page / confidence) and `locate(x,y)` reverse lookup.
- **Interfaces** — CLI, library, MCP stdio server, and REST server, all sharing
  one parse path with byte-identical output.
- **Vendored tract patches** — two minimal, attributed fixes (GatherNd shape
  inference + TopK TDim) that let PP-DocLayoutV2 run on pure-Rust tract; kept
  vendored on `main` by design (see `vendor/README.md`).

[Unreleased]: https://github.com/yzlabai/docparse-rs/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/yzlabai/docparse-rs/releases/tag/v0.1.0
