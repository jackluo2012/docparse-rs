# Phase 24 — Continued / headerless tables in markdown & text

## 背景

真实论文里经常出现"表格被拆开"的两种形态：

1. **跨页续表**：一张表跨两页，第二页只有数据行、没有表头——被检测为一张
   独立"无表头表"。
2. **碎片化检测**：大表只有中间一小块被网格检测识别（ruling 线不完整），
   该小块无表头。

此前 `markdown_table` 无条件把**第一行当表头**：`|  | 32 | 5.01 |` 这种
数据行被渲染成表头，再画一条 `| --- |` 分隔线——把数据行错标为列名，
LLM/RAG 消费时语义错误（"列名"是 `32`、`5.01`）。

实测 Attention Is All You Need 的 Table 3 续行：md 里出现
`|  | 32 | 5.01 | 25.4 | 60 |` + `| --- | --- | --- | --- | --- |`
（错误表头）。

## 改动

纯消费端（渲染层），不动检测与 IR：

1. **`table.rs`**：新增 `looks_like_header_row(&[Cell]) -> bool`——首行任一
   cell 为空、或所有非空 cell 均为数值型（数字/单位/百分号/±/小数点/×）则
   判为数据行；含真实单词才算表头。
2. **`output.rs` markdown**：
   - `markdown_table` 重构：表头行 → 原逻辑（表头 + `---` + 数据行）；
     无表头 → 只渲染数据行，**不再输出假的 `---` 表头行**。
   - `to_markdown` 维护"上一表表头"（跨页有效）：无表头表且列数与上一表
     相同 → 继承表头渲染（`<!-- continued table: header inherited ... -->`
     + 表头行 + `---` + 数据行）；列数不符 → 注释说明 + 数据行；
     无前表 → 注释 + 数据行。
3. **`output.rs` text**：同样维护表头状态——无表头表前缀 `[table without
   header]`；继承时输出 `[continued table] <表头…>`。

## 验证

- 单测 +4（core 107 项全绿）：`looks_like_header_row` 四态（空 cell / 全数字 /
  数值带符号 / 真实单词）；md 无表头表不产生 `---` 且带注释；无表头表继承
  上一表表头（md 含继承表头 + 分隔线 + 数据行，text 含 `[continued table]`）；
  列数不符不继承（表头只出现一次）。
- workspace 16 组全绿；clippy 零新增（core 仅剩既有 table_cluster 基线）。
- e2e 真实论文（Attention Is All You Need）：原 `|  | 32 | 5.01 |...` 处
  不再有假表头分隔线，改为
  `<!-- continued table without header (column count differs from previous) -->`
  + 纯数据行；有表头表（Table 1/2/4/5）渲染不变（`---` 正常）。
