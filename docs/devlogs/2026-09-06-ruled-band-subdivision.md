# 2026-09-06 · Phase 29 — ruled 表"标签行+变体行"堆叠修复

## 背景与缺陷

真实论文（arxiv 1706.03762）Table 3 在 Phase 26 后被检为 ruled 表，但数据严重错乱：

```
| (A) |  |  |  | 1 4 16 32 | 512 128 32 16 | 512 128 32 16 |  |  |  | 5.29 5.00 4.91 5.01 | 24.9 25.5 25.8 25.4 |  |
```

- booktabs 表中 (A) 标签行与其下 **4 个无横线分隔的变体行**被 attempt B（每 band 一行）当成**一个逻辑行**，5 个 cy 的值全部堆叠进同一 cell → RAG 检索到 "1 4 16 32" 这类垃圾。
- 同样问题波及 Table 2（每行 3-5 个模型名堆叠）与 Table 4（引用堆叠）。

## 根因

`try_ruled_region` 的 attempt B（Phase 26 引入）：每个 rule 分隔的 band 视为 1 个逻辑行。对**每行一规则的网格表**这是精确的，但 booktabs 表只在组间画 midrule——(A) 标签与其变体行共享 band，被整体收进一行。

排除项：
- `build_rows`（chunk→Row）阈值 0.5em 不背锅——变体行 pitch ≈10.9pt（>5pt），探针证实它们本就是独立 Row。
- `logical_groups`（attempt A 用）的 gap 双峰在真实 band 内部分合并（(A) 与最近变体行 gap 5.4pt 低于阈值），不彻底。

## 修复（3 处增量）

1. **attempt B band 细分**：每个 band 用 `logical_groups` 按基线 gap 双峰细分成多个逻辑行。真网格表 band 内行距均匀 → singletons，行为不变；booktabs 变体行 → 拆开。
2. **首 band（表头）保护**：细分后 Table 2/Table 3 的多层 wrapped 表头（列名换行，pitch 与数据行相同）被拆成错位行；首个 band（toprule 下第一 band，即表头）保持一行不细分。
3. **build_rows 合并阈值 0.5em → 0.35em**（保险）：更密的表内变体行（pitch <0.5em 但 ≥0.35em）不再被误并为一行；<0.35em 的真换行片段仍合并。

## 验证

- 新增单测 2 项：`dense_data_rows_stay_separate`（4 行 4pt pitch 不合并）、`booktabs_band_variant_rows_do_not_stack`（标签+4 变体行 → 5 逻辑行、cell 单值）。
- table 测试 36 passed；workspace 344 passed / 0 failed。
- clippy：docparse-cli 0 warning，其余为既有基线。
- e2e（paper.pdf → markdown）：
  - Table 2：表头恢复 pipe（`| Model | BLEU EN-DE EN-FR | Training Cost (FLOPs) EN-DE EN-FR |`），**数据从堆叠展开为每行独立**（ByteNet/GNMT/ConvS2S... 各一行）。
  - Table 3：(A)(B) 组变体行展开为独立行（`1 512 512 5.29 24.9` / `4 128 128 5.00 25.5` / ...），数据完整可对齐。
  - Table 4：引用行展开（每行一个引用 + 值）。
  - 回归 diff 仅限 3 张表区域，正文零改动。

## 遗留（记录）

- Table 3 主体从 pipe 化回退为文本行（细分后列数/填充率变化致 `is_tabular` 拒绝）——文本行数据完整、每行独立，对 RAG 优于堆叠 pipe，但丢失 pipe 格式。
- Table 3 表头文本行 chunk 顺序乱（"train PPL BLEU params N d d h..."）——表头 chunk 的 x 排序问题，独立于本修复，留待后续。
- (C)(D)(E) 组 + big 行仍为文本残片（band 拆分长尾，Phase 26 已声明 out of scope）。
