//! Main-content extraction for the HTML backend: pick the subtree that holds
//! the article body so template chrome (follow-me banners, related-post
//! rails, comment sections) never reaches the IR.
//!
//! Two tiers, per the plan (docs/plans/2026-09-07-html-main-content.md):
//!
//! 1. **Site/semantic rules** — a short candidate chain of high-confidence
//!    containers (`#js_content` for WeChat articles, `article`, `main`,
//!    `[role=main]`, …). First hit whose text mass is non-trivial wins.
//!    This is what the industry's cleaners rely on for hard sites; WeChat
//!    alone is 99% of real-world pages.
//! 2. **Generic scoring fallback** (simplified Readability): score every
//!    candidate container by class/id polarity words, paragraph text mass,
//!    and link density; take the best if it holds ≥25% of the page's text.
//!
//! **Fallback boundary**: when neither tier produces a confident candidate
//! the caller walks the whole document — exactly the pre-extraction
//! behavior. A wrong subtree would silently drop the article ("don't
//! swallow data"), so under-confidence degrades to status quo, and the
//! >threshold noise that scoring can't separate stays visible.

use ego_tree::NodeId;
use scraper::{ElementRef, Html, Selector};

/// Minimum characters for a site-rule container to be trusted. The threshold
/// guards against *unrendered shells* (a JS-filled `#js_content` fetched
/// static comes back near-empty) — a real article, even a short notice, sits
/// well above it.
const MIN_SITE_RULE_CHARS: usize = 50;
/// A scored candidate must hold at least this share of the page's text to be
/// adopted (below it, scoring likely picked a sidebar over a short article).
const MIN_TEXT_SHARE: f64 = 0.25;
/// Minimum absolute characters for the score path (tiny pages: keep all).
const MIN_SCORED_CHARS: usize = 100;
/// Link density above this marks a nav/rail container.
const LINK_DENSITY_PENALTY: f64 = 0.4;
const MAX_TEXT_MASS_POINTS: i32 = 100;

/// High-confidence content containers, checked in order. `#js_content` is the
/// WeChat Official-Account article body — the dominant real-world case.
const CONTAINER_RULES: &[&str] = &[
    "#js_content",
    ".rich_media_content",
    "article",
    "main",
    "[role='main']",
    "#content",
];

/// Class/id polarity words (Readability's list, trimmed to what pulls weight
/// on Chinese and English pages alike).
const POSITIVE_WORDS: &[&str] = &[
    "article", "body", "content", "entry", "main", "post", "text",
];
const NEGATIVE_WORDS: &[&str] = &[
    "comment",
    "discuss",
    "footer",
    "header",
    "menu",
    "nav",
    "share",
    "signature",
    "advert",
    "promo",
    "sidebar",
    "related",
    "social",
    "banner",
    "recommend",
    "guide",
    "qr",
];

/// Pick the root node the walker should start from: the main-content subtree
/// when one is confidently identified, else the whole document (`None`).
pub fn main_root(dom: &Html) -> Option<NodeId> {
    // Tier 1 — site/semantic rules.
    for rule in CONTAINER_RULES {
        let Ok(sel) = Selector::parse(rule) else {
            continue;
        };
        if let Some(el) = dom.select(&sel).next() {
            if text_chars(&el) >= MIN_SITE_RULE_CHARS {
                return Some(el.id());
            }
        }
    }
    // Tier 2 — generic scoring across candidate containers.
    let body_sel = Selector::parse("body").ok()?;
    let body = dom.select(&body_sel).next()?;
    let total = text_chars(&body);
    if total < MIN_SCORED_CHARS {
        return None; // tiny page: nothing worth isolating
    }
    let cand_sel = Selector::parse("div,section,article,main,td").expect("static selector");
    let mut best: Option<(i32, NodeId, usize)> = None;
    for cand in dom.select(&cand_sel) {
        let chars = text_chars(&cand);
        // Candidates below the absolute floor can never qualify; skip cheaply.
        if chars < MIN_SCORED_CHARS {
            continue;
        }
        let score = score_container(&cand);
        if score <= 0 {
            continue;
        }
        match best {
            Some((bs, _, _)) if bs >= score => {}
            _ => best = Some((score, cand.id(), chars)),
        }
    }
    let (_, id, chars) = best?;
    (chars as f64 / total as f64 >= MIN_TEXT_SHARE).then_some(id)
}

/// Simplified Readability score: class/id polarity, paragraph text mass,
/// link-density penalty for nav/rail containers.
fn score_container(el: &ElementRef) -> i32 {
    let class_id = format!(
        "{} {}",
        el.value().attr("class").unwrap_or(""),
        el.value().attr("id").unwrap_or("")
    )
    .to_lowercase();
    let mut score: i32 = 0;
    for w in POSITIVE_WORDS {
        if class_id.contains(w) {
            score += 25;
        }
    }
    for w in NEGATIVE_WORDS {
        if class_id.contains(w) {
            score -= 25;
        }
    }
    let p_sel = Selector::parse("p").expect("static selector");
    let text_mass: usize = el
        .select(&p_sel)
        .map(|p| {
            p.text()
                .map(str::trim)
                .map(str::chars)
                .map(Iterator::count)
                .sum::<usize>()
        })
        .sum();
    score += (text_mass as i32 / 25).min(MAX_TEXT_MASS_POINTS);
    let ld = link_density(el);
    if ld > LINK_DENSITY_PENALTY {
        score -= ((ld - LINK_DENSITY_PENALTY) * 200.0) as i32;
    }
    score
}

