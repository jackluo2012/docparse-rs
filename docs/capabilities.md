# 功能与能力总览 · Capabilities

> docparse-rs 「能做什么」的单一真源。架构见 [architecture.md](architecture.md)；接入见 [agent-integration.md](agent-integration.md)；增强决策见 [agent-enhancement-decisions.md](agent-enhancement-decisions.md)；当前进度见 [status.md](status.md)。
> **代码现状永远是真源**，与代码不符以代码为准并回写。

---

## 1. 输入格式（12 种）

确定性解析，零模型。✅=支持，—=该格式无此概念/暂不抽取。

| 格式 | 扩展名 | 底层库 | 文本 | 标题 | 表格 | 列表 | 图片 | 备注 |
|---|---|---|---|---|---|---|---|---|
| **PDF** | .pdf | lopdf + 自研解释器/字体层 | ✅ | ✅ | ✅ | ✅ | ✅ | 数字+扫描混合；tagged PDF 结构树；CCITT/JBIG2/JPX 扫描解码；全部增强子系统入口 |
| **DOCX** | .docx | docx-rs | ✅ | ✅ | ✅ | ✅ | ✅ | 样式定标题；`w:numPr` 列表；`w:drawing` 内联/锚定图片 |
| **HTML** | .html/.htm/.xhtml | scraper(html5ever) | ✅ | ✅ | ✅ | ✅ | ✅ | `<img>` data: URI + 文件相对路径抽图，alt 作图说；远程 URL 不抓取 |
| **XLSX** | .xlsx | calamine | ✅ | ✅ | ✅ | — | — | 每 sheet 一页；单元格（公式取缓存值） |
| **PPTX** | .pptx | quick-xml + zip | ✅ | ✅ | ✅ | — | ✅ | 每 slide 一页；`p:pic` 经 rels 抽图 |
| **Markdown** | .md/.markdown | pulldown-cmark | ✅ | ✅ | ✅ | ✅ | — | 代码块保留缩进 |
| **CSV** | .csv | 自写 RFC4180 | ✅ | — | ✅ | — | — | 整文件一张表 |
| **SRT/VTT** | .srt/.vtt | 自写 | ✅ | — | — | — | — | 字幕→带时间戳段落 |
| **LaTeX** | .tex | 自写子集 | ✅ | ✅ | ✅ | ✅ | — | `\section` 等定层级；数学原样保留 |
| **EML** | .eml | mail-parser | ✅ | ✅ | — | — | — | RFC-5322 多部分；MIME 解码 |
| **图像** | .png/.jpg/.jpeg | zune-png/zune-jpeg | — | — | — | — | ✅ | 整图一元素；OCR 输入 |
| **AsciiDoc** | .adoc/.asciidoc | 自写子集 | ✅ | ✅ | ✅ | ✅ | — | `==` 标题、`\|===` 表 |

坐标：PDF/图像原生带坐标；其余经 [`PageBuilder`](../crates/docparse-core/src/synth.rs) 折算到 PDF 用户空间（合成布局，供排序/引用而非像素保真）。

---

## 2. 输出格式（`-f`，7 种）

| 取值 | 产物 | 关键内容 | 典型用途 |
|---|---|---|---|
| **json**（默认） | 完整 IR | Document→Page→Element(Text/Image/Table)，含 provenance、隐藏文本（审计） | 集成、审计 |
| **markdown** | Markdown | 标题/段落/列表/管道表/`![alt]()` 图片；隐藏文本与页眉页脚已滤 | 人读、Git |
| **text** | 纯文本 | 段落、`•` 列表、Tab 分隔表、图说行 | 全文检索 |
| **chunks** | JSON 数组 | 每 chunk：kind/text/page/bbox/heading_path/section_id（+ 图片 chunk 的 image{}） | **RAG 检索** |
| **outline** | 嵌套 JSON | 结构树 Section{title,level,page,bbox,children}，id 对齐 chunk.section_id | agentic 导航、目录 |
| **okf** | 目录/tar | Open Knowledge Format v0.1：一 section 一 md+YAML frontmatter，镜像结构树，可溯源、确定性 | 知识库交付（git 原生） |
| **meta** | JSON | 元数据投影：source/parser/page_count/`metadata`（PDF Info / OOXML core.xml / HTML `<meta>`，取到多少给多少，日期归一 ISO 8601 UTC） | RAG 入库元数据、归档索引 |

