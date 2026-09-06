# 2026-09-06 — Server-side document cache (`serve` / `mcp --cache-dir`)

Phase 20: wire the CLI's incremental cache into the serving faces, so agents
and REST clients stop re-paying the heavy parse (OCR / layout / UniRec
inference) for the same document.

## Why

- **RAG agent loops** call the same file repeatedly: `get_chunks` →
  `outline` → `locate` → `parse_document` on one PDF. Today each call
  re-parses *and* re-applies OCR — the whole pipeline, every time.
- **REST** clients polling a watched folder upload the same bytes over and
  over.
- The batch face already had `--cache-dir` (Phase 19); the serving faces had
  no equivalent. This feature is the same pain, one tier up.

## Design

### Cached unit: the *enhanced document*, not the rendered output

The batch CLI caches its rendered output (one entry per file/format). The
serving faces cache the **document after parse + `state.apply`** — the shared
heavy step behind every format and tool:

- MCP's five tools (`parse_document` / `get_chunks` / `outline` /
  `export_okf` / `locate`) all render from the same document; one cache entry
  covers all of them.
- REST's `json` / `markdown` / `text` / `chunks` / `outline` / `okf` all
  render from the same document; format never fragments the cache.
- Tool/format arguments (`format`, `envelope`, `table_markdown`, `id`,
  `page`, `x`,`y`, …) affect rendering only, so they are **excluded from the
  signature** — rendering happens after the lookup.

### Key

`(identity, content SHA-256, enhancement + model-set signature)`:

- `identity` — REST: the sanitized upload name (it appears in the output via
  `doc.source`, so two names must not collide); MCP: the caller's path.
- `content_sha` — streamed SHA-256 of the staged temp file (REST) / the
  supplied file (MCP). A content change mints a fresh key and re-parses.
- signature — `EnhanceState::cache_signature`: every `EnhanceOpts` field plus
  the configured model paths / VLM identity, prefixed `srv1;` so serving
  entries never collide with batch entries. Model paths matter because a
  `--cache-dir` can outlive the process; a different model set must never
  replay another server's entries.

### Faces

- **REST** — `POST /parse` runs the lookup/replay inside the existing
  `spawn_blocking` (content hash is file I/O). A hit returns the stored
  document → `render_doc` → body byte-identical to a fresh parse. Observability
  rides in `x-docparse-cache: hit|miss` (added only when `--cache-dir` is
  set), following the `x-docparse-ms` precedent — the body-stays-identical
  contract is untouched. `format=okf` rides the same document cache; the tar
  itself is still rebuilt per request (its source name / mtime come from the
  staging file — pre-existing behavior, unchanged).
- **MCP** — `parse_enhanced` routes through `parse_enhanced_cached` with no
  source override. JSON-RPC has no header channel and tool outputs must stay
  byte-identical, so a hit is **pure acceleration, unmarked**.

### Plumbing

- `cache.rs` — generalized to `lookup_identity` / `store_identity`
  (path-free); the CLI path-based `lookup` / `store` are thin wrappers, so
  batch behavior and byte layout are unchanged (existing cache dirs survive).
- `main.rs` — `parse_enhanced_cached(path, source_name, opts, state) ->
  (Document, Option<bool>)` (None = caching off, Some(true) = hit, Some(false)
  = miss-and-stored); `EnhanceState.cache_dir` + `.with_cache_dir(..)`;
  `cache_signature`.
- `server.rs` — `render` split into cache-aware parse + pure `render_doc`;
  handler adds the cache header; OpenAPI documents both response headers.
- `mcp.rs` — `parse_enhanced` now routes through the cache (one-line change at
  the call site; all five tools benefit automatically).

## Verification

- **Unit**: `cache::identity_variants_share_one_mechanism` (path-free
  round-trip + identity sensitivity); `server::cache_replays_document_byte_
  identically_and_misses_on_change` (miss → store → hit, byte equality,
  content-change miss, source_name in key); `mcp::document_cache_shares_one_
  entry_across_tools` (get_chunks twice byte-identical + outline shares the
  single entry). Workspace ~120 tests green; clippy zero new warnings
  (baseline unchanged).
- **e2e (real binary)**: MCP stdio double-run byte-identical with exactly one
  cache entry across `get_chunks` + `outline`; REST two uploads — body
  byte-identical, `x-docparse-cache: miss` then `hit`. `--help` shows
  `--cache-dir` on both subcommands.

## Not done

- Cache pruning (stale entries linger for changed content/options — same as
  the CLI face; a future `--cache-prune` could walk `CacheEntry.sha256`
  against current keys).
- REST cache-dir is server-startup-only (no per-request opt-out).
