# CLI 接口层人机工学 · 第一批五项高频功能（--pages / stdin / URL / locate / -f meta）

> 计划稿（复杂需求，按 CLAUDE.md §0 复杂路径写）。原始一句话需求："补充高频、顺手、必须要用的功能"。
>
> 状态：**已实施完毕**（2026-09-06，P1→P5 五个 Phase 全部落地，见 status.md Phase 27–30 与对应 devlogs）。实施与计划的两处偏离均已回写：§3.2 core 增 quick-xml+zip 共享依赖（三 OOXML 后端 DRY，零新增供应链面）；P4/P5 合并为同一输入物化层 `cli/input_source.rs`。受限项：三件套字节回归待样例库环境补跑。

## 0. 原始需求

用户（agent 使用者 / RAG 管线工程师 / CLI 用户）在日常高频路径上，当前必须绕路完成五件事：

1. 只解析大文档的一部分页 → 目前只能全量解析后自行过滤 JSON；
2. 解析管道/网络流进来的内容 → 目前必须先落盘成带正确扩展名的文件；
3. 解析一个在线 URL → 目前必须先手动 curl 下载；
4. 用坐标反查切块（MCP 有 `locate` 招牌能力）→ CLI 用户完全没有入口；
5. 拿文档元数据（标题/作者/创建时间，RAG 入库必备）→ 目前 `provenance` 只有 parser 版本。

## 1. 需求三件套

**要解决什么**：补齐接口层的高频人机工学缺口。架构能力早已具备（IR 逐页、rayon 页并行、`chunk::locate` 已在 [chunk.rs:461](../../crates/docparse-core/src/chunk.rs) 实现并暴露给 MCP、各后端已解析 Info/core.xml 级信息但未上抛），缺的只是 CLI/REST/MCP 最后一层。

**给谁用**：
- CLI 用户（shell 工作流、快速查看、质检抽样）→ `--pages` / stdin / URL / locate
- Agent（Claude Code / Cursor 等，经 MCP）→ `pages` 参数、`-f meta`
- RAG 管线工程师（REST 集成、入库元数据）→ `?pages=` / `-f meta`

**成功长这样**（干净机器、零额外脚本）：

```bash
docparse big.pdf --pages 1-5,10 -f chunks        # 只处理并输出这些页，page 引用仍是绝对页码
curl -sL https://example.com/report.pdf | docparse - -f markdown          # 管道直接进
docparse https://example.com/report.pdf -f chunks                          # URL 直接进
docparse locate doc.pdf --page 3 --x 210 --y 700 --top-left                # 坐标反查（CLI 首次可用）
docparse doc.pdf -f meta                          # { source, format, metadata:{title,author,...}, page_count }
curl -F "file=@doc.pdf" "http://127.0.0.1:8642/parse?format=chunks&pages=1-5"
```

## 2. 范围与"不做什么"

**做**（五项，全部零新依赖树之外的东西；`ureq` 已在 workspace 依赖）：

| # | 功能 | 触契约？ |
|---|---|---|
| P1 | `--pages "1-5,10"`（CLI/REST/MCP） | cache 签名 + REST/MCP 参数 |
| P2 | `-f meta` 文档元数据 | **IR 加字段**（SCHEMA_VERSION 0.8.0 → 0.9.0，见 §3.2 取舍） |
| P3 | `docparse locate` 子命令 | 无（纯新增子命令） |
| P4 | stdin 管道输入 `-` | 无 |
| P5 | URL 直接输入 | 无（仅 CLI） |

**不做**（明确边界，防蔓延）：

- **REST/MCP 不加 URL 拉取**——服务端按用户提供的 URL 发请求是真 SSRF 面；URL 仅限本地 CLI（与 curl 同级的信任模型）。
- **locate v1 不接模型增强**（`--ocr` 等不进 locate 子命令）——增强场景 MCP `locate` 已覆盖；CLI locate 定位"确定性顺手查询"。
- **stdin v1 不做通用格式嗅探**——只嗅探 `%PDF-` magic（5 字节，可靠）；其余格式必须 `--input-format` 显式给出（报错信息列出全部合法值）。启发式文本嗅探（`<html`、`# `）是猜测，违背"不静默"原则。
- **XMP 元数据不做**（v1 只读 PDF Info 字典 / OOXML core.xml / HTML meta）——Info 覆盖绝大多数文档，XMP 解析是独立工程。
- **不做页码重映射**——过滤后 chunk/outline 的 `page` 保持原文档绝对页码（引用语义正确性的关键，见 §3.1）。
- **`--pages` 不下沉到 PDF 后端做选择性解析**——数字页快路径 ~700 页/s，全解析后过滤的代价可忽略；下沉需动 `DocumentParser` trait（12 个后端），违反 YAGNI。

