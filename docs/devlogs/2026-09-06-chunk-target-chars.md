# devlog · RAG 切块尺寸可调 --chunk-target-chars（2026-09-06）

一句话：**把 core 里早已存在但从未暴露的切块尺寸旋钮接到 CLI——`--chunk-target-chars <n>` 控制连续段落累积到多少字符才成块；不传时字节不变。**

## 问题

`ChunkOptions.target_chars` 自 chunk 模块起就存在（默认 800），但 CLI/MCP/REST 全部走默认值。RAG 用户想按向量库/召回场景调块粒度（密集索引要小块、上下文敏感场景要长块），只能改代码。

## 实现

- `main.rs` 新增 `--chunk-target-chars <N>`（`default_value_t = 800`），`render_doc` 的 `ChunkOptions` 改为显式传入 `target_chars`。
- 不改 core 逻辑；块原子性不变：heading / list item / code / table 永不拆分，只有**同节连续段落**按目标尺寸累积（现有 `p.char_len < target_chars` 规则）。
- MCP/REST 仍走默认 800——纯增量、默认输出逐字节一致，四接口一致声明不受影响。

## 验收

- 新单测 `target_chars_controls_paragraph_accumulation`：同页 3 段，默认 800 合并为 1 块、`target_chars=1` 拆为 3 块；workspace 全量测试绿。
- `cargo fmt --check` 通过；clippy 零新增告警（既有告警清单不变）。
- e2e：同节 3 段样例——默认 1 个 paragraph 块（635 字符）/ `--chunk-target-chars 1` 拆 3 块（177/242/212）；显式 `--chunk-target-chars 800` 与默认输出**逐字节一致**；heading 块数不变。

## 范围

本轮只暴露既有旋钮，不做跨块 overlap、不做长段落内部再切分（两者都会动输出契约，留待按需设计）。
