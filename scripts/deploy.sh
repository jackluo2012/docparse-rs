#!/usr/bin/env bash
# docparse-rs 全功能部署脚本：构建 → 模型下载 → REST 服务（OCR/版面/表/公式/VLM 全开）
#
# 用法：
#   scripts/deploy.sh daemon    # 后台启动（默认）：构建 + 拉模型 + 起服务 + 健康检查
#   scripts/deploy.sh start     # 前台启动（Ctrl-C 停）
#   scripts/deploy.sh stop      # 停止后台服务
#   scripts/deploy.sh status    # 服务状态 + 模型清单
#   scripts/deploy.sh logs      # 跟随后台日志
#   scripts/deploy.sh restart   # 重启
#
# 环境变量（均可覆盖，括号内为默认值）：
#   PORT          监听端口 (8642)
#   HOST          绑定地址 (127.0.0.1)。0.0.0.0 = 对外开放——服务无鉴权，只应在可信网段
#   MODELS_DIR    模型根目录 (./models)
#   CACHE_DIR     解析缓存目录 (.cache)；置空禁用
#   PPV2          1 = 另装 PP-DocLayoutV2 版面后端 (~210MB，替代 YOLO 用) (0)
#   SKIP_BUILD    1 = 跳过 cargo build --release (0)
#   FORCE_MODELS  1 = 模型已存在也重新校验下载 (0)
#   VLM_URL / VLM_MODEL / VLM_API_KEY
#                 OpenAI 兼容视觉服务（图说/表重抽）。模型本体不随本项目分发，
#                 需自备 vLLM / LM Studio / 云端服务；不设则服务启动时不带 VLM 能力，
#                 请求 ?vlm_* 会得到明确错误。
#
# REST 面（启动后即可调用，全部按页路由、数字页零模型）：
#   curl -F "file=@doc.pdf" "http://HOST:PORT/parse?format=chunks"
#   curl -F "file=@scan.pdf" "http://HOST:PORT/parse?format=chunks&ocr=true"
#   curl -F "file=@hard.pdf" "http://HOST:PORT/parse?format=markdown&layout=true"
#   curl -F "file=@doc.pdf"  "http://HOST:PORT/parse?format=chunks&table_model=true&formula_model=true"
#   curl -F "file=@doc.pdf" "http://HOST:PORT/parse?format=chunks&pages=1-5&envelope=true"
#   [VLM 配置后] &vlm_describe=true / &vlm_tables=true
# 说明：`?transcribe_model=true` 属 CLI 专属档（--transcribe-model），REST/MCP 面不含。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/release/docparse"
PORT="${PORT:-8642}"
HOST="${HOST:-127.0.0.1}"
MODELS_DIR="${MODELS_DIR:-$ROOT/models}"
CACHE_DIR="${CACHE_DIR:-$ROOT/.cache}"
RUN_DIR="$ROOT/.run"
PID_FILE="$RUN_DIR/docparse-serve.pid"
LOG_FILE="$RUN_DIR/docparse-serve.log"

cd "$ROOT"
mkdir -p "$RUN_DIR"

log()  { printf '\033[1;34m[deploy]\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31m[deploy]\033[0m %s\n' "$*" >&2; exit 1; }

build() {
    [[ "${SKIP_BUILD:-0}" == "1" ]] && { log "SKIP_BUILD=1，跳过构建"; return; }
    log "cargo build --release（lto=thin；首次约 3-6 分钟）…"
    cargo build --release
    [[ -x "$BIN" ]] || fail "构建后仍无 $BIN"
}