## 3. 设计决策（每项：现状 → 决策 → 落点）

### 3.1 P1 `--pages`：解析后、增强前过滤

**现状**：无任何页选择能力；`doc.pages` 全量保留。

**决策**：
- 语法 `--pages "1-5,10,20-"`：1-based、**含端点**、逗号分隔；支持单页 `N`、闭区间 `N-M`、开放区间 `N-`（到文档末尾）。
- 时机：**解析完成 → 过滤 → 增强**。理由：① 12 种格式统一免费获得；② OCR/UniRec/版面等模型增强只对保留页跑——省模型成本正是页范围的核心价值；③ 质量评分/画像/路由计划语义正确（看到的页才是评分对象）。
- **页码保留绝对编号，不重映射**：过滤后 `Page.number`、chunk `page`、outline `page/bbox`、`section_id` 全部仍指原文档页码。下游引用（"见第 37 页"）跨页范围调用依然成立。
- 越界语义：显式列出的页码 > 文档页数 → **报错**（不静默截断）；`N-` 开放端自动截断到末尾；`N-M` 中 M 超界同报错。`start > end`、`N=0`、非数字 → 报错（清晰 message）。
- cache（两种 face 不对称且各自正确）：
  - **batch CLI**（缓存渲染后输出）：`pages` 进 `output_signature`（[cache.rs:87](../../crates/docparse-cli/src/cache.rs) 的规则注释已强制要求任何影响输出的 flag 必须加入）；
  - **serving faces**（缓存增强后全量 Document）：`pages` **不进**签名——过滤在 cache 回放之后、渲染之前，避免不同 `pages` 请求把缓存碎片化。
- 三面透传：CLI `Cli.pages: Option<String>`；REST `?pages=`；MCP `parse_document` / `get_chunks` / `outline` / `export_okf` 加 `pages` 参数（`locate` 无需）。batch 模式统一作用于批内每个文件。

**落点**：
- 新 [core/src/pages.rs](../../crates/docparse-core/src/pages.rs)：`parse_ranges(spec: &str, total: usize) -> anyhow::Result<BTreeSet<usize>>`（纯函数，全单测）+ `retain_pages(doc: &mut Document, keep: &BTreeSet<usize>)`；`lib.rs` 注册模块。放 core 而非 cli：三面共用一个实现，且过滤操作的就是 core 的 `Document`。
- [cli/main.rs](../../crates/docparse-cli/src/main.rs) `Cli` 加字段；`parse_and_enhance` 在 `parse_path_with` 返回后立即过滤（约 L1280 处）。
- [cli/server.rs](../../crates/docparse-cli/src/server.rs) 查询参数 + [cli/mcp.rs](../../crates/docparse-cli/src/mcp.rs) 各工具 schema。
- [cli/cache.rs](../../crates/docparse-cli/src/cache.rs) `output_signature` 加 `;pages={:?}`。

### 3.2 P2 `-f meta`：IR 加 `metadata` 字段（唯一触契约项）

**现状**：`Document { source, provenance, pages }`（[ir.rs:271](../../crates/docparse-core/src/ir.rs)），元数据信息在后端可及但全部丢弃。

**决策**：
- IR 变更（与 `provenance` 同款兼容模式）：

```rust
pub struct Document {
    pub source: String,
    #[serde(default)] pub provenance: Option<Provenance>,
    /// Document-level metadata (PDF Info / OOXML core.xml / HTML <meta>),
    /// when the container carries any. Absent → `None` (skipped in JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
    pub pages: Vec<Page>,
}

pub struct Metadata {           // 全字段 Option，取到多少填多少
    pub title: Option<String>, pub author: Option<String>,
    pub subject: Option<String>, pub keywords: Option<String>,
    pub creator: Option<String>, pub producer: Option<String>,
    /// 归一化为 ISO 8601（PDF "D:20260906…" → "2026-09-06T…"）
    pub created: Option<String>, pub modified: Option<String>,
    pub language: Option<String>,
}
```

