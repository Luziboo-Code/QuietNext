//! 网页书签: a link card with the page's title, description and URL.

use super::render::R;
use super::view::Editor;
use crate::editor_theme::{self as theme, Theme};
use crate::feishu_theme::craft;
use crate::model::*;
use gpui::*;
use regex::Regex;
use std::io::Read;
use std::time::Duration;

impl R<'_> {
    pub(super) fn bookmark(&self, b: &Block, bm: &Bookmark, cx: &mut Context<Editor>) -> AnyElement {
        let id = b.id;
        let url = bm.url.clone();
        let target = url.clone();
        let title = if bm.title.trim().is_empty() { display_host(&url) } else { bm.title.clone() };
        let mut card = div().id(("bookmark", id.0)).w_full().flex().flex_col().gap(px(8.)).p(px(16.))
            .rounded(px(8.)).border_1().border_color(Theme::button_border()).bg(crate::theme::raised(Theme::bg_body()))
            .cursor_pointer().hover(|s| s.border_color(theme::c(craft::N400)))
            .child(div().text_size(px(15.)).line_height(px(22.)).font_weight(FontWeight::MEDIUM).text_color(Theme::text())
                .overflow_hidden().whitespace_nowrap().text_ellipsis().child(title));
        if !bm.description.trim().is_empty() {
            card = card.child(div().text_size(px(12.)).line_height(px(19.)).text_color(Theme::text_caption())
                .line_clamp(2).overflow_hidden().child(bm.description.clone()));
        }
        card.child(div().flex().items_center().gap(px(6.)).min_w_0()
            .child(crate::editor_ui::icon("globe", 14., Theme::text_caption()))
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.)).text_color(Theme::text_caption()).child(display_host(&url))))
            .on_mouse_down(MouseButton::Left, cx.listener(move |_, _, _, cx| {
                cx.stop_propagation();
                cx.open_url(&target);
            }))
            .into_any_element()
    }
}

/// `https://wx.mail.qq.com/list/...` → `wx.mail.qq.com`.
fn display_host(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| url.to_string())
}

impl Editor {
    /// Insert a link card at the caret; its title and description fill in when fetched.
    pub(super) fn insert_bookmark(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        let url = url.to_string();
        let block = Block::new(BlockKind::Bookmark(Bookmark { title: display_host(&url), description: String::new(), url: url.clone() }), RichText::default());
        let mut before = std::collections::HashSet::new();
        self.core.doc.for_each_block(|b| if let BlockKind::Bookmark(bm) = &b.kind && bm.url == url { before.insert(b.id); });
        self.core.clear_block_sel();
        self.core.cell_sel = None;
        self.core.paste_blocks(vec![block, Block::text("")]);
        let mut id = None;
        self.core.doc.for_each_block(|b| if let BlockKind::Bookmark(bm) = &b.kind
            && bm.url == url && !before.contains(&b.id) { id = Some(b.id); });
        let Some(id) = id else { return };
        self.changed(window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let fetch_url = url.clone();
            let meta = cx.background_executor().spawn(async move { fetch_meta(&fetch_url) }).await;
            let Some((title, description)) = meta else { return };
            let title = if title.trim().is_empty() { display_host(&url) } else { title };
            let fresh = Bookmark { url, title, description };
            let _ = this.update_in(cx, |e, window, cx| {
                if e.core.refresh_bookmark(id, fresh) { e.changed(window, cx); }
            });
        })
        .detach();
    }
}

/// Fetch the page's title and description (best effort; the card already shows its URL).
fn fetch_meta(url: &str) -> Option<(String, String)> {
    let html = fetch_html(url, true).or_else(|| fetch_html(url, false))?;
    let title = meta_content(&html, &["og:title", "twitter:title"]).or_else(|| title_tag(&html)).unwrap_or_default();
    let description = meta_content(&html, &["og:description", "twitter:description", "description"]).unwrap_or_default();
    (!title.is_empty() || !description.is_empty()).then(|| (clean(&title, 200), clean(&description, 500)))
}

/// The page as text: through the environment's proxy first, then directly.
fn fetch_html(url: &str, from_env: bool) -> Option<String> {
    let mut agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(8));
    if !from_env { agent = agent.try_proxy_from_env(false); }
    let resp = agent.build().get(url)
        .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) QuietNext/0.1")
        .set("Accept", "text/html,application/xhtml+xml")
        .call().ok()?;
    let mut html = String::new();
    resp.into_reader().take(1024 * 1024).read_to_string(&mut html).ok()?;
    Some(html)
}

fn meta_content(html: &str, keys: &[&str]) -> Option<String> {
    let tags = Regex::new(r"(?is)<meta\b[^>]*>").ok()?;
    let attrs = Regex::new(r#"(?is)([a-zA-Z:_-]+)\s*=\s*["']([^"']*)["']"#).ok()?;
    for tag in tags.find_iter(html) {
        let (mut name, mut content) = (None, None);
        for cap in attrs.captures_iter(tag.as_str()) {
            let key = cap[1].to_ascii_lowercase();
            let value = decode(&cap[2]);
            if key == "property" || key == "name" { name = Some(value); }
            else if key == "content" { content = Some(value); }
        }
        if let (Some(name), Some(content)) = (name, content)
            && keys.iter().any(|k| k.eq_ignore_ascii_case(&name)) && !content.trim().is_empty()
        {
            return Some(content);
        }
    }
    None
}

fn title_tag(html: &str) -> Option<String> {
    let re = Regex::new(r"(?is)<title[^>]*>(.*?)</title>").ok()?;
    re.captures(html).map(|c| decode(&c[1]))
}

fn clean(s: &str, max: usize) -> String {
    let s = Regex::new(r"(?is)<[^>]+>").ok().map(|r| r.replace_all(s, " ").into_owned()).unwrap_or_else(|| s.to_string());
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s.chars().take(max).collect()
}

fn decode(s: &str) -> String {
    let s = s.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'")
        .replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ");
    let re = match Regex::new(r"&#(x?[0-9a-fA-F]+);") { Ok(r) => r, Err(_) => return s };
    re.replace_all(&s, |c: &regex::Captures| {
        let raw = &c[1];
        let n = if let Some(hex) = raw.strip_prefix('x').or_else(|| raw.strip_prefix('X')) { u32::from_str_radix(hex, 16).ok() } else { raw.parse::<u32>().ok() };
        n.and_then(char::from_u32).map(|c| c.to_string()).unwrap_or_else(|| c[0].to_string())
    })
    .into_owned()
}
