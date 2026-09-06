# 2026-09-06 · Phase 28 — 页码泄漏修复（page-number margin filter）

## 背景与缺陷

对 Attention 论文（arxiv 1706.03762）的真实解析输出（`-f markdown`）做全量孤立数字行扫描：

```
grep -n '^[0-9][0-9]*$' out26.md
101:2  117:3  167:4  218:5  ...  637:15   # 每页一个页码行，混入正文流
```

- 每页**页码**（"2".."15"）作为孤立数字行混进 md 流，RAG 检索会把这些无意义数字当内容命中。
- 另有 Table 3 (D)(E) 组的孤立数字（28/168/53/90/213，见下"遗留"），属表格检测缺口，非本 Phase 范围。

## 根因

`detect_header_footer` 的 running-content 检测只认 `normalize_repeat(text).len() >= 2` 的行，且靠"跨页重复 ≥3 页"计数：

1. 页码是**单字符**（"2"），被长度门槛排除；
2. 页码**逐页递增**（"2"/"3"/"4"…），归一化文本各不相同，跨页重复计数天然不成立。

两个机制叠加导致页码永远不会被识别为 running footer。

## 修复方案（v2，精确版）

第一版尝试在 `detect_header_footer` 中放行"纯数字 ≤3 位"行、让归一化 key `"#"` 跨页累积。单测立即抓出副作用：`repeated` 集合含 `"#"` 后，`is_running` 在 `page_blocks` **全页**过滤，把正文区的孤立数字（如表残片 "28"）也删了——页码 key 泄漏到整个页面。

v2 改为**位置感知**过滤，`detect_header_footer` 保持原逻辑不动：

```rust
/// A page-number line: a lone 1-3 digit text in the top/bottom 12% margin.
fn is_page_number_line(line: &Line, page: &Page) -> bool {
    let raw = line.text.trim();
    if raw.is_empty() || raw.len() > 3 || !raw.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let h = page.height.max(1.0);
    let top = h * 0.88;
    let bot = h * 0.12;
    line.cy + line.size / 2.0 >= top || line.cy - line.size / 2.0 <= bot
}
```

在 `page_blocks` 过滤：`!hf.is_running(l) && !is_page_number_line(l, page)`。

- 页码：边距区（top/bottom 12%）+ 纯数字 1-3 位 → 删。
- 正文孤立数字：页中部 → 保留。
- 页码不依赖跨页重复，单页文档同样生效（原 running 检测对 <3 页文档完全不工作）。

## 验证

- 新增单测 2 项（`page_number_margin_lines_are_dropped` / `no_footer_no_drop`），均放 `page_items_tests`（该 mod 有现成 helper）：
  - 3 页 doc，每页底边距页码 "1"/"2"/"3" + 页中部孤立数字 "28" → 页码被删、"28" 保留；
  - 无页码时 "28" 保留。
- 首版实现（repeated "#"）的单测失败证明测试有效，v2 通过。
- core layout 测试 24 passed；workspace 全绿。
- clippy：docparse-cli 0 warning；其余为既有基线（chunks_exact×5 / tract unused import / pdf font.rs / ocr lib.rs / core drain_collect）。
- e2e（paper.pdf → markdown）：`diff out26 out28` 只删 14 个页码行 + 14 个随行空行（每页 "N" + 空行），**正文零改动**，md 611 行。

## 遗留（后续 Phase）

Table 3 尾部 (D)(E) 组 + big 行在 md 中仍为孤立数字（28/168/53/90/213）——根因与 (C) 组相同：band 拆分把无横线的表尾切成错位长尾，`is_tabular` 填充率门槛无法收编。属表格检测缺口，本 Phase 刻意不碰（位置过滤会保留这些内容数字，待表格检测修复后自然归位）。
