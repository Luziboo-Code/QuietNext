//! 复习: spaced repetition for notes, after Weave Review and Anki's four
//! grades. Cards live in `.quiet/review.json`; nothing leaves the library.
//! The page is the same shape as 日历: cards on the left, the session in the
//! middle, and today's numbers on the right.

use crate::app::Workspace;
use crate::input::{TextField, TextFieldEvent};
use crate::settings::{self, tr};
use crate::theme::{self, Theme};
use crate::ui::{self, icon, TitleBarExt};
use gpui::{prelude::FluentBuilder, *};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

const DAY: i64 = 86_400;
const MINUTE: i64 = 60;
const PANEL_W: f32 = 224.;
const DETAIL_W: f32 = 268.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    #[default]
    New,
    Learning,
    Review,
    Relearning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rating {
    Again,
    Hard,
    Good,
    Easy,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    pub front: String,
    pub back: String,
    #[serde(default)]
    pub created: i64,
    /// Unix seconds.
    #[serde(default)]
    pub due: i64,
    /// Seconds.
    #[serde(default)]
    pub interval: i64,
    #[serde(default)]
    pub ease: f32,
    #[serde(default)]
    pub reps: u32,
    #[serde(default)]
    pub lapses: u32,
    #[serde(default)]
    pub state: State,
    #[serde(default)]
    pub last: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Log {
    pub time: i64,
    pub card: String,
    pub rating: u8,
    pub interval: i64,
    #[serde(default)]
    pub was_new: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct ReviewStore {
    #[serde(default)]
    cards: Vec<Card>,
    #[serde(default)]
    log: Vec<Log>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Filter {
    All,
    Due,
    New,
}

pub struct Draft {
    id: Option<String>,
    front: Entity<TextField>,
    back: Entity<TextField>,
    /// 删除 was pressed once: the next press really removes the card.
    armed: bool,
    error: Option<SharedString>,
}

pub struct Session {
    queue: Vec<String>,
    at: usize,
    revealed: bool,
    completed: u32,
}

#[derive(Clone)]
pub struct AiDraft {
    front: String,
    back: String,
    selected: bool,
}

/// 复习 › AI 生成卡片: the chosen documents and the preview of what came back.
pub struct DocPick {
    id: String,
    title: String,
    selected: bool,
    /// Folder ids from the nearest parent up to the root.
    folders: Vec<String>,
}

pub struct AiPanel {
    docs: Vec<DocPick>,
    folders: Vec<(String, String)>,
    selected_folders: Vec<String>,
    folder_menu: Option<Point<Pixels>>,
    search: Entity<TextField>,
    count: u32,
    generating: bool,
    error: Option<SharedString>,
    results: Vec<AiDraft>,
    _search_sub: Subscription,
}

pub struct Review {
    cards: Vec<Card>,
    log: Vec<Log>,
    draft: Option<Draft>,
    session: Option<Session>,
    ai: Option<AiPanel>,
    filter: Filter,
    _subs: Vec<Subscription>,
}

fn path() -> PathBuf { crate::store::Store::internal_dir().join("review.json") }

/// The API key is machine-local (per library), never in the synced library.
fn ai_key_path() -> PathBuf {
    use std::hash::{Hash as _, Hasher as _};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    crate::store::Store::root_dir().to_string_lossy().hash(&mut h);
    dirs::data_local_dir().or_else(dirs::home_dir).unwrap_or_else(std::env::temp_dir)
        .join("quiet-md").join("libraries").join(format!("{:016x}-ai-key.json", h.finish()))
}

fn load_ai_key() -> String {
    std::fs::read(ai_key_path()).ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("api_key").and_then(|k| k.as_str()).map(str::to_string))
        .unwrap_or_default()
}

fn save_ai_key(key: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let p = ai_key_path();
    if let Some(dir) = p.parent() { std::fs::create_dir_all(dir)?; }
    let tmp = p.with_extension("json.tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(serde_json::json!({ "api_key": key }).to_string().as_bytes())?;
    f.sync_all()?;
    std::fs::rename(tmp, p)
}

pub(crate) fn ai_key() -> String { load_ai_key() }
pub(crate) fn set_ai_key(key: &str) -> std::io::Result<()> { save_ai_key(key) }

fn load() -> ReviewStore {
    fdl_core::vfs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(cards: &[Card], log: &[Log]) -> anyhow::Result<()> {
    use std::io::Write;
    let dir = crate::store::Store::internal_dir();
    fdl_core::vfs::create_dir_all(&dir)?;
    let tmp = dir.join("review.json.tmp");
    let mut f = fdl_core::vfs::File::create(&tmp)?;
    f.write_all(&serde_json::to_vec_pretty(&ReviewStore { cards: cards.to_vec(), log: log.to_vec() })?)?;
    f.sync_all()?;
    fdl_core::vfs::rename(tmp, path())?;
    Ok(())
}

impl Default for Review {
    fn default() -> Self {
        let store = load();
        Self { cards: store.cards, log: store.log, draft: None, session: None, ai: None, filter: Filter::All, _subs: Vec::new() }
    }
}

fn now() -> i64 { crate::time::now() as i64 }

fn day_start(secs: i64) -> i64 {
    let (y, m, d, _, _) = crate::time::civil(secs.max(0) as u64);
    crate::time::local_to_unix(y, m, d, 0, 0)
}

fn human(secs: i64) -> String {
    let secs = secs.max(MINUTE);
    if secs < 3600 { crate::localized_format!("{} 分钟", "{} min", (secs / MINUTE).max(1)) }
    else if secs < DAY { crate::localized_format!("{} 小时", "{} h", secs / 3600) }
    else if secs < 30 * DAY { crate::localized_format!("{} 天", "{} d", secs / DAY) }
    else if secs < 365 * DAY { crate::localized_format!("{} 个月", "{} mo", secs / (30 * DAY)) }
    else { crate::localized_format!("{} 年", "{} y", secs / (365 * DAY)) }
}

fn status(state: State) -> &'static str {
    match state {
        State::New => tr("新卡片", "New"),
        State::Learning | State::Relearning => tr("学习中", "Learning"),
        State::Review => tr("复习中", "Review"),
    }
}

fn due_label(card: &Card, at: i64) -> String {
    if card.state == State::New { return tr("新卡片", "New").to_string() }
    if card.due <= at { return tr("待复习", "Due").to_string() }
    crate::localized_format!("{} 天后", "in {} d", ((card.due - at + DAY - 1) / DAY).max(1))
}

/// One review's next interval (seconds). FSRS's shape with the four grades:
/// short learning steps, then an ease factor that grows Good and Easy.
fn schedule(card: &mut Card, rating: Rating, at: i64) -> i64 {
    let ease = if card.ease <= 0. { 2.5 } else { card.ease.clamp(1.3, 2.8) };
    let was_review = card.state == State::Review;
    let (interval, state) = match card.state {
        State::New => match rating {
            Rating::Again => (MINUTE, State::Learning),
            Rating::Hard => (6 * MINUTE, State::Learning),
            Rating::Good => (10 * MINUTE, State::Learning),
            Rating::Easy => (4 * DAY, State::Review),
        },
        State::Learning | State::Relearning => match rating {
            Rating::Again => (10 * MINUTE, State::Learning),
            Rating::Hard => (DAY, State::Learning),
            Rating::Good => (2 * DAY, State::Review),
            Rating::Easy => (4 * DAY, State::Review),
        },
        State::Review => {
            let base = card.interval.max(DAY) as f32;
            match rating {
                Rating::Again => { card.lapses += 1; (DAY, State::Relearning) }
                Rating::Hard => ((base * 1.2) as i64, State::Review),
                Rating::Good => ((base * ease) as i64, State::Review),
                Rating::Easy => ((base * ease * 1.3) as i64, State::Review),
            }
        }
    };
    card.ease = match rating {
        Rating::Again => (ease - 0.2).max(1.3),
        Rating::Hard => (ease - 0.15).max(1.3),
        Rating::Good => ease,
        Rating::Easy => (ease + 0.15).min(2.8),
    };
    // 目标记忆率: 85% leaves more room between reviews, 95% tightens them.
    let target = match crate::settings::prefs().review_retention { 85 => 1.15, 95 => 0.8, _ => 1.0 };
    let interval = if was_review { (interval as f32 * target) as i64 } else { interval };
    card.interval = interval.clamp(MINUTE, 3650 * DAY);
    card.due = at + card.interval;
    card.state = state;
    card.reps += 1;
    card.last = at;
    card.interval
}

fn preview(card: &Card, rating: Rating) -> String {
    let mut copy = card.clone();
    human(schedule(&mut copy, rating, now()))
}

impl Review {
    fn sorted(&self, at: i64) -> Vec<Card> {
        let mut out: Vec<Card> = self.cards.iter()
            .filter(|c| match self.filter {
                Filter::All => true,
                Filter::Due => c.state != State::New && c.due <= at,
                Filter::New => c.state == State::New,
            })
            .cloned().collect();
        out.sort_by_key(|c| (c.state == State::New, c.due, c.front.clone()));
        out
    }

    pub fn due(&self, at: i64) -> usize { self.cards.iter().filter(|c| c.state != State::New && c.due <= at).count() }
    pub fn fresh(&self) -> usize { self.cards.iter().filter(|c| c.state == State::New).count() }
    fn learning(&self) -> usize { self.cards.iter().filter(|c| matches!(c.state, State::Learning | State::Relearning)).count() }
    fn reviewing(&self) -> usize { self.cards.iter().filter(|c| c.state == State::Review).count() }

    fn new_today(&self, at: i64) -> u32 {
        let start = day_start(at);
        self.log.iter().filter(|l| l.was_new && l.time >= start).count() as u32
    }

    fn reviewed_today(&self, at: i64) -> usize {
        let start = day_start(at);
        self.log.iter().filter(|l| l.time >= start).count()
    }

    fn retention_today(&self, at: i64) -> Option<u32> {
        let start = day_start(at);
        let today: Vec<&Log> = self.log.iter().filter(|l| l.time >= start).collect();
        (!today.is_empty()).then(|| (today.iter().filter(|l| l.rating >= 2).count() * 100 / today.len()) as u32)
    }
}

impl Workspace {
    pub(crate) fn render_review(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let at = now();
        let due = self.review.due(at);
        let fresh = self.review.fresh();
        let title = div().text_size(px(20.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).child(tr("复习", "Review"));
        let summary = div().ml(px(14.)).flex().items_center().gap(px(10.)).text_size(px(12.)).text_color(Theme::text_caption())
            .child(crate::localized_format!("{} 张待复习", "{} due", due))
            .child("·")
            .child(crate::localized_format!("{} 张新卡片", "{} new", fresh));
        let ai = div().id("rev-ai").no_drag().h(px(28.)).px(px(12.)).flex().items_center().rounded(px(8.)).cursor_pointer()
            .bg(Theme::fill_hover_strong()).text_size(px(12.)).hover(|s| s.bg(Theme::fill_pressed()))
            .child(if self.review.ai.is_some() { tr("关闭 AI", "Close AI") } else { tr("AI 生成卡片", "AI cards") })
            .on_click(cx.listener(|w, _, window, cx| {
                if w.review.ai.is_some() { w.review.ai = None; cx.notify(); } else { w.open_ai(window, cx); }
            }));
        let new_card = div().id("rev-new").no_drag().h(px(28.)).px(px(12.)).flex().items_center().rounded(px(8.)).cursor_pointer()
            .bg(theme::raised(Theme::bg_body())).border_1().border_color(Theme::button_border()).text_size(px(12.))
            .hover(|s| s.bg(Theme::fill_hover())).child(tr("新建卡片", "New card"))
            .on_click(cx.listener(|w, _, window, cx| w.open_card(None, window, cx)));
        let session = if self.review.session.is_some() {
            div().id("rev-stop").no_drag().h(px(28.)).px(px(12.)).flex().items_center().rounded(px(8.)).cursor_pointer()
                .bg(Theme::fill_hover_strong()).text_size(px(12.)).hover(|s| s.bg(Theme::fill_pressed())).child(tr("结束复习", "End session"))
                .on_click(cx.listener(|w, _, _, cx| { w.review.session = None; cx.notify(); }))
        } else {
            div().id("rev-start").no_drag().h(px(28.)).px(px(14.)).flex().items_center().rounded(px(8.)).cursor_pointer()
                .bg(Theme::primary()).text_color(gpui::white()).text_size(px(12.)).hover(|s| s.opacity(0.9)).child(tr("开始复习", "Start review"))
                .on_click(cx.listener(|w, _, _, cx| w.start_review(cx)))
        };
        let header_inner = div().w_full().flex().items_center().child(title).child(summary).child(div().flex_1())
            .child(div().mr(px(8.)).child(ai)).child(div().mr(px(8.)).child(new_card)).child(session);
        let header = crate::trash::page_header_with(self, header_inner, 26., cx);

        let body = div().flex().flex_1().min_h_0()
            .child(self.review_sidebar(cx))
            .child(self.review_main(window, cx))
            .child(self.review_stats(cx));
        div().flex_1().min_w_0().h_full().flex().flex_col().bg(Theme::bg_body())
            .font_family(theme::ui_family()).text_color(Theme::text())
            .child(header).child(body).into_any_element()
    }

    fn review_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let at = now();
        let cards = self.review.sorted(at);
        let filters = [(Filter::All, tr("全部", "All")), (Filter::Due, tr("待复习", "Due")), (Filter::New, tr("新卡片", "New"))];
        let tabs = div().h(px(26.)).p(px(2.)).flex().gap(px(2.)).rounded(px(8.)).bg(Theme::fill_hover_strong())
            .children(filters.map(|(f, label)| {
                let on = self.review.filter == f;
                div().id(("rev-filter", f as usize)).h_full().flex_1().flex().items_center().justify_center().rounded(px(6.)).cursor_pointer()
                    .text_size(px(11.)).line_height(px(16.))
                    .when(on, |d| d.bg(if theme::dark() { theme::rgbaf(0x63, 0x63, 0x66, 1.) } else { Theme::bg_body() }).shadow(theme::shadow_button()).text_color(Theme::text()))
                    .when(!on, |d| d.text_color(Theme::text_caption()).hover(|s| s.text_color(Theme::text())))
                    .child(label)
                    .on_click(cx.listener(move |w, _, _, cx| { w.review.filter = f; cx.notify(); }))
            }));
        let mut list = div().id("rev-list").flex_1().min_h_0().overflow_y_scroll().flex().flex_col().gap(px(1.));
        if cards.is_empty() {
            list = list.child(div().px(px(6.)).py(px(12.)).text_size(px(11.)).text_color(Theme::text_placeholder()).child(tr("暂无卡片", "No cards")));
        }
        for c in &cards {
            let id = c.id.clone();
            let sub = format!("{} · {}", status(c.state), due_label(c, at));
            list = list.child(div().id(SharedString::from(format!("rev-row-{}", id))).min_h(px(40.)).px(px(7.)).py(px(4.)).flex().items_center().gap(px(7.)).rounded(px(6.))
                .cursor_pointer().hover(|s| s.bg(Theme::fill_hover()))
                .child(div().w(px(3.)).h(px(24.)).rounded(px(2.)).bg(match c.state { State::New => theme::c(0x2383e2), State::Learning | State::Relearning => theme::c(0xd9730d), State::Review => theme::c(0x0f7b6c) }))
                .child(div().flex_1().min_w_0().flex().flex_col()
                    .child(div().text_size(px(11.)).truncate().child(c.front.clone()))
                    .child(div().text_size(px(9.)).text_color(Theme::text_placeholder()).child(sub)))
                .on_click(cx.listener(move |w, _, window, cx| { cx.stop_propagation(); w.open_card(Some(id.clone()), window, cx); })));
        }
        div().flex_none().w(px(PANEL_W)).h_full().border_r_1().border_color(Theme::line_divider())
            .flex().flex_col().px(px(14.)).py(px(10.)).overflow_hidden()
            .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text_caption()).child(tr("牌组", "Deck")))
            .child(div().id("rev-deck").mt(px(6.)).h(px(30.)).px(px(6.)).flex().items_center().gap(px(8.)).rounded(px(7.)).bg(Theme::fill_hover())
                .child(icon("review", 14., Theme::primary()))
                .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).child(tr("全部卡片", "All cards")))
                .child(div().text_size(px(11.)).text_color(Theme::text_placeholder()).child(self.review.cards.len().to_string())))
            .child(div().mt(px(16.)).mb(px(6.)).text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text_caption()).child(tr("筛选", "Filter")))
            .child(tabs).child(div().mt(px(8.)).child(list))
            .child(div().pt(px(8.)).px(px(4.)).text_size(px(11.)).line_height(px(16.)).text_color(Theme::text_placeholder())
                .child(tr("卡片保存在文档库中", "Cards are saved in the library")))
    }

    fn review_main(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.review.ai.is_some() { return self.review_ai(window, cx) }
        if self.review.draft.is_some() { return self.review_editor(window, cx) }
        if self.review.session.is_some() { return self.review_session(cx) }
        self.review_home(cx)
    }

    fn review_home(&self, cx: &mut Context<Self>) -> AnyElement {
        let at = now();
        let due = self.review.due(at);
        let fresh = self.review.fresh();
        let total = due + fresh;
        let mut page = div().flex_1().min_w_0().h_full().flex().flex_col().items_center().justify_center().gap(px(10.)).px(px(32.));
        page = page.child(div().size(px(56.)).rounded(px(14.)).bg(Theme::fill_hover()).flex().items_center().justify_center().child(icon("review", 28., Theme::primary())));
        if self.review.cards.is_empty() {
            page = page
                .child(div().mt(px(8.)).text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).child(tr("还没有卡片", "No cards yet")))
                .child(div().max_w(px(420.)).text_center().text_size(px(13.)).line_height(px(20.)).text_color(Theme::text_caption())
                    .child(tr("先新建一张卡片，正面写问题，背面写答案。之后按间隔安排复习。", "Create a card with a question on the front and the answer on the back; reviews are scheduled for you.")))
                .child(div().mt(px(10.)).id("rev-empty-new").h(px(30.)).px(px(14.)).flex().items_center().rounded(px(8.)).cursor_pointer()
                    .bg(Theme::primary()).text_color(gpui::white()).text_size(px(12.)).hover(|s| s.opacity(0.9)).child(tr("新建卡片", "New card"))
                    .on_click(cx.listener(|w, _, window, cx| w.open_card(None, window, cx))));
        } else {
            page = page
                .child(div().mt(px(8.)).text_size(px(13.)).text_color(Theme::text_caption()).child(tr("今日待复习", "Due today")))
                .child(div().text_size(px(52.)).line_height(px(60.)).font_weight(FontWeight::SEMIBOLD).child(total.to_string()))
                .child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(crate::localized_format!("{} 张到期 · {} 张新卡片", "{} due · {} new", due, fresh)))
                .child(div().mt(px(10.)).id("rev-home-start").h(px(32.)).px(px(18.)).flex().items_center().rounded(px(9.)).cursor_pointer()
                    .bg(Theme::primary()).text_color(gpui::white()).text_size(px(13.)).when(total == 0, |d| d.opacity(0.6)).hover(|s| s.opacity(0.9))
                    .child(tr("开始复习", "Start review"))
                    .on_click(cx.listener(|w, _, _, cx| w.start_review(cx))));
        }
        page.into_any_element()
    }

    fn review_session(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(s) = &self.review.session else { return div().into_any_element() };
        let total = s.queue.len();
        let done = s.at >= total;
        let card = s.queue.get(s.at).and_then(|id| self.review.cards.iter().find(|c| &c.id == id)).cloned();
        let progress = div().flex_none().px(px(32.)).pt(px(20.)).flex().items_center().gap(px(14.))
            .child(div().flex_1().h(px(6.)).rounded(px(3.)).bg(Theme::fill_hover_strong())
                .child(div().h_full().rounded(px(3.)).bg(Theme::primary()).w(relative(if total == 0 { 0. } else { s.at as f32 / total as f32 }))))
            .child(div().text_size(px(11.)).text_color(Theme::text_caption()).child(format!("{} / {total}", s.at.min(total))));
        if done || card.is_none() {
            return div().flex_1().min_w_0().h_full().flex().flex_col().child(progress)
                .child(div().flex_1().flex().flex_col().items_center().justify_center().gap(px(12.))
                    .child(div().size(px(64.)).rounded(px(20.)).bg(theme::ca(0x0f7b6c, 0.12)).shadow(theme::shadow_card())
                        .flex().items_center().justify_center().child(icon("todo-check", 30., theme::c(0x0f7b6c))))
                    .child(div().mt(px(4.)).text_size(px(22.)).font_weight(FontWeight::SEMIBOLD).child(tr("今天的复习完成了", "All caught up")))
                    .child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(crate::localized_format!("复习了 {} 张卡片", "Reviewed {} cards", s.completed)))
                    .child(div().id("rev-done").mt(px(8.)).h(px(32.)).px(px(18.)).flex().items_center().rounded(px(9.)).cursor_pointer()
                        .bg(Theme::primary()).text_color(gpui::white()).text_size(px(12.)).hover(|s| s.opacity(0.9)).child(tr("完成", "Done"))
                        .on_click(cx.listener(|w, _, _, cx| { w.review.session = None; cx.notify(); }))))
                .into_any_element();
        }
        let card = card.unwrap();
        let state = status(card.state);
        let mut panel = div().w_full().min_h(px(320.)).p(px(36.)).rounded(px(18.))
            .border_1().border_color(Theme::line_border()).bg(theme::raised(Theme::bg_body())).shadow(theme::shadow_card())
            .flex().flex_col().gap(px(18.))
            .child(div().flex().items_center().justify_between()
                .child(div().text_size(px(10.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text_placeholder()).child(tr("问题", "Question")))
                .child(div().text_size(px(10.)).text_color(Theme::text_placeholder()).child(state)))
            .child(div().flex_1().flex().items_center()
                .child(div().w_full().text_size(px(23.)).line_height(px(36.)).child(card.front.clone())));
        if s.revealed {
            panel = panel
                .child(div().h(px(1.)).bg(Theme::line_divider()))
                .child(div().p(px(16.)).rounded(px(12.)).bg(Theme::fill_hover()).flex().flex_col().gap(px(6.))
                    .child(div().text_size(px(10.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text_placeholder()).child(tr("答案", "Answer")))
                    .child(div().text_size(px(18.)).line_height(px(28.)).child(card.back.clone())));
        }
        let actions = if s.revealed {
            let colors = [(Rating::Again, tr("重来", "Again"), 0xd14345), (Rating::Hard, tr("困难", "Hard"), 0xd9730d), (Rating::Good, tr("良好", "Good"), 0x2383e2), (Rating::Easy, tr("简单", "Easy"), 0x0f7b6c)];
            div().w_full().flex().gap(px(10.)).children(colors.map(|(rating, label, color)| {
                div().id(("rev-rate", rating as usize)).flex_1().h(px(62.)).rounded(px(12.)).cursor_pointer()
                    .bg(theme::ca(color, 0.12)).border_1().border_color(theme::ca(color, 0.2)).hover(|s| s.bg(theme::ca(color, 0.2)))
                    .flex().flex_col().items_center().justify_center().gap(px(2.))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).text_color(theme::c(color)).child(label))
                    .child(div().text_size(px(10.)).text_color(Theme::text_caption()).child(preview(&card, rating)))
                    .on_click(cx.listener(move |w, _, _, cx| w.rate_review(rating, cx)))
            }))
        } else {
            div().w_full().flex().justify_center().child(
                div().id("rev-reveal").h(px(38.)).px(px(26.)).flex().items_center().rounded(px(10.)).cursor_pointer()
                    .bg(Theme::primary()).text_color(gpui::white()).text_size(px(13.)).shadow(theme::shadow_button()).hover(|s| s.opacity(0.9)).child(tr("显示答案", "Show answer"))
                    .on_click(cx.listener(|w, _, _, cx| { if let Some(s) = &mut w.review.session { s.revealed = true; } cx.notify(); })))
        };
        div().flex_1().min_w_0().h_full().flex().flex_col().child(progress)
            .child(div().id("rev-session-scroll").flex_1().min_h_0().overflow_y_scroll().px(px(32.)).pt(px(120.)).pb(px(32.)).flex().flex_col().items_center()
                .child(div().w_full().max_w(px(680.)).flex().flex_col().gap(px(18.)).child(panel).child(actions)))
            .into_any_element()
    }

    fn review_editor(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(d) = &self.review.draft else { return div().into_any_element() };
        let _ = window;
        let area = |e: Entity<TextField>, h: f32| div().w_full().min_h(px(h)).px(px(10.)).py(px(8.)).rounded(px(9.))
            .border_1().border_color(Theme::line_border()).bg(theme::raised(Theme::bg_body())).flex().child(e);
        let staff = div().id("rev-editor-scroll").flex_1().min_w_0().h_full().overflow_y_scroll().px(px(28.)).py(px(24.)).flex().flex_col().items_center()
            .child(div().w_full().max_w(px(620.)).flex().flex_col().gap(px(12.))
                .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(if d.id.is_some() { tr("编辑卡片", "Edit card") } else { tr("新建卡片", "New card") }))
                .child(div().text_size(px(11.)).text_color(Theme::text_caption()).child(tr("正面", "Front")))
                .child(area(d.front.clone(), 96.))
                .child(div().text_size(px(11.)).text_color(Theme::text_caption()).child(tr("背面", "Back")))
                .child(area(d.back.clone(), 120.))
                .children(d.error.clone().map(|e| div().text_size(px(11.)).text_color(Theme::danger()).child(e)))
                .child(div().mt(px(4.)).flex().items_center().gap(px(8.))
                    .when(d.id.is_some(), |x| x.child(settings::link_button("rev-delete", if d.armed { tr("确认删除", "Confirm delete") } else { tr("删除", "Delete") })
                        .on_click(cx.listener(|w, _, _, cx| w.delete_card(cx)))))
                    .child(div().flex_1())
                    .child(ui::secondary_button("rev-cancel", tr("取消", "Cancel"))
                        .on_click(cx.listener(|w, _, _, cx| { w.review.draft = None; cx.notify(); })))
                    .child(ui::primary_button("rev-save", tr("保存", "Save"))
                        .on_click(cx.listener(|w, _, _, cx| w.commit_card(cx))))));
        staff.into_any_element()
    }

    /// Today's numbers, and a way to the review settings.
    fn review_stats(&self, cx: &mut Context<Self>) -> Div {
        let at = now();
        let retention = self.review.retention_today(at);
        let row = |label: &'static str, value: String, color: Hsla| {
            div().h(px(30.)).flex().items_center().justify_between()
                .child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(label))
                .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(color).child(value))
        };
        let p = settings::prefs();
        div().flex_none().w(px(DETAIL_W)).h_full().border_l_1().border_color(Theme::line_divider())
            .bg(theme::raised(Theme::bg_body())).flex().flex_col().px(px(14.)).py(px(14.))
            .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text_caption()).child(tr("统计", "Stats")))
            .child(div().mt(px(8.)).flex().flex_col()
                .child(row(tr("待复习", "Due"), self.review.due(at).to_string(), theme::c(0x2383e2)))
                .child(row(tr("新卡片", "New"), self.review.fresh().to_string(), theme::c(0x6940a5)))
                .child(row(tr("学习中", "Learning"), self.review.learning().to_string(), theme::c(0xd9730d)))
                .child(row(tr("复习中", "Review"), self.review.reviewing().to_string(), theme::c(0x0f7b6c)))
                .child(row(tr("今日已复习", "Reviewed today"), self.review.reviewed_today(at).to_string(), Theme::text()))
                .child(row(tr("今日记忆率", "Retention today"), retention.map_or_else(|| "—".into(), |r| format!("{r}%")), Theme::text())))
            .child(div().mt(px(18.)).h(px(1.)).bg(Theme::line_divider()))
            .child(div().mt(px(14.)).text_size(px(11.)).text_color(Theme::text_caption())
                .child(crate::localized_format!("每日新卡片 {} 张 · 目标记忆率 {}%", "{} new/day · {}% retention", p.review_new_per_day, p.review_retention)))
            .child(div().mt(px(8.)).child(settings::link_button("rev-settings", tr("复习设置", "Review settings"))
                .on_click(cx.listener(|w, _, _, cx| w.open_settings(crate::settings::Page::Review, cx)))))
    }

    pub(crate) fn open_ai(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.current.clone();
        let parents: HashMap<String, String> = self.entries.iter()
            .filter_map(|e| e.meta.parent.clone().map(|p| (e.id.clone(), p)))
            .collect();
        let mut folders: Vec<(String, String)> = self.entries.iter()
            .filter(|e| e.meta.folder.is_some())
            .map(|e| (e.id.clone(), crate::settings::display_title(&e.title)))
            .collect();
        folders.sort_by_key(|f| f.1.to_lowercase());
        let mut docs: Vec<DocPick> = self.entries.iter()
            .filter(|e| e.meta.folder.is_none() && e.meta.kind == crate::store::DocKind::Doc && !e.meta.local())
            .map(|e| {
                let mut chain = Vec::new();
                let mut parent = e.meta.parent.clone();
                while let Some(p) = parent {
                    parent = parents.get(&p).cloned();
                    chain.push(p);
                }
                DocPick {
                    id: e.id.clone(),
                    title: crate::settings::display_title(&e.title),
                    selected: current.as_deref() == Some(e.id.as_str()),
                    folders: chain,
                }
            })
            .collect();
        docs.sort_by_key(|d| d.title.to_lowercase());
        let search = cx.new(|cx| {
            let mut f = TextField::new(tr("搜索文档", "Search documents"), cx);
            f.size = 12.;
            f.line_height = 18.;
            f
        });
        let search_sub = cx.subscribe(&search, |_, _, ev: &TextFieldEvent, cx| {
            if matches!(ev, TextFieldEvent::Changed) { cx.notify() }
        });
        self.review.ai = Some(AiPanel {
            docs, folders, selected_folders: Vec::new(), folder_menu: None, search: search.clone(), count: 5,
            generating: false, error: None, results: Vec::new(), _search_sub: search_sub,
        });
        self.review.draft = None;
        self.review.session = None;
        window.focus(&search.focus_handle(cx), cx);
        cx.notify();
    }

    fn review_ai(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(a) = &self.review.ai else { return div().into_any_element() };
        let label = |text: &'static str| div().text_size(px(11.)).text_color(Theme::text_caption()).child(text);
        let selected = a.results.iter().filter(|r| r.selected).count();
        let chosen = a.docs.iter().filter(|d| d.selected).count();
        let query = a.search.read(cx).text.trim().to_lowercase();
        let picks = &a.selected_folders;
        let visible: Vec<(usize, &DocPick)> = a.docs.iter().enumerate()
            .filter(|(_, d)| picks.is_empty() || d.folders.iter().any(|x| picks.contains(x)))
            .filter(|(_, d)| query.is_empty() || d.title.to_lowercase().contains(&query))
            .collect();
        let all_visible = !visible.is_empty() && visible.iter().all(|(_, d)| d.selected);
        let mut doc_list = div().id("ai-docs").max_h(px(240.)).overflow_y_scroll().flex().flex_col().gap(px(1.))
            .rounded(px(9.)).border_1().border_color(Theme::line_border()).bg(theme::raised(Theme::bg_body())).p(px(4.));
        if a.docs.is_empty() {
            doc_list = doc_list.child(div().px(px(8.)).py(px(12.)).text_size(px(11.)).text_color(Theme::text_placeholder())
                .child(tr("文档库里还没有文档，先新建一篇笔记。", "No documents yet; create a note first.")));
        } else if visible.is_empty() {
            doc_list = doc_list.child(div().px(px(8.)).py(px(12.)).text_size(px(11.)).text_color(Theme::text_placeholder())
                .child(tr("没有匹配的文档", "No matching documents")));
        }
        for (i, d) in &visible {
            let idx = *i;
            doc_list = doc_list.child(div().id(("ai-doc", idx)).h(px(30.)).px(px(7.)).flex().items_center().gap(px(8.)).rounded(px(6.))
                .cursor_pointer().hover(|s| s.bg(Theme::fill_hover()))
                .child(ui::checkbox(d.selected))
                .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).child(d.title.clone()))
                .on_click(cx.listener(move |w, _, _, cx| w.ai_toggle_doc(idx, cx))));
        }
        let folder_label: SharedString = if a.selected_folders.is_empty() {
            tr("全部文件夹", "All folders").into()
        } else if a.selected_folders.len() == 1 {
            a.folders.iter().find(|(id, _)| a.selected_folders.contains(id))
                .map(|(_, name)| name.clone().into())
                .unwrap_or_else(|| tr("1 个文件夹", "1 folder").into())
        } else {
            crate::localized_format!("{} 个文件夹", "{} folders", a.selected_folders.len()).into()
        };
        let mut results = div().flex().flex_col().gap(px(6.));
        for (i, r) in a.results.iter().enumerate() {
            results = results.child(div().id(("ai-result", i)).p(px(9.)).rounded(px(8.)).border_1().border_color(Theme::line_border())
                .bg(theme::raised(Theme::bg_body())).flex().items_start().gap(px(8.)).cursor_pointer().hover(|s| s.bg(Theme::fill_hover()))
                .child(ui::checkbox(r.selected))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.))
                    .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).child(r.front.clone()))
                    .child(div().text_size(px(11.)).line_height(px(16.)).text_color(Theme::text_caption()).child(r.back.clone())))
                .on_click(cx.listener(move |w, _, _, cx| { if let Some(a) = &mut w.review.ai && let Some(r) = a.results.get_mut(i) { r.selected = !r.selected; } cx.notify(); })));
        }
        let body = div().w_full().max_w(px(680.)).flex().flex_col().gap(px(10.))
            .child(div().flex().items_center().justify_between()
                .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(tr("AI 生成卡片", "AI cards")))
                .child(settings::link_button("ai-close", tr("关闭", "Close"))
                    .on_click(cx.listener(|w, _, _, cx| { w.review.ai = None; cx.notify(); }))))
            .child(div().text_size(px(11.)).line_height(px(16.)).text_color(Theme::text_caption())
                .child(tr("学习材料直接来自你选择的文档；可以同时选择一篇或多篇，生成后先预览再加入。", "Study material comes straight from the documents you pick; choose one or several, preview, then add.")))
            .child(div().flex().items_center().justify_between()
                .child(label(tr("选择文档", "Documents")))
                .child(settings::link_button("ai-select-all", if all_visible { tr("全不选", "Select none") } else { tr("全选", "Select all") })
                    .on_click(cx.listener(|w, _, _, cx| w.ai_toggle_all_docs(cx)))))
            .child(div().flex().items_center().gap(px(6.))
                .child(div().flex_1().min_w_0().h(px(28.)).px(px(7.)).flex().items_center().gap(px(6.)).rounded(px(7.)).bg(Theme::fill_hover())
                    .child(icon("search", 13., Theme::icon()))
                    .child(div().flex_1().min_w_0().child(a.search.clone())))
                .when(!a.folders.is_empty(), |d| d.child(
                    div().id("ai-folder-select").h(px(28.)).px(px(8.)).flex_none().flex().items_center().gap(px(4.)).rounded(px(7.))
                        .border_1().border_color(Theme::line_border()).bg(theme::raised(Theme::bg_body())).cursor_pointer()
                        .text_size(px(11.)).text_color(Theme::text_caption())
                        .when(a.folder_menu.is_some(), |s| s.bg(Theme::fill_hover()))
                        .child(div().max_w(px(150.)).truncate().child(folder_label))
                        .child(icon("chevron-down", 11., Theme::text_caption()))
                        .on_click(cx.listener(|w, ev: &ClickEvent, _, cx| {
                            if let Some(a) = &mut w.review.ai {
                                a.folder_menu = if a.folder_menu.is_some() { None } else { Some(point(ev.position().x, ev.position().y + px(4.))) };
                            }
                            cx.notify();
                        })))))
            .child(doc_list)
            .child(div().flex().items_center().gap(px(8.))
                .child(div().flex_1().text_size(px(11.)).text_color(Theme::text_caption()).child(crate::localized_format!("已选 {} 篇文档", "{} documents selected", chosen)))
                .child(label(tr("卡片数量", "Cards")))
                .child(settings::button("ai-count-minus", "−").on_click(cx.listener(|w, _, _, cx| w.ai_count(-1, cx))))
                .child(div().w(px(32.)).flex().justify_center().text_size(px(theme::ui_font())).child(a.count.to_string()))
                .child(settings::button("ai-count-plus", "+").on_click(cx.listener(|w, _, _, cx| w.ai_count(1, cx)))))
            .child(div().flex().items_center().gap(px(6.))
                .child(div().flex_1().text_size(px(10.)).line_height(px(15.)).text_color(Theme::text_placeholder())
                    .child(tr("接口、模型和密钥在 设置 › 复习 中配置；密钥只保存在本机。", "Endpoint, model and key live in Settings › Review; the key stays on this computer.")))
                .child(settings::link_button("ai-open-settings", tr("AI 设置", "AI settings"))
                    .on_click(cx.listener(|w, _, _, cx| w.open_settings(crate::settings::Page::Review, cx)))))
            .children(a.error.clone().map(|e| div().text_size(px(11.)).line_height(px(16.)).text_color(Theme::danger()).child(e)))
            .child(div().mt(px(4.)).flex().items_center().gap(px(8.))
                .child(div().flex_1())
                .child(if a.generating {
                    div().id("ai-generating").h(px(30.)).px(px(14.)).flex().items_center().gap(px(6.)).rounded(px(8.))
                        .bg(Theme::fill_hover_strong()).text_size(px(12.)).text_color(Theme::text_caption())
                        .child(icon("restore", 13., Theme::icon())).child(tr("正在生成…", "Generating…"))
                } else {
                    div().id("ai-generate").h(px(30.)).px(px(16.)).flex().items_center().rounded(px(8.)).cursor_pointer()
                        .bg(Theme::primary()).text_color(gpui::white()).text_size(px(12.)).hover(|s| s.opacity(0.9))
                        .child(tr("生成卡片", "Generate cards"))
                        .on_click(cx.listener(|w, _, window, cx| w.ai_generate(window, cx)))
                }))
            .when(!a.results.is_empty(), |d| d
                .child(div().mt(px(12.)).flex().items_center().justify_between()
                    .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(crate::localized_format!("生成结果（已选 {}）", "{} selected", selected)))
                    .child(div().flex().gap(px(6.))
                        .child(settings::link_button("ai-add-sel", tr("添加选中", "Add selected"))
                            .on_click(cx.listener(|w, _, _, cx| w.ai_add(false, cx))))
                        .child(settings::button("ai-add-all", tr("全部添加", "Add all"))
                            .on_click(cx.listener(|w, _, _, cx| w.ai_add(true, cx))))))
                .child(results));
        let mut page = div().id("rev-ai-scroll").flex_1().min_w_0().h_full().overflow_y_scroll().px(px(28.)).py(px(22.)).flex().flex_col().items_center()
            .child(body);
        if let Some(pos) = a.folder_menu {
            let mut menu = ui::popover().id("ai-folder-menu").occlude().min_w(px(220.)).py(px(crate::ui::MENU_PAD_Y))
                .on_mouse_down_out(cx.listener(|w, _, _, cx| { if let Some(a) = &mut w.review.ai { a.folder_menu = None; } cx.notify(); }))
                .child(ui::menu_row("ai-folder-all", "ai-folder-menu".into())
                    .child(ui::checkbox(a.selected_folders.is_empty()))
                    .child(tr("全部文件夹", "All folders"))
                    .on_click(cx.listener(|w, _, _, cx| {
                        if let Some(a) = &mut w.review.ai { a.selected_folders.clear(); a.folder_menu = None; }
                        cx.notify();
                    })));
            for (i, (id, name)) in a.folders.iter().enumerate() {
                let fid = id.clone();
                let on = a.selected_folders.contains(id);
                menu = menu.child(ui::menu_row(("ai-folder-opt", i), "ai-folder-menu".into())
                    .child(ui::checkbox(on))
                    .child(name.clone())
                    .on_click(cx.listener(move |w, _, _, cx| {
                        if let Some(a) = &mut w.review.ai {
                            if let Some(k) = a.selected_folders.iter().position(|f| f == &fid) { a.selected_folders.remove(k); }
                            else { a.selected_folders.push(fid.clone()); }
                        }
                        cx.notify();
                    })));
            }
            page = page.child(deferred(anchored().anchor(Anchor::TopLeft).position(pos).snap_to_window_with_margin(px(8.)).child(menu)).with_priority(2));
        }
        page.into_any_element()
    }

    fn ai_toggle_doc(&mut self, i: usize, cx: &mut Context<Self>) {
        if let Some(a) = &mut self.review.ai && let Some(d) = a.docs.get_mut(i) { d.selected = !d.selected; cx.notify(); }
    }

    fn ai_toggle_all_docs(&mut self, cx: &mut Context<Self>) {
        if let Some(a) = &mut self.review.ai {
            let query = a.search.read(cx).text.trim().to_lowercase();
            let picks = a.selected_folders.clone();
            let visible: Vec<usize> = a.docs.iter().enumerate()
                .filter(|(_, d)| picks.is_empty() || d.folders.iter().any(|x| picks.contains(x)))
                .filter(|(_, d)| query.is_empty() || d.title.to_lowercase().contains(&query))
                .map(|(i, _)| i).collect();
            let all = !visible.is_empty() && visible.iter().all(|i| a.docs[*i].selected);
            for i in visible { a.docs[i].selected = !all; }
            cx.notify();
        }
    }

    fn ai_count(&mut self, delta: i32, cx: &mut Context<Self>) {
        if let Some(a) = &mut self.review.ai { a.count = (a.count as i32 + delta).clamp(1, 20) as u32; cx.notify(); }
    }

    fn ai_generate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(a) = &self.review.ai else { return };
        if a.generating { return }
        let ids: Vec<String> = a.docs.iter().filter(|d| d.selected).map(|d| d.id.clone()).collect();
        let count = a.count;
        let p = settings::prefs();
        let base = p.ai_base_url.trim().to_string();
        let model = p.ai_model.trim().to_string();
        let key = load_ai_key().trim().to_string();
        let error = if ids.is_empty() {
            Some(tr("请先选择至少一篇文档", "Choose at least one document").to_string())
        } else if base.is_empty() || model.is_empty() || key.is_empty() {
            Some(tr("请先在 设置 › 复习 中配置接口、模型和 API 密钥", "Configure the endpoint, model and API key in Settings › Review first").to_string())
        } else { None };
        if let Some(e) = error {
            self.review.ai.as_mut().unwrap().error = Some(e.into());
            return cx.notify();
        }
        let mut parts = Vec::new();
        for id in &ids {
            let loaded = if self.current.as_deref() == Some(id.as_str()) {
                self.editor.as_ref().map(|ed| {
                    let ed = ed.read(cx);
                    (crate::settings::display_title(&ed.core.doc.title.text), ed.core.doc.plain_text())
                })
            } else {
                self.store.load(id).ok().map(|d| (crate::settings::display_title(&d.title.text), d.plain_text()))
            };
            if let Some((title, body)) = loaded {
                let body: String = body.chars().take(12_000).collect();
                if !body.trim().is_empty() { parts.push(format!("# {title}\n\n{body}")); }
            }
        }
        if parts.is_empty() {
            self.review.ai.as_mut().unwrap().error = Some(tr("所选文档没有可用的正文", "The selected documents have no body text").into());
            return cx.notify();
        }
        let source = parts.join("\n\n---\n\n");
        self.review.ai.as_mut().unwrap().generating = true;
        self.review.ai.as_mut().unwrap().error = None;
        self.review.ai.as_mut().unwrap().results.clear();
        cx.notify();
        let work = cx.background_executor().spawn(async move { generate_ai(&base, &key, &model, &source, count) });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |w, _, cx| {
                if let Some(a) = &mut w.review.ai {
                    a.generating = false;
                    match result {
                        Ok(cards) if cards.is_empty() => a.error = Some(tr("没有生成可用的卡片，请补充材料或换一个模型", "No usable cards came back; add material or try another model").into()),
                        Ok(cards) => a.results = cards.into_iter().take(count as usize).map(|(front, back)| AiDraft { front, back, selected: true }).collect(),
                        Err(e) => a.error = Some(e.into()),
                    }
                }
                cx.notify();
            });
        }).detach();
    }

    fn ai_add(&mut self, all: bool, cx: &mut Context<Self>) {
        let Some(a) = &self.review.ai else { return };
        let picked: Vec<AiDraft> = a.results.iter().filter(|r| all || r.selected).cloned().collect();
        if picked.is_empty() {
            self.show_toast(tr("没有选中的卡片", "No cards selected"), cx);
            return;
        }
        let at = now();
        let mut added = 0;
        for r in picked {
            let (front, back) = (r.front.trim().to_string(), r.back.trim().to_string());
            if front.is_empty() || back.is_empty() { continue }
            self.review.cards.push(Card {
                id: uuid::Uuid::new_v4().to_string(), front, back, created: at, due: at,
                interval: 0, ease: 2.5, reps: 0, lapses: 0, state: State::New, last: 0,
            });
            added += 1;
        }
        if added == 0 {
            self.show_toast(tr("没有可添加的卡片", "Nothing to add"), cx);
            return;
        }
        self.review.cards.sort_by_key(|c| (c.state == State::New, c.due, c.front.clone()));
        if let Err(e) = save(&self.review.cards, &self.review.log) {
            self.show_toast_kind(ui::ToastKind::Error, &format!("{}{e}", tr("保存失败：", "Could not save: ")), cx);
            return;
        }
        self.review.ai = None;
        self.show_toast(&crate::localized_format!("已添加 {} 张卡片", "Added {} cards", added), cx);
        cx.notify();
    }

    // ---- actions
    pub(crate) fn start_review(&mut self, cx: &mut Context<Self>) {
        if self.review.cards.is_empty() {
            self.show_toast(tr("还没有卡片", "No cards yet"), cx);
            return;
        }
        let at = now();
        let p = settings::prefs();
        let new_budget = (p.review_new_per_day as usize).saturating_sub(self.review.new_today(at) as usize);
        let review_room = (p.review_max_per_day as usize).saturating_sub(self.review.reviewed_today(at));
        let mut due: Vec<Card> = self.review.cards.iter().filter(|c| c.state != State::New && c.due <= at).cloned().collect();
        due.sort_by_key(|c| c.due);
        let mut new: Vec<Card> = self.review.cards.iter().filter(|c| c.state == State::New).cloned().collect();
        new.sort_by_key(|c| c.created);
        let mut queue: Vec<String> = due.into_iter().map(|c| c.id).collect();
        queue.extend(new.into_iter().take(new_budget).map(|c| c.id));
        queue.truncate(review_room);
        if queue.is_empty() {
            self.show_toast(tr("现在没有需要复习的卡片", "Nothing is due right now"), cx);
            return;
        }
        self.review.draft = None;
        self.review.session = Some(Session { queue, at: 0, revealed: false, completed: 0 });
        cx.notify();
    }

    fn rate_review(&mut self, rating: Rating, cx: &mut Context<Self>) {
        let Some(id) = self.review.session.as_ref().and_then(|s| s.queue.get(s.at).cloned()) else { return };
        let at = now();
        let was_new = self.review.cards.iter().find(|c| c.id == id).is_some_and(|c| c.state == State::New);
        if let Some(card) = self.review.cards.iter_mut().find(|c| c.id == id) {
            let interval = schedule(card, rating, at);
            self.review.log.push(Log { time: at, card: id, rating: rating as u8, interval, was_new });
        }
        if let Err(e) = save(&self.review.cards, &self.review.log) {
            self.show_toast_kind(ui::ToastKind::Error, &format!("{}{e}", tr("保存失败：", "Could not save: ")), cx);
        }
        if let Some(s) = &mut self.review.session {
            s.completed += 1;
            s.at += 1;
            s.revealed = false;
        }
        cx.notify();
    }

    fn open_card(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let card = id.as_ref().and_then(|id| self.review.cards.iter().find(|c| &c.id == id)).cloned();
        let front_text = card.as_ref().map(|c| c.front.clone()).unwrap_or_default();
        let back_text = card.as_ref().map(|c| c.back.clone()).unwrap_or_default();
        let front = cx.new(|cx| {
            let mut f = TextField::new(tr("问题", "Question"), cx);
            f.size = 14.;
            f.line_height = 22.;
            f.multiline = true;
            f.wrap = true;
            f.set_text(&front_text, cx);
            f
        });
        let back = cx.new(|cx| {
            let mut f = TextField::new(tr("答案", "Answer"), cx);
            f.size = 13.;
            f.line_height = 21.;
            f.multiline = true;
            f.wrap = true;
            f.set_text(&back_text, cx);
            f
        });
        let mut subs = Vec::new();
        for field in [&front, &back] {
            subs.push(cx.subscribe(field, |w: &mut Workspace, _, ev: &TextFieldEvent, cx| {
                if *ev == TextFieldEvent::Cancel {
                    w.review.draft = None;
                    cx.notify();
                }
            }));
        }
        front.update(cx, |f, cx| f.select_all_text(cx));
        self.review.draft = Some(Draft { id: card.map(|c| c.id), front: front.clone(), back, armed: false, error: None });
        self.review._subs = subs;
        window.focus(&front.focus_handle(cx), cx);
        cx.notify();
    }

    fn commit_card(&mut self, cx: &mut Context<Self>) {
        let Some(d) = &self.review.draft else { return };
        let front = d.front.read(cx).text.trim().to_string();
        let back = d.back.read(cx).text.trim().to_string();
        if front.is_empty() || back.is_empty() {
            self.review.draft.as_mut().unwrap().error = Some(tr("正面和背面都不能为空", "Front and back are both required").into());
            return cx.notify();
        }
        let at = now();
        match &d.id {
            Some(id) => {
                if let Some(card) = self.review.cards.iter_mut().find(|c| &c.id == id) {
                    card.front = front;
                    card.back = back;
                }
            }
            None => self.review.cards.push(Card {
                id: uuid::Uuid::new_v4().to_string(), front, back, created: at, due: at,
                interval: 0, ease: 2.5, reps: 0, lapses: 0, state: State::New, last: 0,
            }),
        }
        self.review.cards.sort_by_key(|c| (c.state == State::New, c.due, c.front.clone()));
        if let Err(e) = save(&self.review.cards, &self.review.log) {
            self.review.draft.as_mut().unwrap().error = Some(format!("{}{e}", tr("保存失败：", "Could not save: ")).into());
            return cx.notify();
        }
        self.review.draft = None;
        cx.notify();
    }

    fn delete_card(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.review.draft.as_mut() else { return };
        if !d.armed { d.armed = true; return cx.notify() }
        let Some(id) = d.id.clone() else { return };
        self.review.cards.retain(|c| c.id != id);
        self.review.log.retain(|l| l.card != id);
        if let Err(e) = save(&self.review.cards, &self.review.log) {
            self.review.draft.as_mut().unwrap().error = Some(format!("{}{e}", tr("删除失败：", "Could not delete: ")).into());
            return cx.notify();
        }
        self.review.draft = None;
        cx.notify();
    }

    /// 设置 › 复习: the daily numbers step by fives and tens.
    pub(crate) fn change_review_new(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.set_pref(|p| p.review_new_per_day = (p.review_new_per_day as i32 + delta * 5).clamp(0, 200) as u32, cx);
    }

    pub(crate) fn change_review_max(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.set_pref(|p| p.review_max_per_day = (p.review_max_per_day as i32 + delta * 25).clamp(25, 2000) as u32, cx);
    }
}