- **契约影响（诚实标注）**：有元数据的文档 `-f json` 输出会新增 `"metadata"` 字段 → **`SCHEMA_VERSION` 0.8.0 → 0.9.0**，[schemas/document.json](../../schemas/document.json) golden 重生成，§回归三件套的 `-f json` 字节**预期变化一次**（演进，非回归）；`md`/`text`/`chunks`/`outline` 渲染不含 metadata，**字节必须不变**；无元数据文档（csv/srt…）`json` 也因 `skip_serializing_if` 字节不变。
- 输出 `-f meta`：`Format` 枚举加 `Meta`，渲染为投影结构 `MetaReport { source, parser, page_count, metadata }`（core 新模块 [core/src/meta.rs](../../crates/docparse-core/src/meta.rs)，derive `Serialize + JsonSchema`，schema 面加 [schemas/meta.json](../../schemas/meta.json)，与 outline/quality 同款）。`-f meta` 只到 stdout/`-o`，不进批量落盘命名以外的特殊路径（batch 自动得 `<stem>.meta.json` 后缀，随既有机制）。

**后端填充范围**（v1 五个有真实元数据源的后端，其余诚实 `None`）：

| 后端 | 来源 | 落点 |
|---|---|---|
| pdf | trailer `/Info` 字典（lopdf 已解 COS）；PDF 日期→ISO 8601 纯函数 | [pdf/lib.rs](../../crates/docparse-pdf/src/lib.rs)（新 `metadata.rs` 小模块） |
| docx | `docProps/core.xml`（zip + quick-xml 直读，不依赖 docx-rs 暴露面） | [docx/lib.rs](../../crates/docparse-docx/src/lib.rs) |
| pptx | 同上 `docProps/core.xml` | [pptx/lib.rs](../../crates/docparse-pptx/src/lib.rs) |
| xlsx | 同上 | [xlsx/lib.rs](../../crates/docparse-xlsx/src/lib.rs) |
| html | `<title>`、`meta[name=author|description|keywords]`、`lang` 属性 | [html/lib.rs](../../crates/docparse-html/src/lib.rs) |

加密 PDF：解密后同路径读 Info，无特殊处理。日期转换失败 → 该字段 `None`（不静默造数据）。

### 3.3 P3 `locate` 子命令：把招牌能力带到 CLI

**现状**：[chunk.rs:461](../../crates/docparse-core/src/chunk.rs) `locate(chunks, page, x, y)` 已实现，仅 MCP 暴露。

**决策**：
- 形态：`docparse locate <file> --page N --x X --y Y [--top-left] [-f json|text] [--password …]`（子命令自带 flags——与 `mcp`/`serve` 先例一致，顶层 flag 在子命令后 clap 不可见）。
- 坐标：默认 **PDF 用户空间**（左下原点、y 向上，IR 原生约定）；`--top-left` 换算 `y = page.height - y`（截图/标注工具给的坐标是左上原点——真实高频痛点，一次换算的事）。页码超界 / 文件无该页 → 报错；坐标超页面范围 → 仍执行查询（bbox 判定自然 miss → `null`）。
- 输出：命中 → 该 chunk 的 JSON（与 `get_chunks` 元素同构：text/bbox/page/heading_path/section_id/kind），退出码 0；未命中 → `null`，退出码 **0**（未命中是查询结果而非错误，与 MCP locate 语义对齐）。`-f text` 打命中 chunk 文本，未命中输出空。
- 内部路径：`parse_path_with` → `chunk()` → `locate()`，全确定性；v1 无模型 flag。

**落点**：[cli/main.rs](../../crates/docparse-cli/src/main.rs) `Command::Locate`（约 L339 枚举）+ 一个 `run_locate()` 函数（复用 `render_doc` 不合适——输出是单 chunk，独立小函数）。

### 3.4 P4 stdin 管道 `-`

**现状**：`inputs` 只收路径；管道场景必须先手动落盘。

**决策**：
- inputs 中出现 `-`（单独一个，且不得与 folder/多输入/`--out-dir` 组合——报清晰错误）→ 读 stdin 全量入内存（与"解析本就全量入内存"一致）→ 写临时文件 `docparse-stdin-<pid>-<nanos>.<ext>` → 复用 `parse_path_with` → 结束 best-effort 删除。
- 格式判定：`%PDF-` 前 5 字节 → pdf；否则**必须** `--input-format <fmt>`（新 clap 参数，值 = 解析器名 pdf/docx/html/xlsx/pptx/md/csv/srt/tex/eml/img/adoc），未给 → 报错并列出全部合法值。`--input-format` 同时作为 URL 场景的兜底 override（§3.5）。
- 单文件路径本就不走 `--cache-dir`（[main.rs:1174](../../crates/docparse-cli/src/main.rs) 已有 note），stdin 与 cache 天然无交集，零处理。

