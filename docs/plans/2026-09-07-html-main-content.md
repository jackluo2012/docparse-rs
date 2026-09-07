# docparse-html 正文抽取（站点规则 + Readability 式打分）与 `<br>` 段落重建

> 计划稿（复杂需求：跨 crate 契约面不变但解析行为变化，且有消费方 shortmind-os 协同）。原始需求："网页/公众号清洗不干净——市面标杆（Readability/trafilatura/Jina）怎么做的，按建议全部修改；docparse-rs 直接改这边，shortmind-os 配合。"
>
> 状态：**已实施**（2026-09-07，同日闭环；实施记录见 §6，shortmind 侧见其仓库 devlog）。

## 1. 需求三件套

**要解决什么**：HTML 后端是"结构遍历"（只删 script/style/head/noscript/title/svg），没有正文区域识别——公众号模板头（"点击上方蓝色文字关注我们"）、开场白、评论区（"都这么拍广告是吧？"）全部进 chunks。消费方（shortmind-os 知识库 link 上传）实测切片含明显噪声，且 `<br>` 段内换行导致段落边界错位（切片以句中"了，"开头）。

**给谁用**：所有经 docparse 解析网页/HTML 的消费方（shortmind 知识库 RAG、agent 网页阅读）。

**成功长这样**：同一篇微信公众号文章（mp.weixin.qq.com/s/pm2jBK79SN9CjJqwQUdfHw），`-f chunks` 不再含引导关注/评论区噪声；段落边界对齐自然句；通用网页在无站点规则时由打分兜底。

## 2. 范围与"不做什么"

**做**：
1. 新模块 `html/main_content.rs`：两级正文抽取——
   - **站点规则**：高置信容器候选链（`#js_content` 公众号、`article`、`main`、`[role=main]`、`.rich_media_content`…），第一个"文本量达标"的命中即用；
   - **通用打分兜底**（Readability 简化版）：class/id 正/负语义词表 + 段落文本量 + 链接密度，对 body 下的容器候选打分取最优。
2. **回退边界（防丢内容）**：站点规则命中 → 采用；通用打分仅当候选文本量 ≥ 全页文本量 25% 才采用；两者都不成立 → **回退现行为（全 body 遍历）**——宁可带噪声也不丢正文（"不静默吞数据"）。
3. `<br>` → 段内分隔信号：文本收集遇 `<br>` 插入换行，段落规整时成为边界（修微信段中切断）。
4. 单测：公众号结构 fixture（js_content + 模板头 + 评论区）验证噪声被剔、正文保留；通用页面打分正/反例；`<br>` 段落；回退路径（正文过短页面不清洗）。

**不做**：无头浏览器渲染（JS 动态注入正文——docparse"纯 Rust 快路径"身份约束）；LLM 后处理；`-f json` 契约形状变化（清洗是 html 后端内部行为，IR 不加字段）；站点规则长尾扩张（只收公众号 + 通用语义容器，长尾交给打分兜底）。

## 3. 设计决策

- **打分用 Readability 词表的精简子集**（正：article/body/content/entry/main/post/text；负：comment/discuss/footer/header/-menu/nav/share/signature/advert/promo/sidebar/related/social），不做逗号加权（中文无逗号信号，段落文本量更稳）。
- **链接密度 < 0.35 阈值**：导航/推荐区的强信号（Readability 同款思想）。
- **25% 文本量门槛**：低于此说明打分选错（短正文页面），回退全页——把"清洗过头"的失败模式变成"维持现状"。
- **walk 入口替换而非后过滤**：正文子树确定后从该根遍历——噪声节点根本不进 IR，而非进 IR 后再删（chunks/坐标天然干净）。
- **`<br>` 语义**：公众号段内换行是作者的分段意图，收进 TextChunk 文本时转换行符，layout 层的段落规整自然处理；不做独立的"br 元素"。

## 4. 测试与验收

1. 微信 fixture：模板头 + 引导关注 + js_content 正文 + 评论区 → chunks 无噪声、正文行保留、`<br>` 分段生效。
2. 通用页面：无语义容器、靠打分命中的正例；打分不达标回退全页的反例。
3. 回归：html 后端既有测试全绿；shortmind 侧重传真实微信文章端到端复验。

## 5. shortmind-os 配合项

`upload_link` 抓取后先过 `clean::extract_main_html` 再入流水线（shortmind 自己的 `#js_content` 清洗目前只在 docparse 不可用的降级路径生效——主路径没接上）。两侧清洗同向叠加：shortmind 清洗后发给 docparse 的是正文片段，docparse 的通用兜底负责其余站点。

## 6. 实施记录

已实施（2026-09-07 同日闭环）。

1. **`html/main_content.rs`**（新）：两级抽取 + 回退边界，3 单测（微信 js_content 胜评论区 / 语义容器打分胜链接密集侧栏 / 不自信回退全文档）。
2. **`<br>` → 段内换行**（collect_text）：微信作者的 br 分段保留为块边界。
3. **行级模板噪声过滤**（`NOISE_LINE_PATTERNS` + regex）：公众号"点击上方…关注/长按识别二维码/扫码关注"等平台固定文案，整段 ≤60 字符才匹配（长文提到字样不受影响）；regex 1.12 已在依赖树（零新增供应链面），首版手写 matcher 被否——KISS。
4. **阈值调优**：MIN_SITE_RULE_CHARS 100→50（防"未渲染空壳"而非惩罚短文章），MIN_SCORED_CHARS 200→100——两轮 fixture 实测校准。
5. **真实文章验收**（mp.weixin.qq.com/s/pm2jBK79SN9CjJqwQUdfHw，3.3MB 页面）：chunks 18→8（docparse 直解）；"点击上方蓝色文字关注我们吧"清除 ✓；检索质量不变。"缘分难求"开场白与文末"都这么拍广告是吧？"留言引用**保留**——二者是作者写进正文容器的内容，机器无法判定作者意图（LLM 清洗域，计划 §2 边界外）。
6. shortmind-os 配合（其仓库提交）：`upload_link` 主路径接上 `clean::extract_main_html`，重传实测通过。
7. workspace 380 全绿、clippy 零新增。