fn ai_text_of(value: &serde_json::Value) -> String {
    if let Some(s) = value.as_str() { return s.trim().to_string() }
    if let Some(parts) = value.as_array() {
        return parts.iter().filter_map(|p| p.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join("\n").trim().to_string();
    }
    String::new()
}

fn ai_parse_cards(content: &str, limit: usize) -> Result<Vec<(String, String)>, String> {
    let mut text = content.trim().to_string();
    if let Some(i) = text.find("```") {
        let after = text[i + 3..].trim_start_matches("json").trim_start_matches("JSON");
        if let Some(j) = after.rfind("```") { text = after[..j].trim().to_string(); }
    }
    while let Some(i) = text.find(" thinking") {
        let Some(j) = text[i..].find("</think>") else { break };
        text.replace_range(i..i + j + 8, "");
    }
    let value = serde_json::from_str::<serde_json::Value>(&text).ok().or_else(|| {
        let start = text.find('{')?;
        let end = text.rfind('}')?;
        serde_json::from_str(&text[start..=end]).ok()
    }).ok_or_else(|| "AI 返回内容无法解析为卡片".to_string())?;
    let cards = value.get("cards").and_then(|c| c.as_array()).or_else(|| value.as_array()).ok_or_else(|| "AI 返回中没有 cards".to_string())?;
    let mut out = Vec::new();
    for card in cards {
        let mut front = card.get("front").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let mut back = card.get("back").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if (front.is_empty() || back.is_empty()) && let Some(content) = card.get("content").and_then(|v| v.as_str())
            && let Some((f, b)) = content.split_once("---div---") {
            front = f.trim().to_string();
            back = b.trim().to_string();
        }
        if front.is_empty() || back.is_empty() { continue }
        out.push((front, back));
        if out.len() >= limit { break }
    }
    if out.is_empty() { return Err("AI 没有返回可用的问答卡片".into()) }
    Ok(out)
}

#[cfg(not(target_family = "wasm"))]
fn generate_ai(base: &str, key: &str, model: &str, source: &str, count: u32) -> Result<Vec<(String, String)>, String> {
    use std::io::Write;
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() { return Err("接口地址为空".into()) }
    let url = if base.ends_with("/chat/completions") { base.to_string() } else { format!("{base}/chat/completions") };
    let system = format!(
        "你是 QuietNext 的认知制卡引擎。把用户提供的笔记材料拆成适合间隔重复的原子问答卡片。\n\
         规则：一张卡片只考一个事实；问题短而明确；答案简洁；优先核心定义、机制、易错点；不要编造材料中没有的事实；材料不足时少出卡。\n\
         生成不超过 {count} 张卡片，使用材料本身的语言。\n\
         只输出 JSON，不要 Markdown 代码块或解释：{{\"cards\":[{{\"front\":\"问题\",\"back\":\"答案\"}}]}}"
    );
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": format!("学习材料：\n\n{source}")}
        ],
        "temperature": 0.4,
        "max_tokens": 1200 + count as i64 * 250
    }).to_string();
    let header = std::env::temp_dir().join(format!("quiet-next-ai-{}.hdr", uuid::Uuid::new_v4()));
    std::fs::write(&header, format!("Authorization: Bearer {key}\n")).map_err(|e| format!("写入请求头失败：{e}"))?;
    let header_arg = format!("@{}", header.display());
    let mut cmd = std::process::Command::new("curl");
    cmd.arg("-sS").arg("--max-time").arg("120").arg("-X").arg("POST").arg(&url)
        .arg("-H").arg("Content-Type: application/json")
        .arg("-H").arg(&header_arg)
        .arg("--data-binary").arg("@-")
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    #[cfg(target_os = "windows")]
    { use std::os::windows::process::CommandExt; cmd.creation_flags(0x0800_0000); }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => { let _ = std::fs::remove_file(&header); return Err(format!("无法启动 curl：{e}")) }
    };
    if let Some(mut stdin) = child.stdin.take() && let Err(e) = stdin.write_all(body.as_bytes()) {
        let _ = std::fs::remove_file(&header);
        return Err(format!("发送请求失败：{e}"));
    }
    let output = child.wait_with_output().map_err(|e| format!("请求失败：{e}"))?;
    let _ = std::fs::remove_file(&header);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() { "AI 请求失败".into() } else { stderr });
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("AI 返回的不是 JSON：{e}\n{}", String::from_utf8_lossy(&output.stdout).chars().take(300).collect::<String>()))?;
    if let Some(message) = value.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()) { return Err(message.to_string()) }
    let content = value.get("choices").and_then(|c| c.get(0)).and_then(|c| c.get("message"))
        .and_then(|m| m.get("content")).map(ai_text_of).unwrap_or_default();
    ai_parse_cards(&content, count as usize)
}