### chunk 的 `kind`

`heading` · `paragraph` · `table` · `code` · `list_item` · `image`

**image chunk**：`text` = 图说 ⊕ 周边上下文（可检索字段）；`image` = `{ file?, data_base64?, media_type?, caption?, caption_source? }`（渲染/引用）。详见 §4。

---

## 3. CLI 选项速查

定义见 [cli/main.rs](../crates/docparse-cli/src/main.rs) `Cli`（clap）。

**输出/输入**：`-f/--format`、`--pages <SPEC>`（页范围选择：1-based、含端点、逗号分隔，`3`/`1-5`/`10-` 三种写法可组合如 `1-5,10`；解析后、增强前过滤——OCR/版面/UniRec 只对保留页跑；页码保留绝对编号不重映射，chunk/outline 引用不受影响；显式页码越界报错不静默截断；REST `?pages=`、MCP `pages` 参数同语义，服务端缓存不分片）、`--table-format tab\|markdown`、`--chunk-target-chars <n>`（RAG 切块目标字符数，默认 800，仅 `-f chunks` 生效，缺省输出字节不变）、`--password <pw>` / `--password-env <VAR>` / `--password-file <PATH>`（加密 PDF，标准安全处理器 RC4/AES-128/AES-256；三源互斥：无密码加载加密 PDF 报明确错误、env 未设报错、file 尾部换行剥掉；REST 传 `?password=`、MCP 各工具传 `password` 参数，且密码进入服务端文档缓存签名——不同密码不回放他人条目；`serve`/`mcp` 的三源作启动默认，请求/调用缺省时回落）、`-o/--out`、位置参数 `inputs`（文件/文件夹/多输入）。**元数据**（0.9.0）：`Document.metadata` 可选字段（serde default + skip，无元数据文档 `-f json` 字节不变）；`-f meta` 出投影（schemas/meta.json）；`SCHEMA_VERSION` 0.9.0。
**批量**：`--out-dir`、`-r/--recursive`、`--jobs N`、`--report-json`、`--report-csv`、`--cache-dir <DIR>`（增量缓存：按文件内容 SHA-256 + 输出签名键控，命中直接回放已渲染输出、跳过解析；需 `--out-dir`，不适用 `-f okf`；改内容或换输出参数自然失效重解析）。
**OKF**：`--okf-resource-base <uri>`、`--okf-tar`、`--force`。
**OCR**：`--ocr`、`--ocr-models <dir>`（默认 `models/ppocr-v6`，缺则 TTY 确认下载 / `DOCPARSE_OCR_DOWNLOAD=1`）。路由按页进行：有机器可读文本的页面原样通过；缺少可用文本的 PDF 页优先使用已解码的嵌入扫描图像，否则执行该页的完整 PDF 绘制程序、按需渲染为 RGB 后 OCR。这一回退同时覆盖仅有图像位置、没有图像对象，以及用矢量路径绘制文字外观的页面。
**版面/结构**：`--layout`、`--layout-model <path>`（YOLO 默认 / PPV2 自动识别）、`--table-model <dir>`、`--formula-model <dir>`、`--transcribe-model <dir>`。REST/MCP 面同步暴露 `transcribe_model`（?transcribe_model=true / MCP 工具参数；PDF 专属）。
**输出阅读顺序**：markdown / text / chunks 三种格式共享同一几何阅读顺序——表格与图片按页内位置 splice 回正文（`layout::page_items`），不再沉到页尾；`follows` 要求水平重叠 + 顶边在浮动之下，双栏/多栏页面不会串列。RAG chunk 使用 `page_items_chunk`（绑定的图注并入图片 chunk，不重复输出）。
**续表/无表头表**：markdown/text 渲染时，无表头的数据行（空 cell 或全数值）不再被当成表头画出 `---` 分隔线——列数与上一表相同则继承上一表表头（跨页有效），否则以注释标注后输出数据行（`table::looks_like_header_row` 判定）。
**稀疏 ruled 表**：ruled 检测的"band 即行"路径（≥3 条宽横线）逐列填充率门槛为 40%（原文 60%），booktabs 式多空 cell 表（如 Attention 论文 Table 3 的 13 列变体对比表）可被检出；弱证据的 gap 推断路径仍保持 60% 严格门槛，正文/图形框不会因放宽而误检。
**表格净化**：表格检测后统一过 `table::sanitize_tables`——高度 <18pt 的候选（公式/图形框线误检）整表丢弃；表内绝大多数 cell 为短文本而个别 cell 超长（>40 字符）时清空该 cell（正文泄漏防御）。单元格/表区收集均要求 chunk 尺寸不超出 cell/band（宽 >1.5×cell 宽或高 >2.5×cell 高的正文行不再被吸入）。
**VLM**：`--vlm-describe`、`--vlm-tables`、`--vlm-url`、`--vlm-model`、`--vlm-api-key`。
**图片**：`--image-dir <dir>`、`--image-embed`。
**质量/可观测**：`--quality`、`--profile`、`--route-plan`（均出 JSON 到 stderr）、`--quality-threshold <float>`（复核闸门：无文本层页恒列出，乱码率超阈值页列出，JSON 清单到 stderr，入库前人工复核）、`--progress auto\|always\|never\|json`、`-q/--quiet`、`--stats`。
**输入物化（单输入专用）**：stdin `-`（`%PDF-` magic 嗅探免参数；否则必须 `--input-format`）、URL `http(s)://` 直接下载解析（扩展名 → Content-Type → `--input-format` 依次推断；30s 超时；仅限本地 CLI，REST/MCP 不做 URL 拉取——SSRF 边界）、`--input-format <FMT>`（pdf|docx|html|xlsx|pptx|md|csv|srt|tex|eml|img|adoc 显式格式）。两者物化为临时文件后走常规管线（含 `--pages`/增强/全部输出格式），临时文件跑完即删。
**URL 认证**：`--header <NAME: VALUE>`（可重复，如 Authorization bearer），仅作用 URL 输入；MCP `parse_document` 支持 `format=meta` 投影。
**子命令**：`locate <file> --page N --x X --y Y [--top-left] [-f json|text]`（坐标反查：命中输出该 chunk（与 `-f chunks` 元素同构），未命中 `null`/空行退出码 0——与 MCP `locate` 语义一致；默认 PDF 用户空间，`--top-left` 从左上原点换算 `y=页高-y`；确定性路径无模型 flag）、`mcp`、`serve --port`（均支持 `--cache-dir <DIR>`：服务端文档缓存，同内容+同增强参数命中直接回放增强后 Document、跳过解析；REST 加 `x-docparse-cache: hit|miss` 响应头，MCP 纯加速、输出逐字节一致）、`schema [--name N] [--write]`、`fetch-models <tier> [--dir DIR]`（纯 Rust 模型下载：ocr / ppocr-v6 / layout / unirec / ppv2 / all，HF tree API，无需 hf CLI / Python / shell）。