install_models() {
    local tiers=()
    # OCR：服务默认档是 ppocr-v6（~7MB）。注意不要用 `ocr` 档——那是 v4 回退
    # 档（models/ppocr），其上游 SWHL/RapidOCR 的 v4 字典文件已挪位、拉取会失败。
    [[ "${FORCE_MODELS:-0}" == "1" || ! -d "$MODELS_DIR/ppocr-v6" ]] && tiers+=("ppocr-v6")
    [[ "${FORCE_MODELS:-0}" == "1" || ! -f "$MODELS_DIR/layout/doclayout_yolo.onnx" ]] && tiers+=("layout")
    [[ "${FORCE_MODELS:-0}" == "1" || ! -d "$MODELS_DIR/unirec" ]] && tiers+=("unirec")
    [[ "${PPV2:-0}" == "1" && ( "${FORCE_MODELS:-0}" == "1" || ! -d "$MODELS_DIR/layout-ppv2" ) ]] && tiers+=("ppv2")

    if [[ ${#tiers[@]} -eq 0 ]]; then
        log "模型齐备（$MODELS_DIR），跳过下载（FORCE_MODELS=1 可强制重装）"
        return
    fi
    log "下载模型档：${tiers[*]} → $MODELS_DIR（unirec ~700MB，走 HF，重试内建）…"
    for tier in "${tiers[@]}"; do
        "$BIN" fetch-models "$tier" --dir "$MODELS_DIR"
    done
}

server_args() {
    local args=(serve --host "$HOST" --port "$PORT"
        --ocr-models "$MODELS_DIR/ppocr-v6"
        --layout-model "$MODELS_DIR/layout/doclayout_yolo.onnx"
        --unirec-models "$MODELS_DIR/unirec")
    [[ -n "${CACHE_DIR}" ]] && args+=(--cache-dir "$CACHE_DIR")
    if [[ -n "${VLM_URL:-}" || -n "${VLM_MODEL:-}" ]]; then
        [[ -z "${VLM_URL:-}" || -z "${VLM_MODEL:-}" ]] && fail "VLM_URL 与 VLM_MODEL 必须同时提供"
        args+=(--vlm-url "$VLM_URL" --vlm-model "$VLM_MODEL")
        [[ -n "${VLM_API_KEY:-}" ]] && args+=(--vlm-api-key "$VLM_API_KEY")
    fi
    printf '%s\n' "${args[@]}"
}

healthz() {  # OK 时输出服务信息；服务未起返回非 0
    curl -fsS --max-time 3 "http://127.0.0.1:${PORT}/healthz" 2>/dev/null
}

wait_healthy() {
    log "等待健康检查（惰性加载：此刻仅占 ~30MB，OCR/UniRec 模型首请求才载入）…"
    for _ in $(seq 1 30); do
        if info="$(healthz)"; then
            log "服务就绪：$info"
            return 0
        fi
        sleep 1
    done
    fail "30s 内未通过 /healthz —— 看 $LOG_FILE"
}

print_usage() {
    local base="http://${HOST}:${PORT}"
    cat <<EOF

════════════════════════════════════════════════════════════
 docparse REST 服务已就绪：$base
════════════════════════════════════════════════════════════
  确定性解析（零模型）：
    curl -F "file=@doc.pdf" "$base/parse?format=chunks"
  OCR（扫描件；数字页自动直通）：
    curl -F "file=@scan.pdf" "$base/parse?format=chunks&ocr=true"
  版面重排（DocLayout-YOLO，按需渲染难页）：
    curl -F "file=@hard.pdf" "$base/parse?format=markdown&layout=true"
  表结构 / 公式→LaTeX（UniRec-0.1B，首请求载 ~700MB 后常驻）：
    curl -F "file=@doc.pdf" "$base/parse?format=chunks&table_model=true&formula_model=true"
  页范围 / 质量信封 / 元数据：
    curl -F "file=@big.pdf" "$base/parse?format=chunks&pages=1-5&envelope=true"
    curl -F "file=@doc.pdf" "$base/parse?format=meta"
  机器可读契约：
    curl "$base/openapi.json"   |  curl "$base/schema/chunk"
$( [[ -n "${VLM_URL:-}" ]] && echo "  VLM（已配置 $VLM_MODEL）：…&vlm_describe=true / &vlm_tables=true" \
   || echo "  VLM：未配置（设 VLM_URL/VLM_MODEL 后重启即启用图说与 VLM 表重抽）" )
  MCP（agent 直连，stdio）：$BIN mcp --ocr-models … 同款参数
════════════════════════════════════════════════════════════
EOF
}

do_start_fg() {
    build
    install_models
    log "前台启动：$BIN $(server_args | tr '\n' ' ')"
    exec "$BIN" $(server_args)
}

do_daemon() {
    if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
        log "已在运行（pid $(cat "$PID_FILE")）；restart 可重启，status 看状态"
        healthz || true
        return 0
    fi
    build
    install_models
    log "后台启动 → 日志 $LOG_FILE"
    # shellcheck disable=SC2046
    nohup "$BIN" $(server_args) >>"$LOG_FILE" 2>&1 &
    echo $! > "$PID_FILE"
    wait_healthy
    print_usage
}

do_stop() {
    if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
        local pid; pid="$(cat "$PID_FILE")"
        log "停止 pid $pid …"
        kill "$pid"
        for _ in $(seq 1 10); do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
        kill -0 "$pid" 2>/dev/null && kill -9 "$pid"
        log "已停止"
    else
        log "未在运行"
    fi
    rm -f "$PID_FILE"
}

do_status() {
    if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
        log "运行中：pid $(cat "$PID_FILE")，$(healthz || echo 'healthz 无响应')"
    else
        log "未运行"
    fi
    log "模型目录 $MODELS_DIR："
    du -sh "$MODELS_DIR"/* 2>/dev/null || log "  （空）"
}

case "${1:-daemon}" in
    daemon)  do_daemon ;;
    start)   do_start_fg ;;
    stop)    do_stop ;;
    restart) do_stop; do_daemon ;;
    status)  do_status ;;
    logs)    tail -n 50 -f "$LOG_FILE" ;;
    *)       fail "用法：deploy.sh [daemon|start|stop|restart|status|logs]" ;;
esac