#[cfg(target_family = "wasm")]
fn generate_ai(_base: &str, _key: &str, _model: &str, _source: &str, _count: u32) -> Result<Vec<(String, String)>, String> {
    Err("网页版暂不支持 AI 生成".into())
}

#[cfg(test)]
mod tests {
    use super::ai_parse_cards;

    #[test]
    fn parses_ai_cards() {
        let plain = ai_parse_cards(r#"{"cards":[{"front":"问","back":"答"}]}"#, 5).unwrap();
        assert_eq!(plain, vec![("问".to_string(), "答".to_string())]);
        let fenced = ai_parse_cards("```json\n{\"cards\":[{\"content\":\"问题\\n\\n---div---\\n\\n答案\"}]}\n```", 5).unwrap();
        assert_eq!(fenced, vec![("问题".to_string(), "答案".to_string())]);
        let thinking = ai_parse_cards(" thinkinghidden</think>{\"cards\":[{\"front\":\"A\",\"back\":\"B\"},{\"front\":\"\",\"back\":\"x\"}]}", 5).unwrap();
        assert_eq!(thinking.len(), 1);
        assert!(ai_parse_cards("{}", 5).is_err());
    }

    #[test]
    #[ignore = "needs curl and a loopback socket"]
    fn ai_generation_reads_an_openai_response() {
        use std::io::{Read as _, Write as _};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = vec![0u8; 16384];
            let n = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            assert!(request.contains("Authorization: Bearer test-key"), "{request}");
            let body = r#"{"choices":[{"message":{"content":"{\"cards\":[{\"front\":\"Q\",\"back\":\"A\"}]}"}}]}"#;
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).unwrap();
        });
        let cards = super::generate_ai(&format!("http://127.0.0.1:{port}/v1"), "test-key", "test-model", "lesson", 2).unwrap();
        assert_eq!(cards, vec![("Q".to_string(), "A".to_string())]);
        server.join().unwrap();
    }
}