/// Share of the subtree's text that sits inside `<a>` elements — the classic
/// nav/rail signature (link lists are link-dense; prose is not).
fn link_density(el: &ElementRef) -> f64 {
    let total: usize = el
        .text()
        .map(str::trim)
        .map(str::chars)
        .map(Iterator::count)
        .sum();
    if total == 0 {
        return 0.0;
    }
    let a_sel = Selector::parse("a").expect("static selector");
    let linked: usize = el
        .select(&a_sel)
        .map(|a| {
            a.text()
                .map(str::trim)
                .map(str::chars)
                .map(Iterator::count)
                .sum::<usize>()
        })
        .sum();
    linked as f64 / total as f64
}

/// Trimmed text characters in a subtree.
fn text_chars(el: &ElementRef) -> usize {
    el.text()
        .map(str::trim)
        .map(str::chars)
        .map(Iterator::count)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Debug name of the picked root ("Element(div)" style), for assertions.
    fn root_of(html: &str) -> Option<String> {
        let dom = Html::parse_document(html);
        main_root(&dom).and_then(|id| dom.tree.get(id)).map(|n| {
            let name = match n.value() {
                scraper::node::Node::Element(e) => format!("Element({})", e.name()),
                other => format!("{other:?}"),
            };
            format!("{name}@{:?}", n.id())
        })
    }

    const WECHAT_LIKE: &str = r#"<html><body>
      <div class="rich_media">
        <div class="rich_media_title" id="activity-name">龙头战法核心</div>
        <div class="rich_media_content" id="js_content">
          <p>退潮期不打板、不接力、不抄底，空仓观察是第一原则。</p>
          <p>断板必须立即止损，绝不允许扛单。</p>
          <p>冰点之后往往是新周期的起点，辨识度最高的个股最值得关注。</p>
          <p>当大资金筹码收集后，次日的缩量加速板就是分歧转一致的确定点。</p>
        </div>
        <div class="rich_media_tool">点击上方蓝色文字关注我们吧</div>
      </div>
      <div class="comment_wrp" id="js_cmt_area">
        <p>都这么拍广告是吧？</p>
        <p>谢谢分享！</p>
      </div>
    </body></html>"#;

    #[test]
    fn wechat_js_content_wins_over_comments() {
        let picked = root_of(WECHAT_LIKE).expect("js_content picked");
        assert!(picked.starts_with("Element(div)@"), "{picked}");
        // And it is the js_content node, not the comments wrapper: re-parse
        // and check the subtree text excludes the comment block.
        let dom = Html::parse_document(WECHAT_LIKE);
        let id = main_root(&dom).expect("js_content identified");
        let node = dom.tree.get(id).expect("node exists");
        let text: String = node
            .descendants()
            .filter_map(|n| match n.value() {
                scraper::node::Node::Text(t) => Some(t.to_string()),
                _ => None,
            })
            .collect();
        assert!(text.contains("退潮期不打板"), "{text}");
        assert!(!text.contains("都这么拍广告"), "{text}");
        assert!(!text.contains("点击上方蓝色文字"), "{text}");
    }

    #[test]
    fn semantic_container_beats_sidebar_by_scoring() {
        let html = r#"<html><body>
          <div id="sidebar"> <a href="/a">热门文章一</a> <a href="/b">热门文章二</a>
            <a href="/c">热门文章三</a> <a href="/d">热门文章四</a> <a href="/e">五</a> </div>
          <div class="post-content">
            <p>交易模式的核心四要素是选股、买入、仓位、卖出，四者缺一不可，构成了完整的闭环体系。</p>
            <p>选股端要做最优匹配，需要用数据统计做客观分析，跟随市场的节奏去恒定节点。</p>
            <p>出局以隔日超短为例，当日买入后不论结果如何都要在早盘一小时内找高点走掉。</p>
            <p>仓位如何分配也要根据前端的选股来定，解决连续性问题，避免主观选择性偏差。</p>
            <p>所有问题其实都归结到源头问题，龙头战法的核心同样在选股。</p>
          </div>
        </body></html>"#;
        // No site rule matches; scoring must pick .post-content (text mass +
        // positive word) over the link-dense #sidebar.
        let dom = Html::parse_document(html);
        let id = main_root(&dom).expect("scored candidate");
        let node = dom.tree.get(id).expect("node exists");
        let text: String = node
            .descendants()
            .filter_map(|n| match n.value() {
                scraper::node::Node::Text(t) => Some(t.to_string()),
                _ => None,
            })
            .collect();
        assert!(text.contains("选股端"), "{text}");
        assert!(!text.contains("热门文章一"), "{text}");
    }

    #[test]
    fn under_confident_pages_fall_back_to_full_document() {
        // Short page, no semantic container, scoring can't reach the 25% bar
        // with ≥200 chars — must return None (full-document walk, status quo).
        let html = r#"<html><body><div class="content"><p>只有一句话。</p></div></body></html>"#;
        assert_eq!(root_of(html), None);
    }
}
