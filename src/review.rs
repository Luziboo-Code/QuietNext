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

pub struct Review {
    cards: Vec<Card>,
    log: Vec<Log>,
    draft: Option<Draft>,
    session: Option<Session>,
    filter: Filter,
    _subs: Vec<Subscription>,
}

fn path() -> PathBuf { crate::store::Store::internal_dir().join("review.json") }

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
        Self { cards: store.cards, log: store.log, draft: None, session: None, filter: Filter::All, _subs: Vec::new() }
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
            .child(div().mr(px(8.)).child(new_card)).child(session);
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
        let progress = div().flex_none().px(px(28.)).pt(px(18.)).flex().items_center().gap(px(12.))
            .child(div().flex_1().h(px(4.)).rounded(px(2.)).bg(Theme::fill_hover_strong())
                .child(div().h_full().rounded(px(2.)).bg(Theme::primary()).w(relative(if total == 0 { 0. } else { s.at as f32 / total as f32 }))))
            .child(div().text_size(px(11.)).text_color(Theme::text_caption()).child(format!("{} / {total}", s.at.min(total))));
        if done || card.is_none() {
            return div().flex_1().min_w_0().h_full().flex().flex_col().child(progress)
                .child(div().flex_1().flex().flex_col().items_center().justify_center().gap(px(10.))
                    .child(div().size(px(52.)).rounded_full().bg(theme::ca(0x0f7b6c, 0.12)).flex().items_center().justify_center().child(icon("todo-check", 26., theme::c(0x0f7b6c))))
                    .child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).child(tr("今天的复习完成了", "All caught up")))
                    .child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(crate::localized_format!("复习了 {} 张卡片", "Reviewed {} cards", s.completed)))
                    .child(div().id("rev-done").mt(px(8.)).h(px(30.)).px(px(16.)).flex().items_center().rounded(px(8.)).cursor_pointer()
                        .bg(Theme::primary()).text_color(gpui::white()).text_size(px(12.)).hover(|s| s.opacity(0.9)).child(tr("完成", "Done"))
                        .on_click(cx.listener(|w, _, _, cx| { w.review.session = None; cx.notify(); }))))
                .into_any_element();
        }
        let card = card.unwrap();
        let front = div().text_size(px(19.)).line_height(px(30.)).child(card.front.clone());
        let mut panel = div().w_full().min_h(px(220.)).p(px(24.)).rounded(px(14.)).border_1().border_color(Theme::line_divider())
            .bg(theme::raised(Theme::bg_body())).flex().flex_col().gap(px(14.))
            .child(div().text_size(px(10.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text_placeholder()).child(tr("问题", "Question")))
            .child(front);
        if s.revealed {
            panel = panel
                .child(div().h(px(1.)).bg(Theme::line_divider()))
                .child(div().text_size(px(10.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text_placeholder()).child(tr("答案", "Answer")))
                .child(div().text_size(px(17.)).line_height(px(27.)).child(card.back.clone()));
        }
        let actions = if s.revealed {
            let colors = [(Rating::Again, tr("重来", "Again"), 0xd14345), (Rating::Hard, tr("困难", "Hard"), 0xd9730d), (Rating::Good, tr("良好", "Good"), 0x2383e2), (Rating::Easy, tr("简单", "Easy"), 0x0f7b6c)];
            div().w_full().flex().gap(px(8.)).children(colors.map(|(rating, label, color)| {
                div().id(("rev-rate", rating as usize)).flex_1().h(px(54.)).rounded(px(10.)).cursor_pointer()
                    .bg(theme::ca(color, 0.12)).hover(|s| s.bg(theme::ca(color, 0.2)))
                    .flex().flex_col().items_center().justify_center().gap(px(1.))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(theme::c(color)).child(label))
                    .child(div().text_size(px(10.)).text_color(Theme::text_caption()).child(preview(&card, rating)))
                    .on_click(cx.listener(move |w, _, _, cx| w.rate_review(rating, cx)))
            }))
        } else {
            div().w_full().flex().justify_center().child(
                div().id("rev-reveal").h(px(34.)).px(px(22.)).flex().items_center().rounded(px(9.)).cursor_pointer()
                    .bg(Theme::primary()).text_color(gpui::white()).text_size(px(13.)).hover(|s| s.opacity(0.9)).child(tr("显示答案", "Show answer"))
                    .on_click(cx.listener(|w, _, _, cx| { if let Some(s) = &mut w.review.session { s.revealed = true; } cx.notify(); })))
        };
        div().flex_1().min_w_0().h_full().flex().flex_col().child(progress)
            .child(div().id("rev-session-scroll").flex_1().min_h_0().overflow_y_scroll().px(px(28.)).py(px(22.)).flex().flex_col().items_center()
                .child(div().w_full().max_w(px(600.)).flex().flex_col().gap(px(14.)).child(panel).child(actions)))
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
