# devlog · `--pages` 页范围选择（Phase 27 · 计划 P1）

> 计划：[plans/2026-09-06-cli-interface-ergonomics.md](../plans/2026-09-06-cli-interface-ergonomics.md) §3.1。
> 本文记实施过程、关键取舍与验证结果。2026-09-06。

## 做了什么

五项接口层扩展的第一项：`--pages "1-5,10"` 页范围选择，CLI/REST/MCP 三面同源。

- **新模块 [core/pages.rs](../../crates/docparse-core/src/pages.rs)**（~200 行含测试）：`PageSpec` 两阶段——`parse` 只验语法（REST 在重活前 400），`resolve(total)` 对真实页数做边界校验（越界报错），`retain_pages` 过滤 Document。设计为两阶段是 REST 的现实约束：请求处理早期还不知道总页数，语法错误应尽早 400 而不是解析完整文档后才报。
- **CLI**：`Cli.pages: Option<String>`（`--pages SPEC`）；过滤点放 `parse_and_enhance` 内、`parse_path_with` 之后、一切增强之前——单文件与 batch（经同函数）自动都生效，且 OCR/版面/UniRec 只见保留页（省模型成本正是页范围的核心价值）。
- **REST**：`?pages=`。handler 入口 `PageSpec::parse` 语法校验（400），边界校验 + 过滤在 `spawn_blocking` 内 cache 回放之后（422）；`?format=okf` 的 tar 路径同样透传。
- **MCP**：`parse_document` / `get_chunks` / `outline` / `export_okf` 四工具 inputSchema 加 `pages`（共享 `pages_arg_schema()`）；过滤在共用的 `parse_enhanced` 里。
- **cache**：batch 签名仅当 `--pages` 显式给出才追加 `;pages=<spec>`——签名串变更会让全部存量缓存一次性失效，加个常数后缀不值得；serving faces 缓存的是全量增强文档，`pages` 过滤在回放后做，天然不分片。
- **文档**：capabilities.md §3、两份 README 快速开始、status.md Phase 27。

## 关键取舍（评审已定，实施无偏离）

1. **解析后过滤而非下沉后端选择性解析**：数字页 ~700 页/s，全解析代价可忽略；下沉要动 `DocumentParser` trait（12 个后端），YAGNI。
2. **绝对页码不重映射**：`Page.number`、chunk `page`、outline `page` 全保持原文档编号——"见第 37 页"的引用跨不同 `--pages` 调用依然成立。
3. **越界 = 报错**（`999`、`20-` 起点、`1-999` 末端）：静默截断会产生空输出/缺数据，违背"不静默吞数据"。唯一例外是开放端 `5-` 的**末端**截到文档最后页（这是开区间的本意）。
4. **MCP 过滤放 `parse_enhanced`**：`locate` 也经此函数但不宣告 `pages` 参数——客户端不传即无影响，实现上零特判。

## 踩坑

- MCP 工具错误不走 JSON-RPC `error`，而是 `isError: true` + `content[0].text`（`call_tool` 的既有约定）——初版测试断言 `resp["error"]["message"]` 落空，读 `call_tool` 后改为断言 isError 形状。
- lopdf `dictionary!` 的 `Count` 字段 `n.into()` 类型歧义（E0283），显式 `Object::Integer` 解决。
- 测试期望先写反过一次（`"20-"` 对 5 页文档误以为截断到空）：修正为报错语义后与"不静默"原则一致。

## 验证

- `cargo test --workspace`：**355 绿 / 0 败**（pages 相关新增：core 4、cli 单元 4、cli 渲染回归 1、mcp 1、cache 2 断言并入既有测试）。
- 真实二进制冒烟（手写 3 页 PDF）：`--pages 2 -f chunks` 输出单 chunk 且 `page: 2`（绝对页码）；`--pages 9` → `page 9 is out of range (document has 3 pages)`；`--pages 3-1` → `starts after it ends`。
- `cargo clippy --all-targets`：本改动零新增（table_cluster/pdf/ocr/raster 的存量 warning 与本改动无关）。
- `cargo fmt` 过；schema golden（`committed_schemas_are_current`）绿——pages 不触 IR，schema 无漂移。
- **受限**：`../opendataloader-pdf/samples/pdf` 回归三件套（lorem/bialetti/1901.03003）本环境无样例库未跑。缓解证据：渲染路径（output/layout/chunk）零改动、新逻辑全部是 opt-in 过滤、schema golden 绿、新增 `pages_filter_keeps_text_rendering_intact` 回归测试锁定"--pages 产物 == 手工删除同页"。**待办**：样例环境补跑 `-f md/text/chunks` 字节对比。

## 插曲：fixture 触发页眉检测（非 bug，反证了一次正确行为）

冒烟时发现手写 3 页 PDF 的 `-f text` 输出全空（json/chunks 却有全部文本），一度怀疑多页渲染存在存量 bug。二分定位（单页 vs 多页 × 手写 vs lopdf 生成）后真相：**fixture 每页在同一 y=720 放 "Page N"——≥3 页的"同位置重复文本"正是页眉的定义**，`layout.rs` 页眉/页脚检测正确把它从正文剔除；单页无法建立"重复"证据所以正常。修正 fixture（每页 y 错开）后一切正常。这个 degenerate 用例反证了页眉检测在工作，已把 fixture 的 y 错开与原因写进代码注释；`pages_filter_keeps_text_rendering_intact` 测试固化"多页正文照常渲染 + --pages 等价手工删页"两条性质。

## 实耗时

~1.5h（含读 server.rs/mcp.rs/cache.rs 落点、MCP 错误形状踩坑返工一次）。

## 下一步

计划 P2 `-f meta`（唯一契约变更项：IR 加 `metadata`，SCHEMA_VERSION → 0.9.0）→ P3 `locate` 子命令 → P4 stdin → P5 URL。顺带发现 main.rs 已有 PDF 日期→ISO 转换函数（password 相关工作引入），P2 可直接复用。
