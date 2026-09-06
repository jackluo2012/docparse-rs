# Phase 22：`--password` 安全注入（env / file / 服务端默认密码）

- 日期：2026-09-06
- 范围：docparse-cli（main.rs / batch.rs / server.rs / mcp.rs）+ docparse-pdf 错误消息
- commit：见 git log（Phase 22）

## 动机

`--password <PW>` 直传会把明文口令暴露在 `ps` 进程列表、shell history 与
systemd/容器命令参数里；服务端（`serve`/`mcp`）此前只有启动时的一次性
`--password`，无法覆盖"docker secret 注入 / CI 环境变量"这类真实部署。

## 设计

### CLI 三源注入（互斥）

- `--password-env <VAR>`：从环境变量读取。**未设报错**（绝不静默降级为
  "无密码"再给一条误导性的加密错误）。
- `--password-file <PATH>`：从文件读取，**尾部单个 `\n`/`\r` 剥掉**
  （Docker secrets、`.secret` 文件的换行尾）。
- 与 `--password` 用 clap `conflicts_with_all` 互斥（编译期保证）。

```rust
pub(crate) fn resolve_password(
    explicit: Option<String>,
    env_var: Option<&str>,
    file: Option<&Path>,
) -> anyhow::Result<Option<String>> {
    if let Some(pw) = explicit {
        return Ok(Some(pw));
    }
    if let Some(var) = env_var {
        match std::env::var(var) {
            Ok(v) => return Ok(Some(v)),
            Err(_) => anyhow::bail!(
                "--password-env {var} is not set — refusing to parse without a password"
            ),
        }
    }
    if let Some(path) = file {
        let raw = std::fs::read_to_string(path).map_err(|e| {
            anyhow::anyhow!("--password-file {}: {e}", path.display())
        })?;
        let trimmed = raw.trim_end_matches(['\n', '\r']);
        return Ok(Some(trimmed.to_string()));
    }
    Ok(None)
}
```

优先级 explicit > file > env——clap 已互斥，该顺序只兜底程序化调用。

### 服务端启动默认密码

- `EnhanceState` 增 `default_password: Option<String>`（`with_default_password`）。
- `serve` / `mcp` 各自新增 `--password` / `--password-env` / `--password-file`
  三字段，启动时 resolve 一次。
- REST：`request_password(q, state)` = `?password=` 优先，否则回落
  `state.default_password`（抽成可测函数）。OpenAPI password 描述补
  "缺省回落到启动默认"一句。
- MCP：`parse_enhanced` 的 `password` 参数缺省时回落 `state.default_password`。
- 默认值同样经 `parse_enhanced_cached` → `cache_signature`：**按密码隔离缓存
  的约定对默认密码同样成立**（换默认密码绝不回放旧密码条目）。

### 错误消息

docparse-pdf 无密码 / 密码错误两条消息更新为列出三种注入方式：
`--password` / `--password-env VAR` / `--password-file PATH`。

## 验证

- 7 个新单测：
  - `resolve_password` ×5：explicit 优先、file 剥尾部换行、env 读取 +
    未设报错、file 不存在报错、无源 → None；
  - server `request_password`：缺省回落默认 / 查询覆盖默认 / 无默认无查询 → None；
  - mcp 默认密码：无 `password` 参数时按默认密码解析加密 PDF。
- e2e（真实二进制 + lopdf 生成的真实 R2/RC4-40 加密 PDF）：
  - CLI：`--password-env` 解析、env 缺失报错、`--password-file` 解析
    （剥换行）、`--password` + `--password-env` 互斥拒绝；
  - REST：`serve --password-env` 默认密码解析（无 `?password=` 200）、
    `?password=wrong` 覆盖 422、`serve --password-file`（错误默认）422；
  - MCP：`--password-file` 默认密码、无工具参数解析成功。
- workspace 全绿（docparse-cli 56 项）；`cargo clippy -p docparse-cli
  --all-targets` 零新增警告。

## 未做（诚实）

- `--password-env`/`--password-file` 在 `docparse serve`/`mcp` 之外没有
  交互式输入（保持非交互定位）。
- 无密码权限 PDF（空密码）不受影响，无需注入。
