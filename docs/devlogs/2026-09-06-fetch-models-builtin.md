# devlog · fetch-models 纯 Rust 化（2026-09-06）

一句话：**`docparse fetch-models <tier>` 内建模型下载，去掉 HuggingFace CLI / Python / shell 依赖——HF tree API 列目录 + glob 匹配 + resolve 直链，全 Rust。**

## 问题

模型获取此前依赖 `scripts/fetch-models.sh` + `hf`（huggingface_hub）CLI：没有 pip 就没有模型，且 shell 下载器在各平台行为不一致。docparse-rs 的卖点是"单二进制、纯 Rust"，这条 shell 尾巴是最后一块不纯。

## 实现

- `docparse_ocr::fetch` 重写为统一机制：`Tier` 枚举（Ocr/Ppv6/Layout/Unirec/Ppv2）携带 repo + glob + 目标文件名；`fetch_tier` 走 **tree API**（`/api/models/{repo}/tree/main?recursive=true`）列出仓库全部文件 → 小型 glob 匹配器选中（排序取首个，确定性）→ `resolve/main` 流式下载 → 临时文件 + 原子 rename（中断不留半截模型），CDN 抖动重试 3 次（沿用既有 `download_one`）。
- `Tier::Unirec` 整仓下载（保留仓库相对路径，loader 按原名找文件）。
- `fetch_ppocr_v6`（首次 OCR 提示下载入口）改为 `fetch_tier(Ppv6)` 薄封装——**单一下载路径**，不再有硬编码 URL 与 glob 两套机制。
- CLI：`fetch-models <tier> [--dir DIR]` 子命令（tier 名与脚本一致：`ppocr-v6`），banner/进度/结果全走 stderr；`ppv2` 下载后打印 onnxsim 静态化指引（该步骤仍需 Python，一次性，保持文档说明）。`ensure_ocr_models` 提示语指向内建命令；`scripts/fetch-models.sh` 降级为薄封装（`exec docparse fetch-models "$@"`）。

## 关键设计

- **glob 匹配活仓库**：旧脚本用 glob 是因为 `hf download --include` 需要；新实现同样按 glob 匹配实时目录，仓库重组不破坏（比硬编码 resolve 路径健壮）。
- **确定性与安全**：匹配结果排序取首个（`layout` 的 `**/*.onnx` 可能命中多个文件）；下载走临时文件 + 同目录原子 rename + 最小体积校验。

## 验收

- 3 单测：glob 匹配（`**/X`、`**/*.onnx`、大小写敏感）×2 + tier 规格自检（repo 合法 / dest 为裸文件名 / 无重复 dest）。
- e2e（真实网络）：`docparse fetch-models ppocr-v6 --dir /tmp/fm-e2e` → 4 文件 ~6.8MB 落盘（det 1.8M / rec 4.5M / yml 55K / cls 585K），tree API → glob → 下载 → 安装全链路通过；`--help` 列出 5 tier + all。
- workspace 全量测试绿；clippy 零新增；fmt 通过。

## 范围

CLI + 库层统一。**未做**：多文件并行下载（顺序下载，简单可预期）；断点续传（重试即覆盖）；下载进度条（stderr 逐文件行，沿用脚本风格）。
