# Phase 25 — Table cell sanitization & pseudo-table veto

## 背景

真实论文里两类表格质量问题的实测（Attention Is All You Need）：

1. **伪表误检**：Figure 1（Transformer 架构图）的 box 矢量线段构成网格，
   被检测为一张"表格"（9 列 × 3 行、bbox 高度仅 13.4pt = 一行文字）——
   md 输出出现 `| Q | d | ×d | K | ... |`，把公式/图形标注当表格。
2. **正文泄漏进单元格**：该伪表的单元格把下方正文行
   `Where the projections are parameter matrices W ∈ R k...` 吸入——
   md 187 行 `|  | model | ... | Where the projections are... |  |`，
   RAG 检索这张"表"时读到无关正文。

## 改动

检测层三处 + 表级后处理：

1. **bordered 单元格收集**（`detect_tables`）：除"中心落入 cell"外，新增
   chunk 尺寸约束——宽度 > 1.5×cell 宽、或高度 > 2.5×cell 高则排除
   （跨 cell 的正文/图注行中心恰好落进网格时不再被吸入）。
2. **ruled 区域收集**（`detect_ruled_tables`）：center 落入 + chunk 不得
   伸出 band 左右边界（宽正文行同样被排除）。
3. **表级 sanitizer**（`sanitize_tables`，四路检测汇合后统一应用）：
   - 高度 veto：bbox 高度 < 18pt（不足两行文字）→ 判为公式/图形框误检，
     整表丢弃；
   - cell 清洗：若表内 ≥60% 的 cell 是短文本（≤8 字符）而某个 cell 超过
     40 字符，则该长 cell 文本清空（正文泄漏防御；真正的长文本表
     ——如 BLEU 对比表——大部分 cell 长，不受影响）。

## 验证

- 单测 +6（core 113 项全绿）：bordered 溢出 chunk 排除（宽正文/高标签不
  进 cell）、近满宽 cell 文本保留；sanitize 四态（单行伪表丢弃 / 真实表
  保留 / 长正文泄漏清洗 / 长文本表不动）。
- workspace 16 组全绿；clippy 零新增。
- e2e 真实论文：误检的架构图表消失（table chunk 5 → 4，pipe 行 27 → 23），
  `Where the projections...` 从 pipe 行变回普通正文；真实表 Table 2/3/4/5
  全部保留、内容不变。