**落点**：[cli/main.rs](../../crates/docparse-cli/src/main.rs) main() 单文件判定前插入输入预处理（URL/stdin 统一在这一步物化成本地临时路径，后续管线零感知）。

### 3.5 P5 URL 直接输入

**现状**：必须手动 curl 下载。

**决策**：
- inputs 以 `http://` / `https://` 开头 → `ureq`（workspace 已有，cli 的 Cargo.toml 加 `ureq.workspace = true`，零新供应链面）下载到临时文件 → 单文件路径。仅限**单个输入**（与 batch 组合拒绝，明确性优先）。
- 扩展名推断：URL path 末段后缀 → 无后缀时看 `Content-Type`（application/pdf→pdf、text/html→html）→ 仍无法判定且给了 `--input-format` → 用之 → 否则报错。
- 超时 30s（connect+read）、跟随重定向（ureq 默认 ≤5）、失败错误含 URL 与状态码；下载阶段走 `reporter.spinner("download")`（stderr，不污染 stdout）。
- 临时文件清理同 §3.4（同一 helper）。

**落点**：与 §3.4 同一个输入预处理模块（建议新 [cli/input_source.rs](../../crates/docparse-cli/src/input_source.rs)：`resolve_input(path) -> ResolvedInput{ Local, Stdin, Url(temp) }`，单测覆盖扩展名/Content-Type 推断纯函数）。

## 4. 用户使用例子（验收即跑）

```bash
# P1 页范围：引用仍是绝对页码
docparse big.pdf --pages 30-40 -f chunks | jq '.[0].page'        # ≥ 30
docparse big.pdf --pages 999 -f chunks                            # 报错：页码越界
# P2 元数据
docparse 1901.03003.pdf -f meta | jq '.metadata.title'
docparse scan.pdf -f json | jq '.metadata'                        # 无 Info → null/缺省，不造数据
# P3 locate（左上原点换算）
docparse locate 1901.03003.pdf --page 1 --x 300 --y 200 --top-left
# P4 管道
curl -sL https://arxiv.org/pdf/1901.03003 | docparse - -f chunks --chunk-target-chars 400
cat notes.md | docparse - --input-format md -f text
# P5 URL
docparse https://arxiv.org/pdf/1901.03003 -f outline
# REST / MCP 透传
curl -F "file=@big.pdf" "http://127.0.0.1:8642/parse?format=chunks&pages=1-3"
# MCP: get_chunks { path, pages: "1-3" }
```

## 5. 契约变更清单

| 变更 | 面 | 动作 |
|---|---|---|
| `Document.metadata` 字段 + `Metadata` 类型 | IR / `-f json` | `SCHEMA_VERSION` 0.8.0→0.9.0；`schema --write` 重生成 document.json |
| `MetaReport` + `schemas/meta.json` | `-f meta` / `schema` 子命令 / REST `/schema/meta` | core derive + schema.rs 注册 + golden |
| `--pages` / `--input-format` / `Command::Locate` / `Format::Meta` | CLI | clap 定义 + `output_signature` 加 `pages` |
| REST `?pages=`、`/schema/meta` | REST | server.rs 参数与路由 |
| MCP `pages` 参数（4 工具） | MCP | mcp.rs inputSchema 更新 |
| clients/python·typescript 类型 | clients | `pages`/meta 字段薄客户端透传（可选，最后做） |

## 6. 测试用例

### TC-101 · `--pages` 解析与过滤
- P0 / unit+e2e / `core/pages.rs` 单测 + cli 集成
- 前置：多页样例（`1901.03003.pdf`）
- 用例：`"3"`→单页；`"1-3,7"`→混合；`"5-"`→开放端；`"0"`/`"3-1"`/`"abc"`/`"999"`（越界）→ 全部报错且信息明确；过滤后 `Page.number` 保留绝对页码；`--pages` + `--ocr`：模型只对保留页路由（route_plan 对比验证）；`--pages` 改变 cache 命中（batch 场景签名变化）。

### TC-102 · REST/MCP 透传
- P0 / e2e / server.rs + mcp.rs 测试
- `?pages=1-3` 与 MCP `pages` 参数产物 = CLI 同输入字节一致（四接口一致性不变量）。

