# devlog · 质量复核闸门 --quality-threshold（2026-09-06）

一句话：**为 RAG 入库前的人工复核提供确定性闸门——无文本层或乱码率超阈值的页面以 JSON 清单输出到 stderr；干净数字页永不出现，空清单即"通过"。**

## 问题

语料入库缺"质量门"：解析（含 OCR 增强）后仍然低质量的页面（扫描未恢复、解码乱码、数字文本+扫描混排）会静默进入知识库、污染检索。`quality` 模块已有逐页评估信号（`assess_page`），但没有"阈值→动作"：既无复核清单，也无管线可消费的"通过/不通过"结论。

## 实现

- `core/quality.rs` 新增 `ReviewPage` / `ReviewList` + `review_pages(doc, garbled_threshold)` 纯函数。列出条件：**无文本层**恒列出（`no_text_layer`，与阈值无关）；**乱码率 > 阈值**列出（`garbled`）；**数字文本+像素图混排**列出（`mixed_text_and_scan`）。确定性、零模型，反映增强后状态（`--ocr` 恢复的页面不再列出）。
- CLI 新增 `--quality-threshold <FLOAT>`（`Option`，缺省不启用）：JSON 清单输出到 stderr，与 `--quality` / `--profile` / `--route-plan` 同一约定。
- 不触碰输出契约：`-f json/md/text/chunks/outline/okf` 与四接口字节不变；复核清单是可观测产物，不注册进公开 schema。

## 验收

- 4 个新单测（clean 通过 / 无文本列出 / 阈值灵敏 / 混排列出）+ workspace 全量测试绿。
- `cargo clippy --all-targets` 零新增警告（既有 warning 均为预存在代码：table_cluster / tract-core / raster / ocr / font）。
- e2e：干净 Markdown → `review_pages: []`；1×1 PNG → `no_text_layer` 列出；加/不加 flag stdout 逐字节一致（`cmp`）。

## 范围

本轮只做"阈值→复核清单"这一观测动作。**未做**：批量路径（`--report-json`）集成、退出码语义（避免破坏既有管线）、自动拒绝/告警动作。
