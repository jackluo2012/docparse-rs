# AGENTS.md

## 执行环境约定（必须遵守）

本仓库位于 WSL 2（Ubuntu）内：`~/data/docparse-rs`，Windows 侧等价路径为 `\\wsl.localhost\Ubuntu\home\jackluo\data\docparse-rs`。

1. **本项目内所有 Shell 命令必须通过 `wsl` 前缀送进 Ubuntu 执行**，不得用 Windows 原生命令（PowerShell / cmd）处理本项目的构建、测试、git、文件操作等任何事项。
   - 标准格式：`wsl bash -lc '<命令>'`（默认发行版 Ubuntu，用户 jackluo）
   - 示例：`wsl bash -lc 'cd ~/data/docparse-rs && cargo test'`
2. `wsl` 命令默认在 Windows 当前目录启动，会自动映射到 `/mnt/c/...`；操作本仓库前先 `cd ~/data/docparse-rs`（或其子目录）。
3. 读写项目文件使用 UNC 路径 `\\wsl.localhost\Ubuntu\home\jackluo\data\docparse-rs\...`（Read / Write / Edit / Glob 均支持）。
4. WSL 内访问 Windows 文件用 `/mnt/c/...`。
