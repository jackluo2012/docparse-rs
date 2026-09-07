#!/usr/bin/env bash
# docparse REST 服务全功能冒烟：逐项打真实请求，验证 OCR / 版面 / 表 / 公式 / 元数据 /
# 页范围全部可用。VLM 两项仅在配置了 VLM_URL/VLM_MODEL 时测试。
#
# 用法：scripts/smoke.sh [BASE_URL]     # 默认 http://127.0.0.1:8642
# 依赖：curl + python3（json 断言）；样例文件现场生成，无需仓库外资源。

set -uo pipefail
BASE="${1:-http://127.0.0.1:8642}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0; failed=0

ok()   { pass=$((pass+1)); printf '  \033[32m✓\033[0m %s\n' "$1"; }
bad()  { failed=$((failed+1)); printf '  \033[31m✗\033[0m %s\n' "$1"; }
check(){ [[ $2 == "$3" ]] && ok "$1" || bad "$1（期望 [$3] 实得 [$2]）"; }

# 生成双页"数字原生"PDF（有文本层）与扫描样张 PNG（现场手绘位图，零依赖）。
python3 - "$TMP" <<'EOF'
import sys, zlib, struct
tmp = sys.argv[1]

def pdf(paths_text, path):
    n = len(paths_text)
    objs = ["1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n"]
    kids = " ".join(f"{3+i} 0 R" for i in range(n))
    objs.append(f"2 0 obj\n<< /Type /Pages /Kids [{kids}] /Count {n} >>\nendobj\n")
    cid = 3 + n
    for i in range(n):
        objs.append(f"{3+i} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
                    f"/Contents {cid+i} 0 R /Resources << /Font << /F1 {cid+n} 0 R >> >> >>\nendobj\n")
    objs.append(f"{cid+n} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n")
    for i, s in enumerate(paths_text):
        st = s.encode()
        objs.append(f"{cid+i} 0 obj\n<< /Length {len(st)} >>\nstream\n".encode() + st + b"\nendstream\nendobj\n")
    out = b"%PDF-1.4\n"; offs = []
    for o in objs:
        offs.append(len(out)); out += o if isinstance(o, bytes) else o.encode()
    x = len(out)
    out += f"xref\n0 {len(objs)+1}\n0000000000 65535 f \n".encode()
    for off in offs: out += f"{off:010d} 00000 n \n".encode()
    out += f"trailer\n<< /Size {len(objs)+1} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n".encode()
    open(path, "wb").write(out)

streams = [f"BT /F1 18 Tf 72 {720-i*60} Td (Smoke page {i+1} body text) Tj ET" for i in range(2)]
pdf(streams, f"{tmp}/digital.pdf")

F = {'D':["11110","10001","10001","10001","10001","10001","11110"],
     'O':["01110","10001","10001","10001","10001","10001","01110"],
     'C':["01110","10001","10000","10000","10000","10001","01110"],
     ' ':["00000"]*7}
text = "DOC"; SCALE = 8; M = 40
W = M*2 + len(text)*6*SCALE; H = 160
px = [[255]*W for _ in range(H)]
for ci, ch in enumerate(text):
    for gr, row in enumerate(F[ch]):
        for gc, bit in enumerate(row):
            if bit == '1':
                for dy in range(SCALE):
                    for dx in range(SCALE):
                        px[45+gr*SCALE+dy][M+ci*6*SCALE+gc*SCALE+dx] = 0
raw = b''.join(b'\x00' + bytes(r) for r in px)
def chunk(t, d):
    c = t + d
    return struct.pack('>I', len(d)) + c + struct.pack('>I', zlib.crc32(c) & 0xffffffff)
png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', W, H, 8, 0, 0, 0, 0)) \
      + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b'')
open(f"{tmp}/scan.png", "wb").write(png)
EOF

echo "== /healthz =="
hz="$(curl -fsS --max-time 5 "$BASE/healthz")" && ok "healthz: $hz" || bad "healthz 无响应"

