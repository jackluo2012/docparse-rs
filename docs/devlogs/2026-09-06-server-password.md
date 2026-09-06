# Server / MCP Encrypted-PDF Password — Phase 21

**2026-09-06** · serve/mcp `--password` · commit 见 git log

## 目标

CLI 自 Phase 17 起支持 `--password <pw>` 解析加密 PDF（标准安全处理器
RC4 / AES-128 / AES-256）。本阶段把同一能力接通服务端两条通道，并把密码
纳入服务端文档缓存的签名，杜绝"换密码回放他人解析结果"的串味。

## 决策

- **REST**：`POST /parse?password=secret`（query 字符串）。OpenAPI 已记录该
  参数，并注明服务器定位 localhost/LAN、勿暴露给不可信网络。明文 query 是
  本服务定位下的刻意取舍（与 `x-docparse-ms` 等先例一致，body 字节不变）。
- **MCP**：五个工具（parse_document / get_chunks / outline / export_okf /
  locate）各加**可选** `password` 参数（`args.get("password")`，区别于必填的
  `str_arg`）。仅加密 PDF 需要传。
- **缓存签名**：`EnhanceState::cache_signature` 现在含 `password` 字段。
  密码是输出决定性选项——不同密码（或无密码）解析结果不同，必须进键，
  否则缓存会把 A 密码的解析结果回放给 B 密码。键 = (来源, 内容 SHA-256,
  增强参数+模型集签名, 密码)。
- **错误保持可操作**：无密码 → `PDF is encrypted — provide a password with
  --password`（REST 422 / MCP isError）；错密码 → `invalid password for
  encrypted PDF — check --password`。服务端不吞错、不降级为垃圾输出。

## 实现

- `parse_enhanced_cached(path, source_name, opts, password, state)` 新增
  `password: Option<String>` 参数；`parse_fresh` 闭包把它透传给
  `parse_path_with`（原本硬编码 `None`）。
- server.rs：handler 读 `q.get("password")`；`render` / `render_okf_tar`
  透传；OpenAPI parameters 数组加 `password`（含边界说明）。
- mcp.rs：5 个工具 schema 各加 `password` 属性；`parse_enhanced` 可选读取。
- 测试夹具：**弃用手写 RC4-40 生成器**（曾踩 R2 的 U = RC4(file_key,
  PAD_BYTES) 而非 RC4(file_key, pad(user)) 等规范细节），改用 **lopdf 官方
  加密 API**（`EncryptionVersion::V2 { key_length: 40 }` +
  `Document::encrypt` + `save_to`）生成真实加密 PDF——生成与解密同库，
  夹具永不漂移。cli 因此新增 `lopdf` 直接依赖（workspace 已有版本号，
  lock 不变），移除临时 `md-5` dev-dependency。

## 验证

- 单测（docparse-cli 49 项全过，新增 2 项）：
  - REST：无密码 422（含 encrypted）/ 错密码 422（含 password）/ 正确密码
    解析出 `Secret`，重复调用字节一致且缓存 hit；目录恰 1 条目（每
    (内容,密码) 三重键）。
  - MCP：无密码 `isError: true` / 正确密码解析出 `Secret` / 错密码
    isError。
- e2e（真实二进制，`e2e_password.sh` 全过）：
  - CLI 三态：无密码报 `--password` 提示 / `--password secret` 输出 Secret
    / 错密码报 invalid password。
  - REST 四态：无 422 / 错 422 / 对 200+miss→hit 字节一致 / 第二个加密
    文件各自密码各占 1 缓存条目（共 2 条）。
  - MCP 三态 + `--cache-dir` 跨工具（get_chunks + outline）共享单条目。
- workspace 全量测试通过；`cargo clippy -p docparse-cli --all-targets`
  零新增（唯一新增的 too_many_arguments 已对测试辅助 `render` 加 allow）。

## 未做

- 密码通过环境变量 / `--password-stdin` 注入（防命令行泄漏），留给后续
  （CLI 与服务端共用同一解析层，接入点已在 `parse_path_with`）。