---

## 4. 图片 → RAG

页面占比 ≥1% 的图成为 `kind:"image"` chunk（滤图标/分隔线）；按 bbox splice 进阅读顺序；caption 行不重复成段落。

| 来源格式 | 抽图机制 |
|---|---|
| PDF | 内容流 XObject 解码（DCTDecode→Jpeg 透传；Flate→Rgb8/Gray8；扫描页保留像素） |
| DOCX | `w:drawing → a:blip r:embed` 经 `docx.images`(rId→字节) |
| PPTX | `p:pic` + slide `_rels`(rId→`ppt/media/*`) |
| HTML | `<img>` data: URI（base64 解码）/ 文件相对路径（读盘）；远程 URL 跳过 |

**caption 四档来源**（`caption_source`，优先级高→低）：

1. `vlm:<model>` —— `--vlm-describe` 神经描述（最高）
2. `layout-caption` —— 版面模型 `--layout` 的 Caption 区域 / tagged PDF 的 Caption 角色
3. `caption-line` —— 就近文档内 "Figure N / 图N / Fig. / Abbildung" 行（零模型）
4. `alt` —— HTML `<img alt>`

`--image-dir` 导出文件（`image.file`）/ `--image-embed` 嵌 base64（`image.data_base64`）；二者皆无时 chunk 仍带 caption+bbox 可检索可引用。