### TC-201 · 元数据抽取
- P0 / unit+e2e / pdf metadata 单测 + 三件套回归
- PDF Info 全字段 / 缺字段 / 坏日期 → 对应字段 None；PDF 日期→ISO 8601 纯函数表驱动单测；DOCX/PPTX/XLSX core.xml；HTML title/meta；`-f meta` 输出结构；**无 metadata 文档 `-f json` 字节不变**（skip_serializing_if）；md/text/chunks/outline 字节不变；document.json golden 更新 + meta.json 新增。

### TC-301 · locate
- P0 / e2e / cli 测试
- 命中返回含 bbox/heading_path 的 chunk JSON；未命中 `null` + 退出码 0；`--top-left` 换算正确（与默认系对照：同一物理点两系命中同一 chunk）；页码越界报错。

### TC-401 · stdin
- P0 / integration / cli 测试
- `%PDF-` 喂入 → 免参数解析成功；非 PDF 无 `--input-format` → 报错列出合法值；`--input-format md` 解析 markdown；临时文件跑完已清理；`-` 与多输入/`--out-dir` 组合报错。

### TC-501 · URL
- P1 / integration（本地起 axum 静态文件或 file:// 不做——用本地 serve fixture）/ cli 测试
- URL 下载解析成功；无扩展名 + `Content-Type: application/pdf` → 推断 pdf；连接失败/404 错误含 URL 与状态码；下载进度走 stderr、stdout 纯数据。

### 回归底线（每 Phase 收口必跑）
```bash
cargo build && cargo clippy --all-targets && cargo fmt --check && cargo test
S=../opendataloader-pdf/samples/pdf
for f in lorem 1901.03003 issue-336-conto-economico-bialetti; do
  ./target/debug/docparse $S/$f.pdf -f md      # 字节不变
  ./target/debug/docparse $S/$f.pdf -f text    # 字节不变
  ./target/debug/docparse $S/$f.pdf -f chunks  # 字节不变
  ./target/debug/docparse $S/$f.pdf -f json    # 仅预期新增 metadata（0.9.0），diff 仅此字段
done
```

## 7. 实施顺序（每 Phase 独立可交付、独立 commit 尺寸）

| Phase | 内容 | 为什么这个序 |
|---|---|---|
| 1 | `--pages`（core/pages.rs → CLI → cache → REST/MCP） | 触面最广（三面+cache），先立"过滤在增强前"的语义 |
| 2 | `-f meta`（IR 字段 → 5 后端 → 渲染 → schema） | 唯一契约变更，紧跟其后便于一起 bump 0.9.0 + 重生成 schema |
| 3 | `locate` 子命令 | 纯 CLI 皮，复用 P1-P2 稳定后的入口 |
| 4 | stdin `-`（input_source.rs 起步） | 输入物化模块先建 |
| 5 | URL 输入（复用 input_source.rs + ureq） | 最独立，最后收尾 |

每 Phase：实施（源码+测试同批）→ 回归底线 → 更新 [capabilities.md](../capabilities.md) CLI 速查表 → devlog 一篇。

## 8. 验收标准

1. `cargo build` / `clippy --all-targets` 零 warning / `cargo fmt --check` 过 / `cargo test` 全绿（新增单测 ≥ 25：pages ≥8、metadata 日期 ≥4、locate ≥3、input_source ≥6、cache 签名 ≥2、meta 渲染 ≥2）。
2. §4 全部例子手工跑通且输出符合语义。
3. 回归底线通过：md/text/chunks/outline 四格式三件套字节不变；json 仅 metadata 字段差异；`schemas/` 重生成且 golden 测试绿。
4. 四接口一致性：`--pages` / meta / locate 语义在 CLI=MCP=REST 三面对同输入字节一致（既有不变量不破）。
5. 文档同步：capabilities.md、agent-integration.md（MCP pages 参数）、README 快速开始（`--pages` 一行示例）、status.md 里程碑一条。

## 9. 风险与开放问题

- **json 回归字节变化**（预期内，见 §3.2）：若下游有逐字节快照测试需同步——已在本计划 §6 显式声明，评审时确认接受。
- `--pages` 开区间 `"10-"` 在 REST query 里的编码：`pages=10-` 合法（`-` 是 URL 安全字符），无需转义。
- stdin 大文件内存：与既有解析内存模型一致（PDF 后端本就整读），不在本计划引入流式解析（YAGNI，真需求出现再立项）。
- URL 场景 --pages 组合：天然支持（下载物化后即单文件路径），无需特判。
- 未决（评审时定）：`-f meta` 批量模式下聚合报告的 pages 列是否影响——预计无影响（report 与 format 无关），实施时验证。