j() { python3 -c "import json,sys;d=json.load(sys.stdin);print(eval(\"d$1\"))"; }

echo "== 确定性解析（零模型）=="
check "json 页数" "$(curl -fsS -F file=@"$TMP/digital.pdf" "$BASE/parse?format=json" | j "['pages'].__len__()" 2>/dev/null)" "2"
check "chunks 首块页码" "$(curl -fsS -F file=@"$TMP/digital.pdf" "$BASE/parse?format=chunks" | j "[0]['page']" 2>/dev/null)" "1"

echo "== --pages 页范围（REST ?pages=）=="
check "?pages=2 只剩 1 页" "$(curl -fsS -F file=@"$TMP/digital.pdf" "$BASE/parse?format=json&pages=2" | j "['pages'].__len__()" 2>/dev/null)" "1"
check "?pages=2 页码绝对" "$(curl -fsS -F file=@"$TMP/digital.pdf" "$BASE/parse?format=json&pages=2" | j "['pages'][0]['number']" 2>/dev/null)" "2"
st="$(curl -s -o /dev/null -w '%{http_code}' -F file=@"$TMP/digital.pdf" "$BASE/parse?format=json&pages=99")"
check "?pages=99 → 422/400（不静默截断）" "$([ "$st" = 422 ] || [ "$st" = 400 ] && echo bad-input)" "bad-input"

echo "== 元数据（-f meta / ?format=meta）=="
check "meta 投影" "$(curl -fsS -F file=@"$TMP/digital.pdf" "$BASE/parse?format=meta" | j "['parser']" 2>/dev/null)" "pdf"
check "数字文档无 metadata 字段（0.9.0 兼容）" "$(curl -fsS -F file=@"$TMP/digital.pdf" "$BASE/parse?format=json" | j ".get('metadata','absent')" 2>/dev/null)" "absent"

echo "== OCR（真实 PP-OCRv6 推理，扫描样张）=="
ocr_json="$(curl -fsS -F file=@"$TMP/scan.png" "$BASE/parse?format=json&ocr=true")"
check "OCR 读出 DOC 字样" "$(echo "$ocr_json" | grep -q "D0C\|DOC" && echo hit)" "hit"

echo "== 版面（DocLayout-YOLO 真实推理，数字页按页路由）=="
lay_json="$(curl -fsS -F file=@"$TMP/digital.pdf" "$BASE/parse?format=json&layout=true")"
check "layout 解析成功且页数不变" "$(echo "$lay_json" | j "['pages'].__len__()" 2>/dev/null)" "2"

echo "== UniRec 表/公式/整页转写（首请求载 ~700MB，稍慢属预期）=="
u_json="$(curl -fsS --max-time 300 -F file=@"$TMP/digital.pdf" "$BASE/parse?format=json&table_model=true&formula_model=true")"
check "UniRec 路径解析成功" "$(echo "$u_json" | j "['pages'].__len__()" 2>/dev/null)" "2"
t_json="$(curl -fsS --max-time 300 -F file=@"$TMP/digital.pdf" "$BASE/parse?format=json&transcribe_model=true")"
check "transcribe_model 路径解析成功" "$(echo "$t_json" | j "['pages'].__len__()" 2>/dev/null)" "2"

if [[ -n "${VLM_URL:-}" && -n "${VLM_MODEL:-}" ]]; then
    echo "== VLM（$VLM_MODEL）=="
    v="$(curl -fsS --max-time 120 -F file=@"$TMP/digital.pdf" "$BASE/parse?format=json&vlm_describe=true" | j "['pages'].__len__()" 2>/dev/null)"
    check "vlm_describe 解析成功" "$v" "2"
else
    echo "== VLM：未配置（VLM_URL/VLM_MODEL），跳过真实调用 =="
fi

echo
echo "结果：$pass 通过，$failed 失败"
[[ $failed -eq 0 ]]
