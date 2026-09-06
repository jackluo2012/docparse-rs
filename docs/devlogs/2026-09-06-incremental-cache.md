# devlog · 增量解析缓存 --cache-dir（2026-09-06）

一句话：**`--cache-dir <DIR>` 让 RAG 语料批量重跑跳过未变更文件——按 (路径, 内容 SHA-256, 输出签名) 键控，命中直接回放已渲染输出，OCR/版面/UniRec 等重活全跳过。**

## 问题

批量构建 RAG 语料的典型节奏是反复重跑同一个文件夹：文件很少变，但每次重跑都把 OCR/版面/表格/转写推理再付一遍——重活全浪费在没变的文件上。

## 实现

- `cache.rs`（docparse-cli）：`CacheEntry`（v/pages/sha256/output）+ 三个纯函数：
  - `content_sha`：流式 SHA-256（64KB 缓冲），内容而非 mtime/size 做源身份——touch 不能冒充"未变"，内容变了绝不能回放旧解析；
  - `output_signature`：所有影响输出字节的 CLI 选项的规范化字符串（format/table_format/chunk_target/password/ocr+models/layout+model/vlm×4/table/formula/transcribe/image_embed+dir），**stderr 专用观测旗标（--quality/--profile/--route-plan/--progress/--stats）与批量机制（--out-dir/--jobs/--report-*）刻意排除**；代码注释注明"新增影响输出的旗标必须同步加进签名"；
  - `lookup`/`store`：文件名 = SHA-256(path|sha|sig)，改动即换名，无索引无失效遍历；条目内校验 v 与 sha（纵深防御）；写入走同目录 tmp + 原子 rename（--jobs 并发下 rename 原子，赢家整写、败者整弃）。
- `batch.rs`：`--cache-dir` 生效需 `--out-dir`（结果落盘才有意义）且非 `-f okf`（目录包无单一输出字节串），否则打印 note 正常运行；命中回放（pages 取自条目，报告仍准确）→ `write_rendered_output` 与新鲜解析共用同一写出路径；未命中解析后把渲染结果 best-effort 存缓存（写失败静默降级，绝不失败批次）。`FileStat` 加 `cached`：表格状态列显示 cached、JSON 报告加 `cached` 字段、CSV 加列。
- 单文件 stdout 路径：给出 note（--cache-dir 仅批量生效）。

## 关键设计

- **键即全部真相**：命中只能回放对"当前 path+内容+选项三元组"可证明正确的字节；内容变或参数变 → 新键 → 自然 miss 重解析，旧条目滞留但永不被查。
- **正确性不变量**：缓存回放与同参数无缓存批量**逐字节一致**（e2e 断言 diff -r 为空）。
- **缓存是优化不是正确性来源**：store 失败/条目损坏 → miss → 重解析，永远不失败批次。

## 验收

- 3 单测：`content_sha` 稳定且内容敏感；`output_signature` 对 6 类输出旗标敏感、对 stderr/批量机制旗标不敏感（用 clap `try_parse_from` 构造真实 Cli）；store/lookup 回环 + 三类 miss（内容变/签名变/目录缺失）。
- e2e（真实二进制）：基线（无缓存批量）→ 冷缓存全 miss → 热缓存全 hit → **缓存回放与基线 diff -r 逐字节一致** → 改一个文件只重解析它（输出已刷新）→ 换 `-f` 全 miss → 表格显示 cached 状态 → 无 --out-dir 时给 note。
- workspace 全量测试绿；clippy 零新增；fmt 通过。
- 依赖：`sha2` 已在树中（lopdf 传递依赖），直依赖零新增供应链面。

## 范围

CLI 批量路径。**未做**：`-f okf` 目录包缓存（目录非单一字节串）；缓存修剪与配额（旧键随内容/参数演变滞留，磁盘成本按文件数增长）；MCP/REST 服务端缓存（按请求参数键控，后续可按需接）。
