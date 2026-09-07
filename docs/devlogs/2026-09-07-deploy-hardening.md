# devlog · 部署加固 · 边界收敛 · 一键本地/Docker（Phase 31）

> 计划：[plans/2026-09-07-deployment-hardening.md](../plans/2026-09-07-deployment-hardening.md)。2026-09-07。

## 做了什么（对照昨天的踩坑清单）

| 昨天踩的坑 | 本次根治 |
|---|---|
| `fetch-models` 整仓拉取被 31 字节 README 误杀（LFS 误判） | 上一提交已修（67e0f93）；本次再加直连 URL 兜底 |
| v4 OCR 档上游失效（SWHL/RapidOCR 删了 `ppocr_keys_v1.txt`） | `FileSpec.url` 直连 PaddlePaddle 官方 GitHub raw；4 文件全下 + v4 真实 OCR 推理闭环（识别质量确实逊 v6，符合"v6 默认"结论） |
| 554MB decoder 下到 80% 断流，从头重来 | `download_one` Range/206 断点续传（.partial 保留，重试/重跑都续） |
| HF 单连接限速 ~5-7MB/min，700MB 要 1h+ | `fetch_repo` 3 路并行（≈3×） |
| 旧 serve 进程占端口、PID 文件丢失 → 新服务 bind 失败、healthz 打到旧进程（假就绪） | `take_over_port`（本工具进程自动停/异进程报错）+ `wait_healthy` 监听 pid 一致性校验 |
| （排查中发现）`set -e` 下 `grep` 无匹配返回 1 静默杀脚本；`ss` 缺失同险 | 封装 `port_listener_pid`（`|| true` + command -v 护栏） |
| cargo 不存在时用户看到难懂的报错 | 前置工具链检查，指向 rustup |

**边界收敛**：`transcribe_model` 上 REST/MCP（EnhanceOpts/cache_signature/apply + 两工具 schema；真实推理冒烟 ✓）；MCP `parse_document` 支持 `format=meta`。

**devlog 遗留清零**：EML 元数据（Subject/From/Date 投影，恒 `Some` 与 HTML 同约定；2 单测）；CLI `--header "NAME: VALUE"` 可重复（本地鉴权 server 端到端：无 header 401、带 header 200 解析成功）。

**Docker**：Dockerfile 加 `INSTALL_MODELS=1` 烘焙（默认轻镜像约定不变）；CMD 改 shell form 展开 ENV 模型路径（exec form 不展开——首版即踩）；`exec` 保 SIGTERM；VLM_URL/VLM_MODEL 环境变量在 entrypoint 自动接线；新增 docker-compose.yml（healthcheck/缓存卷/VLM env）；`deploy.sh docker` 一键 + `DOCKER_BAKE_MODELS=1` 挂载模式。

## 踩坑

- **restart 后"服务未运行"**：新加的 `wait_healthy` 校验链上 `ss … | grep | head` 在端口无监听时 grep 返回 1，`set -e` 静默杀死脚本——症状是 daemon 无输出消失。修复用 `port_listener_pid`（`|| true`）。教训：给部署脚本加防御代码时，防御本身要先过 `set -e` 这关。
- Dockerfile CMD 的 exec form 不展开 `${ENV}`——首版写了 exec 数组形式，模型路径全是字面量；改 shell form + `exec` 前缀保留信号语义。
- 一个先行教训（诚实记录）：后台任务先于编译完成启动、跑了旧二进制，得出"修复无效"的假结论——重测前先确认二进制新鲜度。

## 验证

- 本地 daemon 重启后 `scripts/smoke.sh` **12/12**（新增 transcribe 真实推理）。
- workspace 测试全绿（EML +2、MCP +1、input_source +1）。
- v4 档：4 文件下载 ✓、字典 6623 行 ✓、`--ocr-models models/ppocr` 真实推理 ✓。
- `--header`：401/200 双态端到端 ✓。
- Docker 镜像构建 + 容器冒烟：见下方实施记录（构建含 780MB 模型烘焙，耗时较长，后台完成后补记）。

## 实耗时

~3h（含 set -e 隐蔽退出与 Dockerfile CMD 展开两处排查）。
