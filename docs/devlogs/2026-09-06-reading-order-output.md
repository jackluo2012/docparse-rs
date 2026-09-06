# Phase 23 — Unified reading order for every output format

## 背景

chunk 层（RAG 输出）早已有"表格/图片按几何位置 splice 回阅读顺序"的逻辑
（`chunk.rs` 的 `Item` + `follows` 列内锚定 + 按 y1 升序插入），但
markdown/text 输出（`output.rs`）仍是旧的三段式：正文块 → 表格 → 图片，
表格永远排在页尾。结果是同一份 PDF 的三种消费端（RAG chunk / markdown /
纯文本）阅读顺序不一致——RAG 里表格夹在正文中间，markdown 里却在页尾，
喂给 LLM 时上下文断裂。

## 改动

把 splice 逻辑从 chunk 层上移为共享顺序，让三种输出一致：

1. **`layout.rs`**（新共享顺序层）：
   - 新增 `PageItem`（`Block` / `Table` / `Image` 三个变体 + `bbox()`），
     统一"一页里的内容单元"；`Block` 补 `#[derive(Clone)]`。
   - 新增 `page_items_with(doc, drop_bound_captions)` → `page_items`（正文用，
     保留绑定的图注）与 `page_items_chunk`（RAG 用，绑定的图注并入图片
     chunk，不重复输出为段落）。
   - splice 语义：跳空行表；图片覆盖率过滤留给消费端（RAG 保留原门，
     正文不因小图被丢）；floats 按 y1 升序；`follows` 要求水平重叠 + 顶边在
     float 之下（右栏浮动不会跳到左栏段落前面）。
   - caption 辅助（`is_caption_line` / `h_overlap` / `v_gap` /
     `block_is_caption` / `find_caption_idx`）与常量（`IMAGE_ADJ_GAP` /
     `MIN_IMAGE_COVERAGE`）从 chunk.rs 迁入，`pub(crate)` 共享。

2. **`chunk.rs`**：删除本地复制，改消费 `layout::page_items_chunk`，并额外
   取一次完整 `page_blocks` 供 caption/context 查找（caption 行已从 items
   剔除，若从 items 里过滤会查不到 caption——修过两处测试）。
   Image 分支保留覆盖率门。

3. **`output.rs`**：`PageContent` 改持 `Vec<PageItem>`；
   `to_markdown` / `to_text` 按 `PageItem` 渲染——表格用 `markdown_table`
   插到正文中间（原逻辑：所有表格排在页尾），图片按 alt/caption 渲染。
   删除 `page_tables()`（空行表过滤已在 layout 做）。

## 验证

- 单测（core 103 项全绿）：layout `page_items_tests` 4 项（无浮动保序 /
  表格按位置 splice / 图片按位置 splice / 右栏浮动不串列）；output 2 项
  （markdown 与 text 中表格均位于两段正文之间、纯文本保序）。
- workspace 全量 16 组 test 全绿；clippy 相对基线零新增（core 仅剩
  table_cluster drain_collect 等既有基线；docparse-cli 0 warning）。
- e2e 真实二进制：手写"双栏正文 + 中间 ruled 表格 + 底部段落"PDF →
  markdown 输出为 `左栏 → 右栏 → | 表格 | → 底部`（表格 line 21 夹在
  line 11 与 line 25 之间）；`-f text` 同位置；`-f chunks` 表格 chunk id
  位于正文 chunk 之间（table id 2 < bottom id 3）。

## 收益

- 三种消费端（RAG / markdown / text）阅读顺序一致，喂 LLM 不丢上下文。
- 表格、图片按几何位置回到正文中间，不再无脑沉底。
- 双栏/多栏页面的浮动不串列（水平重叠才绑定）。
