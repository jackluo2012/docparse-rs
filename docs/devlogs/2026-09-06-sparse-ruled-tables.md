# Phase 26 — Sparse ruled tables: relax attempt-B fill gate

## 背景

Attention 论文的 Table 3（Variations on the Transformer architecture，
跨 p8–p9）主体此前**完全不进表格**：md 里表头、base、(A)、(B) 组都是普通
文本，只有 p9 尾部 4 行被检成一张独立的 4 列小表。RAG 检索"模型变体对比"
时读不到表头与分组语义。

## 探查（临时 eprintln，已移除）

ruled 检测其实**命中了**候选：p9 规则组（9 条宽横线、y 408–662、x 108–509）
拆出 bands=8、ncols=7，但 `is_tabular` 拒绝；bisect 后 y 557–662 子组
ncols=13 也被拒。

**根因**：`is_tabular` 的逐列填充率门槛——任一列填充率 ≤60% 即拒绝。
Table 3 是 booktabs 稀疏表：13 列、(A)(B)(C) 组每行只填部分列（dk/dv/dff
等列大片空 cell），最低列填充率 <60%，两路 attempt 全被拒。

## 修复

`is_tabular` 填充率门槛按证据强度区分：

- **attempt B（cells_wrap=true，band 即行）**：有 ≥3 条宽横线且 bands≥3 的
  强 ruled 证据，逐列填充率放宽到 **40%**（`filled*10 <= nrows*4` 拒绝）。
- **attempt A（gap 推断）**：证据弱，保持 **60%** 不变。

风险面小：ruled 候选必须 ≥2 条 ≥80pt 宽横线、≥3 行 band，正文/图形框无法
轻易凑齐；放宽只影响本就命中候选的稀疏表。

## 验证

- 单测 +3（core table 33 项）：确定性 50% 填充的 8×13 稀疏表 attempt B
  接受、attempt A 拒绝；稠密表两路接受。
- e2e 真实论文：Table 3 表头 + base + (A) + (B) 完整 pipe 化（13 列，
  `| (A) | ... |` 行出现）；ruled OK 仅新增 p9 Table 3（p8=Table 2、
  p10=Table 5 原样），无任何新误检。
- workspace 全绿；clippy 零新增。

## 遗留

(C) 组（p9 顶部 4 行）与 Table 4 的 3 行仍在独立小表/文本形态——它们是
Table 3 跨页后的另一检测区域，填充率已不是瓶颈，根因在 band 拆分与区域
划分，留待后续。
