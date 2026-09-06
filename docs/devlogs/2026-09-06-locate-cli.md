# devlog · `locate` 子命令（Phase 29 · 计划 P3）

> 计划：[plans/2026-09-06-cli-interface-ergonomics.md](../plans/2026-09-06-cli-interface-ergonomics.md) §3.3。2026-09-06。

## 做了什么

`docparse locate <file> --page N --x X --y Y [--top-left] [-f json|text] [--password]`：

- 实现是纯接口活：`parse_path_with` → `chunk_document` → `core::chunk::locate`（[chunk.rs:461](../../crates/docparse-core/src/chunk.rs) 既有函数，MCP 同款），零核心改动。
- **`--top-left`**：截图/标注工具给的是左上原点坐标；`y_user = page.height - y` 一次换算。测试断言同一物理点双坐标系命中同一 chunk（792-670=122）。
- **未命中语义**：`null`（json）/ 空行（text）+ 退出码 0——miss 是查询结果而非错误，与 MCP `locate` 的 chunk-or-null 对齐；页码越界才是错误（`no page 9 (document has 3 pages)`）。
- 子命令自带 flags（顶层 flag 在子命令后 clap 不可见，`mcp`/`serve` 先例），v1 只带 `--password`，无模型 flag——增强场景的 locate 归 MCP face，CLI 定位"确定性顺手查询"（计划 §2 明确边界）。

## 冒烟发现的非 bug

手写 3 页 fixture 最初每页同一 y=720 放 "Page N"，`-f text` 输出全空——**页眉检测正确工作**（≥3 页同位置重复文本=页眉定义），单页因无法建立"重复"证据而正常。修正 fixture（逐页 y 错开 60pt）后一切正常；把原因写进 fixture 注释防后人再踩，并新增 `pages_filter_keeps_text_rendering_intact` 回归测试（--pages 产物 == 手工删同页 + 多页正文照常渲染）。（此插曲发生在 Phase 27 收口期，fixture 为 27/29 共用。）

## 验证

- 测试 2：双坐标系命中一致性（走真实 `parse_path_with` + lopdf fixture）；页越界前置条件。
- 冒烟三态：命中（text/json 形态、bbox 660-684）、`--top-left` 等价命中、页越界报错。
- workspace 372 全绿、clippy 零新增。

## 实耗时

~40min（含页眉插曲的二分定位）。
