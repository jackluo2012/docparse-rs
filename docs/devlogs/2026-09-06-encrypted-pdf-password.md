# devlog · 加密 PDF 支持 --password（2026-09-06）

一句话：**`--password <pw>` 解锁加密 PDF——无密码加载报明确错误（此前会静默解析出乱码对象），密码错误报 "invalid password"；解密后输出与明文版本逐字节一致。**

## 问题

带用户密码的 PDF 此前无法使用：lopdf 在**无密码**加载加密 PDF 时不会报错，而是返回对象仍为密文的文档——调用方会静默解析出乱码（比报错更糟）。密码错误则报 terse 的 "invalid password"。

## 实现

- `docparse-pdf`：`PdfParser` 新增 `password: Option<String>`；`load_tolerant(bytes, password)` 用 `LoadOptions::with_password` 加载，并新增 `require_decrypted` 检查——lopdf 解密成功会从 trailer 移除 `/Encrypt`，因此加载后 trailer 仍含 `/Encrypt` = "需要密码但没给"，报 `PDF is encrypted — provide a password with --password`。修复路径同样携带密码。
- `explain_load_error`：`Error::InvalidPassword` → `invalid password for encrypted PDF — check --password`；其余错误原样透传。
- CLI：`--password <PASSWORD>`；MCP/REST 本轮回 `None`（保持纯增量，四接口默认字节不变）。

## 算法覆盖（lopdf 标准安全处理器）

RC4 40/128-bit（R2-R4）、AES-128（V4/CF）、AES-256（V5/R5/R6）；空用户密码文档无需传参即可加载。

## 验收

- 4 个单测：无密码加密文档被拒（消息含 `--password`）/ 已解密文档原样通过 / InvalidPassword 映射 / 非密码错误透传。
- e2e（自写 R2/RC4-40 加密样例生成器，纯 stdlib）：① 明文可解析；② 加密无密码 → 明确错误；③ 密码错误 → invalid password；④ 正确密码 → 与明文输出**逐字节一致**（仅 source 路径字段不同）；⑤ 空用户密码文档无参加载且内容一致；⑥ `--help` 列出 flag。workspace 全量测试绿；clippy 零新增；fmt 通过。

## 范围

CLI 单点接入。**未做**：REST/MCP 的 password 参数（按需接线，函数签名已预留）、权限位/所有者口令区分（lopdf 统一按用户口令处理）。
