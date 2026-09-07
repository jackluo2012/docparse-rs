# 部署加固 · 边界收敛 · 一键本地/Docker 部署

> 计划稿（复杂需求，CLAUDE.md §0 复杂路径）。原始需求："一键完全部署脚本，不让其它用户再踩坑；支持本地和 Docker；解决两个已知边界问题；顺手解决 devlog 中记录的遗留问题。"
>
> 状态：**已实施**（2026-09-07）。范围以用户列示为准。

## 1. 需求三件套

**要解决什么**：2026-09-06 部署过程暴露的坑（v4 OCR 档上游失效、31 字节 README 误杀整仓拉取、HTTP/2 断流无续传、HF 单连接限速、端口被旧进程占用、PID 文件丢失、大模型下载耗时 1h+）不应由下一个用户再踩；同时收敛两个已声明边界（REST/MCP 无 transcribe；`--header` URL 认证缺失）与 devlog 遗留（EML 元数据、MCP meta 投影）。

**给谁用**：所有后续部署者（本地裸机 / Docker），以及经 REST/MCP 调用全量能力的 agent。

**成功长这样**：
```bash
scripts/deploy.sh daemon        # 本地：构建→装模型→起服务→健康检查，一条命令
scripts/deploy.sh docker        # Docker：构建镜像（可烘焙模型）→ 起容器 → 健康检查
curl -F "file=@scan.pdf" "…/parse?format=chunks&ocr=true&transcribe_model=true"   # 全能力
```

## 2. 范围与不做什么

**做**：
1. fetch.rs 可靠性：`download_one` 断点续传（Range/206 + .partial 追加）；`fetch_repo` 并行下载（≤3 线程，进度语义保留）；`FileSpec` 支持直连 URL，v4 档字典改指 PaddlePaddle 官方 GitHub raw（HF 上游已删该文件）。
2. 边界①：REST `?transcribe_model=true` + MCP `parse_document`/`get_chunks` 的 `transcribe_model` 参数（复用服务端已载的 UniRec+layout）。
3. 边界②：`fetch-models ocr`（v4）修复可跑通。
4. devlog 遗留：EML 元数据（Subject→title、From→author、Date→created，`core::meta::unix_to_iso` 与 OKF 的 iso8601_utc 合一）；MCP `parse_document` 支持 `format=meta`；CLI `--header <NAME: VALUE>`（可多次）透传给 URL 下载。
5. deploy.sh 加固：启动前端口占用接管（杀同工具旧进程/异工具报错）；healthz 版本一致性校验（旧进程假就绪防护——本次实际踩中）；cargo 工具链前置检查；docker 子命令。
6. Docker：现 Dockerfile 保持"无模型轻镜像 + 运行时挂载"默认，新增 `INSTALL_MODELS=1` 构建参数烘焙全量模型（开箱即用镜像）；新增 docker-compose.yml（全能力示例）；deploy.sh `docker` 一键。

**不做**：stdin 流式解析（YAGNI，计划 §9 已声明）；三件套字节回归（样例库不在本环境，维持已声明受限）；RTL/韩文 OCR（roadmap 域外）。

## 3. 关键决策

- **续传粒度**：`.partial` 已存在且服务器支持 206 → 追加；否则整写。最终完整性由模型加载器兜底（tract 载入失败即明确报错）——不在 fetch 层做哈希（上游未提供）。
- **并行度 3**：HF 对单连接限速（实测 ~5-7MB/min），3 并发 ≈ 3×；再高无益且有被封风险。`progress` 回调改为开始前一次性列出全部文件（主线程），完成后逐个报 ✓/✗——签名不变。
- **v4 字典源**：GitHub raw（`PaddlePaddle/PaddleOCR main`）是上游真源且实测可达（6623 行）；`FileSpec.url: Option<&'static str>` 直连，不引入"GitHub repo 树 API"的一般化（YAGNI，只有这一个文件需要）。
- **端口接管**：占用者若是 `docparse serve`（本工具进程）→ 自动 kill 后启动（部署脚本的合理职权）；否则报错退出（不误杀无关服务）。
- **Docker 模型**：默认不烘焙（镜像轻、既有 README 约定、模型可挂卷共享）；`--build-arg INSTALL_MODELS=1` 供"完全开箱"需求。compose 示例用挂载方式。

## 4. 测试与验收

- fetch：LFS 判定已有单测；续传/直连 URL 的纯逻辑单测（Range 头构造、FileSpec 选择）。
- REST/MCP transcribe：mcp.rs mock 参数测试 + 真实服务冒烟（`?transcribe_model=true` 200）。
- EML：真实 .eml fixture 单测（Subject/From/Date 投影）。
- --header：parse 单测 + 本地 http.server 冒烟。
- smoke.sh 增补 transcribe 项；全部通过后交付。

## 5. 实施记录

已实施（2026-09-07），与计划无范围偏离。补充两点实施细节：
1. `fetch_repo` 并行化的 `progress` 回调语义改为"开始前一次性列出全部文件"（回调签名与主线程归属不变）——并行线程不能共享 `&mut FnMut`。
2. 部署脚本防御代码自身需过 `set -e`：`grep` 无匹配 / `ss` 缺失都会隐式杀脚本，全部收口进 `port_listener_pid`（`|| true` + `command -v` 护栏）——这是当天唯一返工点。
3. EML 元数据未引入 `core::meta::unix_to_iso`（计划原列）：mail-parser 的 `DateTime::to_rfc3339` 已产出 ISO 8601，无需共享转换——YAGNI。
4. Docker 镜像构建与容器冒烟结果见 devlog 实施记录。