**ImageKind**：`None`（仅位置）/ `Gray8` / `Rgb8` / `Jpeg`（透传）/ `Encoded`（PNG/GIF/… 已编码透传）。

---

## 5. 模型能力矩阵（opt-in）

| 能力 | flag | 模型 | 需下载 | 范围 | source 标记 |
|---|---|---|---|---|---|
| OCR | `--ocr` | PP-OCRv6 tiny（v4 回退） | ~7MB 自动 | PDF（逐页质量路由；嵌入像素快路径，否则按需渲染页面外观） | `ocr:ppocr` |
| 版面重排 | `--layout` | DocLayout-YOLO / PP-DocLayoutV2 | 是 | PDF | `layout:<model>`(group/tag) |
| 表结构 | `--table-model` | UniRec-0.1B | 是 | PDF | `table:unirec-0.1b` |
| 公式→LaTeX | `--formula-model` | UniRec-0.1B | 是 | PDF | `formula:unirec-0.1b` |
| 整页转写 | `--transcribe-model` | UniRec-0.1B | 是 | PDF | `transcribe:…` |
| 图说 | `--vlm-describe` | OpenAI 兼容服务 | 否（服务） | PDF | `vlm:<model>` |
| 表重抽 | `--vlm-tables` | OpenAI 兼容服务 | 否（服务） | PDF | `vlm:<model>` |

数字页零模型；模型整批/整服务只载一次；任一模型 flag 开则批量强制串行（防爆内存）。所有增强产物带 `source` 溯源、confidence 封顶，确定性结果始终独立成立。

---

## 6. 质量 · 可观测 · 安全

**质量/路由**（[core/quality.rs](../crates/docparse-core/src/quality.rs)）：
- `--quality` → 覆盖率、乱码率、隐藏 chunk 数、flags（`ScannedNoText`/`PartialTextCoverage`/`HighGarble`/`HiddenTextPresent`/`MixedTextAndScan`）。
- `--profile` → 每页 kind(digital/scanned/mixed/empty)、text_chars、image_count/coverage、tables、enhanced_chunks、reading_order_anomaly（诊断观测，未接自动路由）。
- `--route-plan` → 哪些页因何 flag 需增强。

**可观测**：`--stats`（getrusage：峰值 RSS / CPU 时间 / util% / wall）；`--progress`（TTY 自动进度 + 结束速度小结，`json` 出事件流，全程 stderr 不污染 stdout）。

**安全/健壮**：隐藏文本过滤（标记可审计、渲染输出排除、JSON 保留）；zip-bomb 预检（OOXML 只读中央目录）；页数早停；坏页→空 Page 不 panic；批量坏文件不中断；增强失败降级。

---

## 7. 一句话能力边界

- **赢**：数字原生文档（PDF/DOCX/HTML…）——零依赖单二进制、确定、可溯源、快。
- **持平**：结构理解（born-digital 的表/列表/标题）、多格式广度。
- **外接不内化**：神经表格/公式/手写/复杂 CJK 版面——经可插拔边界外接模型，主流程不被绑定（扫描 OCR 已内嵌可打）。
