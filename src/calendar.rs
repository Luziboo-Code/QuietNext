//! 日历: a local calendar with Notion Calendar's month, week and day
//! views. Events live in `.quiet/calendar.json`; nothing leaves the library.
//! The page keeps the app's own chrome: a left mini month and calendar list,
//! the grid in the middle, and an event editor on the right.

use crate::app::Workspace;
use crate::input::{TextField, TextFieldEvent};
use crate::settings::{self, tr};
use crate::theme::{self, craft, Theme};
use crate::ui::{self, icon, TitleBarExt};
use gpui::{prelude::FluentBuilder, *};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const PANEL_W: f32 = 224.;
const DETAIL_W: f32 = 288.;
const HOUR_H: f32 = 44.;
const MINI_CELL: f32 = 26.;
const CHIP_H: f32 = 18.;
const EVENT_COLORS: [u32; 6] = [0x2383e2, 0x0f7b6c, 0x6940a5, 0xd9730d, 0xe03e3e, 0xad1a72];

/// Sunday first, as Notion Calendar shows it.
const WEEK: [(&str, &str); 7] = [("日", "Sun"), ("一", "Mon"), ("二", "Tue"), ("三", "Wed"), ("四", "Thu"), ("五", "Fri"), ("六", "Sat")];

fn path() -> PathBuf {
    crate::store::Store::internal_dir().join("calendar.json")
}

fn load_events() -> Vec<Event> {
    fdl_core::vfs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Atomic write like `settings::save`: temp file, fsync, rename.
fn save_events(events: &[Event]) -> anyhow::Result<()> {
    use std::io::Write;
    let dir = crate::store::Store::internal_dir();
    fdl_core::vfs::create_dir_all(&dir)?;
    let tmp = dir.join("calendar.json.tmp");
    let mut f = fdl_core::vfs::File::create(&tmp)?;
    f.write_all(&serde_json::to_vec_pretty(events)?)?;
    f.sync_all()?;
    fdl_core::vfs::rename(tmp, path())?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub title: String,
    /// Local unix seconds.
    pub start: i64,
    pub end: i64,
    #[serde(default)]
    pub all_day: bool,
    #[serde(default)]
    pub color: u8,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Month,
    Week,
    Day,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Filter {
    All,
    Open,
    Done,
}

pub struct Draft {
    id: Option<String>,
    day: i64,
    title: Entity<TextField>,
    start_field: Entity<TextField>,
    end_field: Entity<TextField>,
    content: Entity<TextField>,
    notes: Entity<TextField>,
    all_day: bool,
    done: bool,
    color: u8,
    error: Option<SharedString>,
}

pub struct Calendar {
    events: Vec<Event>,
    mode: Mode,
    /// Year and month of the month view and the mini calendar.
    cursor: (i64, u32),
    /// The day the right panel and the week view are built around.
    selected: i64,
    draft: Option<Draft>,
    show_events: bool,
    /// 全部 / 未完成 / 已完成.
    filter: Filter,
    /// The filter list covers the whole library instead of the shown month.
    filter_all_months: bool,
    _subs: Vec<Subscription>,
}

impl Default for Calendar {
    fn default() -> Self {
        let today = today_day();
        let (y, m, _) = crate::time::civil_from_days(today);
        Self { events: load_events(), mode: Mode::Month, cursor: (y, m), selected: today, draft: None, show_events: true, filter: Filter::All, filter_all_months: false, _subs: Vec::new() }
    }
}

fn today_day() -> i64 {
    let (y, m, d, _, _) = crate::time::civil(crate::time::now());
    crate::time::days_from_civil(y, m, d)
}

/// Sunday = 0 … Saturday = 6.
fn weekday_sun(day: i64) -> i64 { (day + 4).rem_euclid(7) }

fn day_of(secs: i64) -> i64 {
    let (y, m, d, _, _) = crate::time::civil(secs.max(0) as u64);
    crate::time::days_from_civil(y, m, d)
}

fn add_months((y, m): (i64, u32), delta: i64) -> (i64, u32) {
    let k = y * 12 + i64::from(m) - 1 + delta;
    (k.div_euclid(12), k.rem_euclid(12) as u32 + 1)
}

fn month_of(day: i64) -> (i64, u32) {
    let (y, m, _) = crate::time::civil_from_days(day);
    (y, m)
}

fn date_label(day: i64) -> String {
    let (y, m, d) = crate::time::civil_from_days(day);
    crate::localized_format!("{y}年 {m}月 {d}日", "{y}-{m:02}-{d:02}")
}

fn parse_hm(text: &str) -> Option<u32> {
    let (h, m) = text.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

fn hm(min: u32) -> String { format!("{:02}:{:02}", min / 60, min % 60) }

/// Minutes after local midnight of a unix timestamp.
fn minute_of_day(secs: i64) -> u32 { ((secs + crate::time::local_offset()).rem_euclid(86_400) / 60) as u32 }

fn event_color(index: u8) -> Hsla { theme::c(EVENT_COLORS[index as usize % EVENT_COLORS.len()]) }
fn event_tint(index: u8) -> Hsla { theme::ca(EVENT_COLORS[index as usize % EVENT_COLORS.len()], 0.16) }

/// Lunar month lengths and leap months, 1900–2100 (bit 0–3: leap month,
/// bit 16: leap month has 30 days, bits 15–4: the 12 months' lengths).
const LUNAR_INFO: [u32; 201] = [
    0x04bd8, 0x04ae0, 0x0a570, 0x054d5, 0x0d260, 0x0d950, 0x16554, 0x056a0, 0x09ad0, 0x055d2,
    0x04ae0, 0x0a5b6, 0x0a4d0, 0x0d250, 0x1d255, 0x0b540, 0x0d6a0, 0x0ada2, 0x095b0, 0x14977,
    0x04970, 0x0a4b0, 0x0b4b5, 0x06a50, 0x06d40, 0x1ab54, 0x02b60, 0x09570, 0x052f2, 0x04970,
    0x06566, 0x0d4a0, 0x0ea50, 0x06e95, 0x05ad0, 0x02b60, 0x186e3, 0x092e0, 0x1c8d7, 0x0c950,
    0x0d4a0, 0x1d8a6, 0x0b550, 0x056a0, 0x1a5b4, 0x025d0, 0x092d0, 0x0d2b2, 0x0a950, 0x0b557,
    0x06ca0, 0x0b550, 0x15355, 0x04da0, 0x0a5b0, 0x14573, 0x052b0, 0x0a9a8, 0x0e950, 0x06aa0,
    0x0aea6, 0x0ab50, 0x04b60, 0x0aae4, 0x0a570, 0x05260, 0x0f263, 0x0d950, 0x05b57, 0x056a0,
    0x096d0, 0x04dd5, 0x04ad0, 0x0a4d0, 0x0d4d4, 0x0d250, 0x0d558, 0x0b540, 0x0b6a0, 0x195a6,
    0x095b0, 0x049b0, 0x0a974, 0x0a4b0, 0x0b27a, 0x06a50, 0x06d40, 0x0af46, 0x0ab60, 0x09570,
    0x04af5, 0x04970, 0x064b0, 0x074a3, 0x0ea50, 0x06b58, 0x05ac0, 0x0ab60, 0x096d5, 0x092e0,
    0x0c960, 0x0d954, 0x0d4a0, 0x0da50, 0x07552, 0x056a0, 0x0abb7, 0x025d0, 0x092d0, 0x0cab5,
    0x0a950, 0x0b4a0, 0x0baa4, 0x0ad50, 0x055d9, 0x04ba0, 0x0a5b0, 0x15176, 0x052b0, 0x0a930,
    0x07954, 0x06aa0, 0x0ad50, 0x05b52, 0x04b60, 0x0a6e6, 0x0a4e0, 0x0d260, 0x0ea65, 0x0d530,
    0x05aa0, 0x076a3, 0x096d0, 0x04afb, 0x04ad0, 0x0a4d0, 0x1d0b6, 0x0d250, 0x0d520, 0x0dd45,
    0x0b5a0, 0x056d0, 0x055b2, 0x049b0, 0x0a577, 0x0a4b0, 0x0aa50, 0x1b255, 0x06d20, 0x0ada0,
    0x14b63, 0x09370, 0x049f8, 0x04970, 0x064b0, 0x168a6, 0x0ea50, 0x06b20, 0x1a6c4, 0x0aae0,
    0x0a2e0, 0x0d2e3, 0x0c960, 0x0d557, 0x0d4a0, 0x0da50, 0x05d55, 0x056a0, 0x0a6d0, 0x055d4,
    0x052d0, 0x0a9b8, 0x0a950, 0x0b4a0, 0x0b6a6, 0x0ad50, 0x055a0, 0x0aba4, 0x0a5b0, 0x052b0,
    0x0b273, 0x06930, 0x07337, 0x06aa0, 0x0ad50, 0x14b55, 0x04b60, 0x0a570, 0x054e4, 0x0d160,
    0x0e968, 0x0d520, 0x0daa0, 0x16aa6, 0x056d0, 0x04ae0, 0x0a9d4, 0x0a2d0, 0x0d150, 0x0f252,
    0x0d520,
];

fn lunar_leap_month(year: i64) -> u32 { LUNAR_INFO[(year - 1900) as usize] & 0xf }

fn lunar_leap_days(year: i64) -> i64 {
    if lunar_leap_month(year) != 0 && LUNAR_INFO[(year - 1900) as usize] & 0x10000 != 0 { 30 } else if lunar_leap_month(year) != 0 { 29 } else { 0 }
}

fn lunar_month_days(year: i64, month: u32) -> i64 {
    if LUNAR_INFO[(year - 1900) as usize] & (0x10000 >> month) != 0 { 30 } else { 29 }
}

fn lunar_year_days(year: i64) -> i64 {
    let mut sum = 348;
    let mut bit = 0x8000;
    while bit > 0x8 {
        if LUNAR_INFO[(year - 1900) as usize] & bit != 0 { sum += 1; }
        bit >>= 1;
    }
    sum + lunar_leap_days(year)
}

/// Lunar date of a day number: year, month, day, and whether it is a leap month.
fn lunar_of(day: i64) -> Option<(i64, u32, u32, bool)> {
    let mut offset = day - crate::time::days_from_civil(1900, 1, 31);
    if offset < 0 { return None }
    let mut year = 1900;
    loop {
        let days = lunar_year_days(year);
        if offset < days { break }
        offset -= days;
        year += 1;
        if year > 2100 { return None }
    }
    let leap = lunar_leap_month(year);
    let mut is_leap = false;
    let mut month = 1u32;
    while month < 13 && offset >= 0 {
        let days;
        if leap > 0 && month == leap + 1 && !is_leap {
            month -= 1;
            is_leap = true;
            days = lunar_leap_days(year);
        } else {
            days = lunar_month_days(year, month);
        }
        if is_leap && month == leap + 1 { is_leap = false; }
        offset -= days;
        if offset < 0 { offset += days; break }
        month += 1;
    }
    Some((year, month, (offset + 1) as u32, is_leap))
}

fn lunar_day_name(day: u32) -> String {
    const DIGITS: [&str; 11] = ["", "一", "二", "三", "四", "五", "六", "七", "八", "九", "十"];
    match day {
        10 => "初十".into(),
        20 => "二十".into(),
        30 => "三十".into(),
        _ => {
            let prefix = ["初", "十", "廿", "卅"][(day / 10).min(3) as usize];
            format!("{prefix}{}", DIGITS[(day % 10) as usize])
        }
    }
}

fn lunar_text(day: i64) -> String {
    let Some((_, month, date, leap)) = lunar_of(day) else { return String::new() };
    const MONTHS: [&str; 13] = ["", "正", "二", "三", "四", "五", "六", "七", "八", "九", "十", "冬", "腊"];
    format!("{}{}月{}", if leap { "闰" } else { "" }, MONTHS[(month as usize).min(12)], lunar_day_name(date))
}

fn qingming_day(year: i64) -> u32 {
    if !(2000..=2099).contains(&year) { return 5 }
    let y = (year % 100) as f64;
    ((y * 0.2422 + 4.81).floor() - (y / 4.0).floor()) as u32
}

fn holiday_of(day: i64) -> Option<&'static str> {
    let (year, month, date) = crate::time::civil_from_days(day);
    if month == 4 && date == qingming_day(year) { return Some("清明") }
    let solar = match (month, date) {
        (1, 1) => "元旦",
        (2, 14) => "情人节",
        (3, 8) => "妇女节",
        (5, 1) => "劳动节",
        (6, 1) => "儿童节",
        (9, 10) => "教师节",
        (10, 1) => "国庆节",
        (12, 25) => "圣诞节",
        _ => "",
    };
    if !solar.is_empty() { return Some(solar) }
    let (ly, lm, ld, leap) = lunar_of(day)?;
    if leap { return None }
    Some(match (lm, ld) {
        (1, 1) => "春节",
        (1, 15) => "元宵节",
        (5, 5) => "端午节",
        (7, 7) => "七夕",
        (8, 15) => "中秋节",
        (9, 9) => "重阳节",
        (12, 8) => "腊八节",
        (12, _) if ld == lunar_month_days(ly, 12) as u32 => "除夕",
        _ => return None,
    })
}

impl Calendar {
    pub fn draft_open(&self) -> bool { self.draft.is_some() }
    pub fn close_draft(&mut self) { self.draft = None; }
    /// Previews and screenshots: show this day's month.
    pub fn show_day(&mut self, day: i64) { self.selected = day; self.cursor = month_of(day); }

    fn events_for(&self, day: i64) -> Vec<Event> {
        let mut out: Vec<Event> = self.events.iter()
            .filter(|e| day_of(e.start) <= day && day <= day_of(e.end) && self.matches(e.done))
            .cloned().collect();
        out.sort_by_key(|e| (!e.all_day, e.start, e.end));
        out
    }

    fn matches(&self, done: bool) -> bool {
        match self.filter { Filter::All => true, Filter::Open => !done, Filter::Done => done }
    }

    /// The filter box's list: matching events for the shown month, or all of them.
    fn filtered_events(&self) -> Vec<Event> {
        let (vy, vm) = self.cursor;
        let mut out: Vec<Event> = self.events.iter()
            .filter(|e| self.matches(e.done))
            .filter(|e| self.filter_all_months || month_of(day_of(e.start)) == (vy, vm))
            .cloned().collect();
        out.sort_by_key(|e| (day_of(e.start), !e.all_day, e.start));
        out
    }

    fn next_event_start(&self, day: i64) -> u32 {
        self.events.iter().filter(|e| day_of(e.start) == day && !e.all_day)
            .map(|e| minute_of_day(e.start)).max().unwrap_or(9 * 60)
    }
}

impl Workspace {
    pub(crate) fn render_calendar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (y, m) = self.calendar.cursor;
        let title = div().text_size(px(20.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD)
            .child(crate::localized_format!("{y}年 {m}月", "{y}-{m:02}"));

        let nav = |key: &'static str, name: &'static str| {
            div().id(key).no_drag().size(px(28.)).flex().items_center().justify_center().rounded(px(8.)).cursor_pointer()
                .hover(|s| s.bg(Theme::fill_hover()))
                .child(icon(name, 16., Theme::icon()))
        };
        let prev = nav("cal-prev", "chevron-left").on_click(cx.listener(|w, _, _, cx| w.calendar_step(-1, cx)));
        let next = nav("cal-next", "chevron-right").on_click(cx.listener(|w, _, _, cx| w.calendar_step(1, cx)));
        let today = div().id("cal-today").no_drag().h(px(28.)).px(px(10.)).flex().items_center().rounded(px(8.)).cursor_pointer()
            .text_size(px(theme::ui_font())).line_height(px(20.)).text_color(Theme::text())
            .hover(|s| s.bg(Theme::fill_hover()))
            .child(tr("今天", "Today"))
            .on_click(cx.listener(|w, _, _, cx| { let d = today_day(); w.calendar.selected = d; w.calendar.cursor = month_of(d); w.calendar.draft = None; cx.notify(); }));
        let modes = self.calendar_mode_switch(cx);
        let header_inner = div().w_full().flex().items_center()
            .child(title)
            .child(div().ml(px(16.)).child(prev))
            .child(div().ml(px(2.)).child(next))
            .child(div().ml(px(6.)).child(today))
            .child(div().flex_1())
            .child(modes);
        let header = crate::trash::page_header_with(self, header_inner, 26., cx);

        let body = div().flex().flex_1().min_h_0()
            .child(self.calendar_sidebar(cx))
            .child(self.calendar_main(cx))
            .child(self.calendar_detail(window, cx));

        div().flex_1().min_w_0().h_full().flex().flex_col().bg(Theme::bg_body())
            .font_family(theme::ui_family()).text_color(Theme::text())
            .child(header).child(body).into_any_element()
    }

    fn calendar_mode_switch(&self, cx: &mut Context<Self>) -> Div {
        let modes = [(Mode::Month, tr("月", "Month")), (Mode::Week, tr("周", "Week")), (Mode::Day, tr("日", "Day"))];
        div().h(px(28.)).p(px(2.)).flex().gap(px(2.)).rounded(px(9.)).bg(Theme::fill_hover_strong())
            .children(modes.map(|(mode, label)| {
                let on = self.calendar.mode == mode;
                div().id(("cal-mode", mode as usize)).no_drag().h_full().px(px(12.)).flex().items_center().justify_center().rounded(px(7.)).cursor_pointer()
                    .text_size(px(theme::ui_font())).line_height(px(theme::ui_font() + 5.))
                    .when(on, |d| d.bg(if theme::dark() { theme::rgbaf(0x63, 0x63, 0x66, 1.) } else { Theme::bg_body() }).shadow(theme::shadow_button()).text_color(Theme::text()))
                    .when(!on, |d| d.text_color(Theme::text_caption()).hover(|s| s.text_color(Theme::text())))
                    .child(label)
                    .on_click(cx.listener(move |w, _, _, cx| w.set_calendar_mode(mode, cx)))
            }))
    }

    fn set_calendar_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.calendar.mode = mode;
        self.calendar.cursor = month_of(self.calendar.selected);
        self.calendar.draft = None;
        cx.notify();
    }

    fn calendar_step(&mut self, delta: i64, cx: &mut Context<Self>) {
        match self.calendar.mode {
            Mode::Month => self.calendar.cursor = add_months(self.calendar.cursor, delta),
            Mode::Week => {
                self.calendar.selected += delta * 7;
                self.calendar.cursor = month_of(self.calendar.selected);
            }
            Mode::Day => {
                self.calendar.selected += delta;
                self.calendar.cursor = month_of(self.calendar.selected);
            }
        }
        self.calendar.draft = None;
        cx.notify();
    }

    fn calendar_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let (vy, vm) = self.calendar.cursor;
        let today = today_day();
        let selected = self.calendar.selected;
        let first = crate::time::days_from_civil(vy, vm, 1);
        let start = first - weekday_sun(first);
        let mut rows = div().flex().flex_col().gap(px(2.));
        for row in 0..6 {
            let mut line = div().flex().gap(px(2.));
            for col in 0..7 {
                let day = start + row * 7 + col;
                let (y, m, d) = crate::time::civil_from_days(day);
                let inside = (y, m) == (vy, vm);
                let is_today = day == today;
                let is_sel = day == selected;
                line = line.child(div().id(("cal-mini", day as usize)).size(px(MINI_CELL)).flex().items_center().justify_center()
                    .rounded(px(6.)).text_size(px(11.)).cursor_pointer()
                    .text_color(if is_today { gpui::white() } else if is_sel { Theme::primary() } else if inside { Theme::text() } else { Theme::text_placeholder() })
                    .when(is_today, |x| x.bg(theme::c(0xeb5757)))
                    .when(is_sel && !is_today, |x| x.bg(Theme::fill_hover_strong()).font_weight(FontWeight::SEMIBOLD))
                    .when(!is_today && !is_sel, |x| x.hover(|s| s.bg(Theme::fill_hover())))
                    .child(d.to_string())
                    .on_click(cx.listener(move |w, _, _, cx| { w.calendar.selected = day; w.calendar.cursor = month_of(day); w.calendar.draft = None; cx.notify(); })));
            }
            rows = rows.child(line);
        }
        let mut week = div().flex().gap(px(2.)).mb(px(2.));
        for (zh, _) in WEEK { week = week.child(div().size(px(MINI_CELL)).flex().items_center().justify_center().text_size(px(10.)).text_color(Theme::text_placeholder()).child(crate::settings::text(zh))); }

        let count = self.calendar.events_for(selected).len();
        let eye = if self.calendar.show_events { "eye" } else { "eye-open" };
        let status = {
            let entries = [(Filter::All, tr("全部", "All")), (Filter::Open, tr("未完成", "Open")), (Filter::Done, tr("已完成", "Done"))];
            div().h(px(26.)).p(px(2.)).flex().gap(px(2.)).rounded(px(8.)).bg(Theme::fill_hover_strong())
                .children(entries.map(|(f, label)| {
                    let on = self.calendar.filter == f;
                    div().id(("cal-filter", f as usize)).h_full().flex_1().flex().items_center().justify_center().rounded(px(6.)).cursor_pointer()
                        .text_size(px(11.)).line_height(px(16.))
                        .when(on, |d| d.bg(if theme::dark() { theme::rgbaf(0x63, 0x63, 0x66, 1.) } else { Theme::bg_body() }).shadow(theme::shadow_button()).text_color(Theme::text()))
                        .when(!on, |d| d.text_color(Theme::text_caption()).hover(|s| s.text_color(Theme::text())))
                        .child(label)
                        .on_click(cx.listener(move |w, _, _, cx| { w.calendar.filter = f; cx.notify(); }))
                }))
        };
        let scope = {
            let entries = [(false, tr("本月", "This month")), (true, tr("全部月份", "All months"))];
            div().h(px(24.)).p(px(2.)).flex().gap(px(2.)).rounded(px(7.)).bg(Theme::fill_hover_strong())
                .children(entries.map(|(all, label)| {
                    let on = self.calendar.filter_all_months == all;
                    div().id(("cal-scope", all as usize)).h_full().px(px(7.)).flex().items_center().justify_center().rounded(px(5.)).cursor_pointer()
                        .text_size(px(10.)).line_height(px(14.))
                        .when(on, |d| d.bg(if theme::dark() { theme::rgbaf(0x63, 0x63, 0x66, 1.) } else { Theme::bg_body() }).shadow(theme::shadow_button()).text_color(Theme::text()))
                        .when(!on, |d| d.text_color(Theme::text_caption()).hover(|s| s.text_color(Theme::text())))
                        .child(label)
                        .on_click(cx.listener(move |w, _, _, cx| { w.calendar.filter_all_months = all; cx.notify(); }))
                }))
        };
        let items = self.calendar.filtered_events();
        let mut filter_list = div().id("cal-filter-list").mt(px(8.)).flex_1().min_h_0().overflow_y_scroll().flex().flex_col().gap(px(1.));
        if items.is_empty() {
            filter_list = filter_list.child(div().px(px(6.)).py(px(10.)).text_size(px(11.)).text_color(Theme::text_placeholder()).child(tr("没有符合筛选的事项", "No matching events")));
        }
        for e in &items {
            let id = e.id.clone();
            let day = day_of(e.start);
            let (_, mo, dd) = crate::time::civil_from_days(day);
            let when = format!("{mo}/{dd} {}", crate::settings::text(WEEK[weekday_sun(day) as usize].0));
            filter_list = filter_list.child(div().id(SharedString::from(format!("cal-filter-{id}"))).h(px(30.)).px(px(6.)).flex().items_center().gap(px(7.)).rounded(px(6.))
                .cursor_pointer().hover(|s| s.bg(Theme::fill_hover()))
                .when(e.done, |d| d.opacity(0.6))
                .child(div().size(px(7.)).rounded_full().bg(event_color(e.color)))
                .child(div().flex_1().min_w_0().flex().flex_col()
                    .child(div().text_size(px(11.)).truncate().child(e.title.clone()))
                    .child(div().text_size(px(9.)).text_color(Theme::text_placeholder()).child(when)))
                .when(e.done, |d| d.child(icon("todo-check", 11., Theme::primary())))
                .on_click(cx.listener(move |w, _, window, cx| { cx.stop_propagation(); w.open_event(&id, window, cx); })));
        }

        div().relative().flex_none().w(px(PANEL_W)).h_full().border_r_1().border_color(Theme::line_divider())
            .flex().flex_col().px(px(14.)).py(px(10.)).overflow_hidden()
            .child(week).child(rows)
            .child(div().mt(px(18.)).mb(px(6.)).flex().items_center().justify_between()
                .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text_caption()).child(tr("日历", "Calendar")))
                .child(div().id("cal-eye").size(px(22.)).flex().items_center().justify_center().rounded(px(6.)).cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover())).child(icon(eye, 14., Theme::icon()))
                    .on_click(cx.listener(|w, _, _, cx| { w.calendar.show_events = !w.calendar.show_events; cx.notify(); }))))
            .child(div().id("cal-list-row").h(px(30.)).px(px(6.)).flex().items_center().gap(px(8.)).rounded(px(7.))
                .when(self.calendar.show_events, |d| d.bg(Theme::fill_hover()))
                .child(div().size(px(9.)).rounded_full().bg(event_color(0)))
                .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).child(tr("日历", "Calendar")))
                .child(div().text_size(px(11.)).text_color(Theme::text_placeholder()).child(count.to_string()))
                .on_click(cx.listener(|w, _, _, cx| { w.calendar.show_events = !w.calendar.show_events; cx.notify(); })))
            .child(div().mt(px(18.)).flex().items_center().justify_between()
                .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text_caption()).child(tr("筛选", "Filter")))
                .child(scope))
            .child(div().mt(px(6.)).child(status))
            .child(filter_list)
            .child(div().pt(px(8.)).px(px(4.)).text_size(px(11.)).line_height(px(16.)).text_color(Theme::text_placeholder())
                .child(tr("活动保存在文档库中", "Events are saved in the library")))
    }

    fn calendar_main(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.calendar.mode {
            Mode::Month => self.calendar_month(cx),
            Mode::Week => self.calendar_week(cx, 7),
            Mode::Day => self.calendar_week(cx, 1),
        }
    }

    fn calendar_month(&self, cx: &mut Context<Self>) -> AnyElement {
        let (vy, vm) = self.calendar.cursor;
        let first = crate::time::days_from_civil(vy, vm, 1);
        let start = first - weekday_sun(first);
        let today = today_day();
        let selected = self.calendar.selected;

        let mut head = div().flex_none().h(px(28.)).flex().border_b_1().border_color(Theme::line_divider());
        for (zh, _) in WEEK {
            head = head.child(div().flex_1().min_w_0().flex().items_center().justify_center().text_size(px(11.)).text_color(Theme::text_placeholder()).child(crate::settings::text(zh)));
        }
        let mut weeks = div().flex_1().min_h_0().flex().flex_col();
        for row in 0..6 {
            let mut line = div().flex().flex_1().min_h_0();
            for col in 0..7 {
                let day = start + row * 7 + col;
                let (y, m, d) = crate::time::civil_from_days(day);
                let inside = (y, m) == (vy, vm);
                line = line.child(self.calendar_month_cell(day, d, inside, day == today, day == selected, cx));
            }
            weeks = weeks.child(line);
        }
        div().relative().flex_1().min_w_0().h_full().flex().flex_col().child(head).child(weeks).into_any_element()
    }

    fn calendar_month_cell(&self, day: i64, d: u32, inside: bool, is_today: bool, selected: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let events = if self.calendar.show_events { self.calendar.events_for(day) } else { vec![] };
        let shown = events.len().min(3);
        let weekday = crate::settings::text(WEEK[weekday_sun(day) as usize].0);
        let lunar = lunar_text(day);
        let holiday = holiday_of(day);
        let mut cell = div().id(("cal-cell", day as usize)).relative().flex_1().min_w_0().min_h_0().p(px(5.))
            .flex().flex_col().gap(px(2.)).overflow_hidden().group("cal-cell")
            .border_r_1().border_b_1().border_color(Theme::line_divider())
            .when(!inside, |x| x.bg(theme::ca(craft::N1000, 0.022)))
            .cursor_pointer()
            .child(div().flex().items_center().justify_between().h(px(22.))
                .child(div().flex().items_center().gap(px(4.)).min_w_0().overflow_hidden()
                    .child(div().size(px(22.)).flex_none().flex().items_center().justify_center().rounded_full().text_size(px(12.))
                        .text_color(if is_today { gpui::white() } else if selected { Theme::primary() } else if inside { Theme::text() } else { Theme::text_placeholder() })
                        .when(is_today, |x| x.bg(theme::c(0xeb5757)).font_weight(FontWeight::SEMIBOLD))
                        .when(!is_today && selected, |x| x.bg(Theme::fill_hover_strong()).font_weight(FontWeight::SEMIBOLD))
                        .child(d.to_string()))
                    .child(div().flex_none().text_size(px(11.)).line_height(px(14.)).text_color(Theme::text_caption()).child(weekday)))
                .child(div().id(("cal-cell-add", day as usize)).size(px(18.)).flex().items_center().justify_center().rounded(px(5.))
                    .invisible().group_hover("cal-cell", |x| x.visible()).hover(|x| x.bg(Theme::fill_hover_strong()))
                    .child(icon("plus", 12., Theme::icon()))
                    .on_click(cx.listener(move |w, _, window, cx| { cx.stop_propagation(); w.open_new_event(day, window, cx); }))))
            .child(div().flex().items_center().gap(px(4.)).h(px(14.)).min_w_0().overflow_hidden().text_size(px(10.)).line_height(px(14.))
                .child(div().flex_none().text_color(Theme::text_placeholder()).child(lunar)))
            .children(holiday.map(|h| div().h(px(14.)).flex_none().text_size(px(10.)).line_height(px(14.)).text_color(Theme::danger()).child(h)));
        for e in events.iter().take(shown) {
            let id = e.id.clone();
            let title = e.title.clone();
            let color = e.color;
            let all_day = e.all_day;
            let done = e.done;
            cell = cell.child(div().id(SharedString::from(format!("cal-ev-{day}-{id}"))).w_full().h(px(CHIP_H)).px(px(5.))
                .flex().items_center().gap(px(4.)).rounded(px(4.)).overflow_hidden()
                .bg(event_tint(color)).text_color(event_color(color)).text_size(px(11.)).line_height(px(CHIP_H))
                .when(all_day, |x| x.font_weight(FontWeight::MEDIUM))
                .when(done, |x| x.opacity(0.55))
                .child(div().flex_1().min_w_0().truncate().child(title))
                .on_click(cx.listener(move |w, _, window, cx| { cx.stop_propagation(); w.open_event(&id, window, cx); })));
        }
        if events.len() > shown {
            let more = crate::localized_format!("还有 {} 项", "+{} more", events.len() - shown);
            cell = cell.child(div().px(px(5.)).text_size(px(11.)).text_color(Theme::text_caption()).child(more));
        }
        cell.on_click(cx.listener(move |w, _, window, cx| { w.calendar.selected = day; w.open_new_event(day, window, cx); }))
    }

    fn calendar_week(&self, cx: &mut Context<Self>, days: i64) -> AnyElement {
        let selected = self.calendar.selected;
        let start = if days == 7 { selected - weekday_sun(selected) } else { selected };
        let today = today_day();
        let shown: Vec<i64> = (0..days).map(|i| start + i).collect();
        let mut head = div().flex_none().h(px(52.)).flex().border_b_1().border_color(Theme::line_divider())
            .child(div().flex_none().w(px(52.)));
        for &day in &shown {
            let (_, _, d) = crate::time::civil_from_days(day);
            let is_today = day == today;
            let sel = day == selected;
            head = head.child(div().flex_1().min_w_0().flex().flex_col().items_center().justify_center().gap(px(2.))
                .border_l_1().border_color(Theme::line_divider())
                .child(div().text_size(px(10.)).text_color(Theme::text_placeholder()).child(crate::settings::text(WEEK[weekday_sun(day) as usize].0)))
                .child(div().size(px(24.)).flex().items_center().justify_center().rounded_full().text_size(px(12.))
                    .when(is_today, |x| x.bg(theme::c(0xeb5757)).text_color(gpui::white()))
                    .when(sel && !is_today, |x| x.bg(Theme::fill_hover_strong()))
                    .child(d.to_string())));
        }
        let mut all_day = div().flex_none().min_h(px(24.)).flex().border_b_1().border_color(Theme::line_divider())
            .child(div().flex_none().w(px(52.)).px(px(6.)).flex().items_center().justify_end().text_size(px(10.)).text_color(Theme::text_placeholder()).child(tr("全天", "All day")));
        for &day in &shown {
            let mut col = div().flex_1().min_w_0().flex().flex_col().gap(px(2.)).py(px(2.)).px(px(2.)).border_l_1().border_color(Theme::line_divider());
            for e in self.calendar.events_for(day).into_iter().filter(|e| e.all_day) {
                let id = e.id.clone();
                let done = e.done;
                col = col.child(div().id(SharedString::from(format!("cal-ad-{}-{}", day, id))).h(px(CHIP_H)).px(px(5.)).flex().items_center().rounded(px(4.))
                    .bg(event_tint(e.color)).text_color(event_color(e.color)).text_size(px(11.)).truncate().child(e.title.clone())
                    .when(done, |x| x.opacity(0.55))
                    .on_click(cx.listener(move |w, _, window, cx| { cx.stop_propagation(); w.open_event(&id, window, cx); })));
            }
            all_day = all_day.child(col);
        }
        let mut gutter = div().flex_none().w(px(52.)).flex().flex_col();
        for h in 0..24 { gutter = gutter.child(div().flex_none().h(px(HOUR_H)).pr(px(6.)).flex().items_start().justify_end().text_size(px(10.)).text_color(Theme::text_placeholder()).child(format!("{h}:00"))); }
        let mut grid = div().flex().flex_none().h(px(24. * HOUR_H)).child(gutter);
        for &day in &shown {
            let mut col = div().relative().flex_1().min_w_0().border_l_1().border_color(Theme::line_divider());
            for h in 0..24 {
                let hour = h as u32;
                col = col.child(div().id(("cal-hour", day as usize * 24 + h as usize)).flex_none().h(px(HOUR_H)).border_b_1().border_color(theme::ca(craft::N1000, 0.06)).cursor_pointer()
                    .on_click(cx.listener(move |w, _, window, cx| {
                        w.calendar.selected = day;
                        w.open_new_event_at(day, hour * 60, window, cx);
                    })));
            }
            if self.calendar.show_events {
                for e in self.calendar.events_for(day).into_iter().filter(|e| !e.all_day) {
                    let id = e.id.clone();
                    let top = minute_of_day(e.start) as f32 / 60. * HOUR_H;
                    let len = ((e.end - e.start).max(30) as f32 / 3600. * HOUR_H).max(20.);
                    col = col.child(div().id(SharedString::from(format!("cal-tv-{}-{}", day, id))).absolute().left(px(3.)).right(px(3.)).top(px(top + 1.)).h(px(len - 2.))
                        .px(px(4.)).rounded(px(4.)).overflow_hidden().bg(event_tint(e.color)).text_color(event_color(e.color)).text_size(px(11.)).line_height(px(14.))
                        .when(e.done, |x| x.opacity(0.55))
                        .child(div().truncate().child(e.title.clone()))
                        .child(div().text_size(px(10.)).opacity(0.8).child(hm(minute_of_day(e.start))))
                        .on_click(cx.listener(move |w, _, window, cx| { cx.stop_propagation(); w.open_event(&id, window, cx); })));
                }
            }
            grid = grid.child(col);
        }
        div().flex_1().min_w_0().h_full().flex().flex_col()
            .child(head).child(all_day)
            .child(div().id("cal-week-scroll").flex_1().min_h_0().overflow_y_scroll().child(grid)).into_any_element()
    }

    // ---- the right panel

    fn calendar_detail(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = div().flex_none().h(px(48.)).px(px(14.)).flex().items_center().justify_between()
            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(tr("活动", "Activity")))
            .child(div().id("cal-close").size(px(24.)).flex().items_center().justify_center().rounded(px(6.)).cursor_pointer()
                .hover(|s| s.bg(Theme::fill_hover()))
                .child(icon("x", 14., Theme::icon()))
                .on_click(cx.listener(|w, _, _, cx| { w.calendar.draft = None; cx.notify(); })));
        let body = if let Some(d) = &self.calendar.draft { self.calendar_draft(d, window, cx) } else { self.calendar_day_detail(cx) };
        div().flex_none().w(px(DETAIL_W)).h_full().border_l_1().border_color(Theme::line_divider())
            .bg(theme::raised(Theme::bg_body())).flex().flex_col().overflow_hidden()
            .child(header).child(div().id("cal-detail-scroll").flex_1().min_h_0().overflow_y_scroll().child(body))
    }

    fn calendar_day_detail(&self, cx: &mut Context<Self>) -> Div {
        let day = self.calendar.selected;
        let events = if self.calendar.show_events { self.calendar.events_for(day) } else { vec![] };
        let mut list = div().flex().flex_col().gap(px(2.));
        for e in &events {
            let id = e.id.clone();
            let time = if e.all_day { tr("全天", "All day").to_string() } else { hm(minute_of_day(e.start)) };
            list = list.child(div().id(SharedString::from(format!("cal-day-{}", id))).h(px(36.)).px(px(8.)).flex().items_center().gap(px(8.)).rounded(px(7.))
                .cursor_pointer().hover(|s| s.bg(Theme::fill_hover()))
                .when(e.done, |x| x.opacity(0.6))
                .child(div().w(px(3.)).h(px(20.)).rounded(px(2.)).bg(event_color(e.color)))
                .child(div().flex_1().min_w_0().flex().flex_col()
                    .child(div().flex().items_center().gap(px(4.))
                        .when(e.done, |x| x.child(icon("todo-check", 10., Theme::primary())))
                        .child(div().flex_1().min_w_0().text_size(px(12.)).truncate().child(e.title.clone())))
                    .child(div().text_size(px(11.)).text_color(Theme::text_caption()).child(time)))
                .on_click(cx.listener(move |w, _, window, cx| { cx.stop_propagation(); w.open_event(&id, window, cx); })));
        }
        div().px(px(14.)).py(px(12.)).flex().flex_col().gap(px(10.))
            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(date_label(day)))
            .child(div().id("cal-new-event").w_full().h(px(30.)).px(px(10.)).flex().items_center().justify_center().rounded(px(8.)).cursor_pointer()
                .bg(Theme::primary()).text_color(gpui::white()).text_size(px(12.))
                .hover(|s| s.opacity(0.9))
                .child(tr("新建事件", "New event"))
                .on_click(cx.listener(move |w, _, window, cx| w.open_new_event(day, window, cx))))
            .child(div().mt(px(6.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text_caption()).child(tr("活动", "Activity")))
            .when(events.is_empty(), |d| d.child(div().py(px(12.)).text_size(px(12.)).text_color(Theme::text_placeholder()).child(tr("暂无活动", "No events"))))
            .child(list)
    }

    fn calendar_draft(&self, d: &Draft, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let field = |e: Entity<TextField>| div().h(px(30.)).px(px(8.)).flex().items_center().rounded(px(8.)).bg(Theme::fill_hover())
            .child(div().flex_1().min_w_0().child(e));
        let textarea = |e: Entity<TextField>, h: f32| div().w_full().min_h(px(h)).px(px(8.)).py(px(6.)).rounded(px(8.)).bg(Theme::fill_hover())
            .flex().child(e);
        let day = d.day;
        let day_label = date_label(day);
        let mut colors = div().flex().gap(px(8.));
        for i in 0..EVENT_COLORS.len() {
            let on = d.color as usize % EVENT_COLORS.len() == i;
            colors = colors.child(div().id(("cal-color", i)).size(px(18.)).rounded_full().cursor_pointer()
                .bg(event_color(i as u8))
                .border_2().border_color(if on { Theme::text() } else { transparent_black() })
                .on_click(cx.listener(move |w, _, _, cx| { if let Some(d) = &mut w.calendar.draft { d.color = i as u8; } cx.notify(); })));
        }
        let can_delete = d.id.is_some();
        div().px(px(14.)).py(px(12.)).flex().flex_col().gap(px(10.))
            .child(field(d.title.clone()))
            .child(div().flex().items_center().justify_between()
                .child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(day_label))
                .child(div().flex().gap(px(2.))
                    .child(div().id("cal-day-prev").size(px(22.)).flex().items_center().justify_center().rounded(px(6.)).cursor_pointer().hover(|s| s.bg(Theme::fill_hover()))
                        .child(icon("chevron-left", 14., Theme::icon()))
                        .on_click(cx.listener(|w, _, _, cx| { if let Some(d) = &mut w.calendar.draft { d.day -= 1; } cx.notify(); })))
                    .child(div().id("cal-day-next").size(px(22.)).flex().items_center().justify_center().rounded(px(6.)).cursor_pointer().hover(|s| s.bg(Theme::fill_hover()))
                        .child(icon("chevron-right", 14., Theme::icon()))
                        .on_click(cx.listener(|w, _, _, cx| { if let Some(d) = &mut w.calendar.draft { d.day += 1; } cx.notify(); })))))
            .child(div().flex().items_center().justify_between()
                .child(div().text_size(px(12.)).child(tr("全天", "All day")))
                .child(settings::switch("cal-allday", d.all_day)
                    .on_click(cx.listener(|w, _, _, cx| { if let Some(d) = &mut w.calendar.draft { d.all_day = !d.all_day; } cx.notify(); }))))
            .when(!d.all_day, |x| x.child(div().flex().items_center().gap(px(6.))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(3.))
                    .child(div().text_size(px(11.)).text_color(Theme::text_caption()).child(tr("开始", "Start")))
                    .child(field(d.start_field.clone())))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(3.))
                    .child(div().text_size(px(11.)).text_color(Theme::text_caption()).child(tr("结束", "End")))
                    .child(field(d.end_field.clone())))))
            .child(div().flex().items_center().gap(px(8.))
                .child(div().text_size(px(11.)).text_color(Theme::text_caption()).child(tr("颜色", "Color")))
                .child(colors))
            .child(div().text_size(px(11.)).text_color(Theme::text_caption()).child(tr("活动内容", "Details")))
            .child(textarea(d.content.clone(), 84.))
            .child(div().id("cal-done").h(px(28.)).flex().items_center().gap(px(8.)).rounded(px(6.)).cursor_pointer()
                .hover(|s| s.bg(Theme::fill_hover()))
                .child(ui::checkbox(d.done))
                .child(div().text_size(px(12.)).child(tr("已完成", "Done")))
                .on_click(cx.listener(|w, _, _, cx| { if let Some(d) = &mut w.calendar.draft { d.done = !d.done; } cx.notify(); })))
            .child(div().text_size(px(11.)).text_color(Theme::text_caption()).child(tr("备注", "Notes")))
            .child(textarea(d.notes.clone(), 72.))
            .children(d.error.clone().map(|e| div().text_size(px(11.)).line_height(px(16.)).text_color(Theme::danger()).child(e)))
            .child(div().mt(px(4.)).flex().items_center().gap(px(6.))
                .when(can_delete, |x| x.child(settings::link_button("cal-delete", tr("删除", "Delete"))
                    .on_click(cx.listener(|w, _, _, cx| w.delete_event(cx)))))
                .child(div().flex_1())
                .child(ui::secondary_button("cal-cancel", tr("取消", "Cancel"))
                    .on_click(cx.listener(|w, _, _, cx| { w.calendar.draft = None; cx.notify(); })))
                .child(ui::primary_button("cal-save", tr("保存", "Save"))
                    .on_click(cx.listener(|w, _, _, cx| w.commit_event(cx)))))
    }

    // ---- events

    fn open_new_event(&mut self, day: i64, window: &mut Window, cx: &mut Context<Self>) {
        let start_min = self.calendar.next_event_start(day);
        self.open_draft(None, day, start_min, (start_min + 60).min(23 * 60 + 59), window, cx);
    }

    fn open_new_event_at(&mut self, day: i64, start_min: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.open_draft(None, day, start_min, (start_min + 60).min(23 * 60 + 59), window, cx);
    }

    pub(crate) fn open_event(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.calendar.events.iter().find(|e| e.id == id).cloned() else { return };
        self.calendar.selected = day_of(e.start);
        let start_min = minute_of_day(e.start);
        let end_min = minute_of_day(e.end);
        let day = day_of(e.start);
        let (start_min, end_min) = if e.all_day { (0, 23 * 60 + 59) } else { (start_min, end_min.max(start_min + 1)) };
        self.open_draft(Some(e), day, start_min, end_min, window, cx);
    }

    fn open_draft(&mut self, event: Option<Event>, day: i64, start_min: u32, end_min: u32, window: &mut Window, cx: &mut Context<Self>) {
        let all_day = event.as_ref().is_some_and(|e| e.all_day);
        let done = event.as_ref().is_some_and(|e| e.done);
        let color = event.as_ref().map_or(0, |e| e.color);
        let title_text = event.as_ref().map(|e| e.title.clone()).unwrap_or_default();
        let content_text = event.as_ref().map(|e| e.content.clone()).unwrap_or_default();
        let notes_text = event.as_ref().map(|e| e.notes.clone()).unwrap_or_default();
        let title = cx.new(|cx| {
            let mut f = TextField::new(tr("添加标题", "Add a title"), cx);
            f.size = 14.;
            f.line_height = 20.;
            f.set_text(&title_text, cx);
            f
        });
        let start_field = cx.new(|cx| {
            let mut f = TextField::new("09:00", cx);
            f.size = 12.;
            f.line_height = 18.;
            f.set_text(&hm(start_min), cx);
            f
        });
        let end_field = cx.new(|cx| {
            let mut f = TextField::new("10:00", cx);
            f.size = 12.;
            f.line_height = 18.;
            f.set_text(&hm(end_min), cx);
            f
        });
        let content = cx.new(|cx| {
            let mut f = TextField::new(tr("添加活动内容…", "Add details…"), cx);
            f.size = 12.;
            f.line_height = 18.;
            f.multiline = true;
            f.wrap = true;
            f.set_text(&content_text, cx);
            f
        });
        let notes = cx.new(|cx| {
            let mut f = TextField::new(tr("添加备注…", "Add notes…"), cx);
            f.size = 12.;
            f.line_height = 18.;
            f.multiline = true;
            f.wrap = true;
            f.set_text(&notes_text, cx);
            f
        });
        let mut subs = Vec::new();
        for field in [&title, &start_field, &end_field, &content, &notes] {
            subs.push(cx.subscribe(field, |w: &mut Workspace, _, ev: &TextFieldEvent, cx| match ev {
                TextFieldEvent::Cancel => { w.calendar.draft = None; cx.notify(); }
                TextFieldEvent::Submit => w.commit_event(cx),
                _ => {}
            }));
        }
        title.update(cx, |f, cx| f.select_all_text(cx));
        self.calendar.draft = Some(Draft { id: event.map(|e| e.id), day, title: title.clone(), start_field, end_field, content, notes, all_day, done, color, error: None });
        self.calendar._subs = subs;
        self.calendar.selected = day;
        self.calendar.cursor = month_of(day);
        window.focus(&title.focus_handle(cx), cx);
        cx.notify();
    }

    fn commit_event(&mut self, cx: &mut Context<Self>) {
        let Some(d) = &self.calendar.draft else { return };
        let title = d.title.read(cx).text.trim().to_string();
        if title.is_empty() {
            self.calendar.draft.as_mut().unwrap().error = Some(tr("标题不能为空", "A title is required").into());
            return cx.notify();
        }
        let all_day = d.all_day;
        let (start_min, end_min) = if all_day { (0, 23 * 60 + 59) } else {
            let (Some(s), Some(e)) = (parse_hm(&d.start_field.read(cx).text), parse_hm(&d.end_field.read(cx).text)) else {
                self.calendar.draft.as_mut().unwrap().error = Some(tr("时间格式应为 HH:MM", "Use HH:MM for the time").into());
                return cx.notify();
            };
            if e <= s {
                self.calendar.draft.as_mut().unwrap().error = Some(tr("结束时间要晚于开始时间", "The end must be after the start").into());
                return cx.notify();
            }
            (s, e)
        };
        let (y, m, dd) = crate::time::civil_from_days(d.day);
        let start = crate::time::local_to_unix(y, m, dd, start_min / 60, start_min % 60);
        let end = crate::time::local_to_unix(y, m, dd, end_min / 60, end_min % 60);
        let content = d.content.read(cx).text.clone();
        let notes = d.notes.read(cx).text.clone();
        let (id, color, day, done) = (d.id.clone(), d.color, d.day, d.done);
        let event = Event { id: id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()), title, start, end, all_day, color, done, content, notes };
        match id {
            Some(id) => {
                if let Some(old) = self.calendar.events.iter_mut().find(|e| e.id == id) { *old = event; }
            }
            None => self.calendar.events.push(event),
        }
        self.calendar.events.sort_by_key(|e| e.start);
        if let Err(e) = save_events(&self.calendar.events) {
            self.calendar.draft.as_mut().unwrap().error = Some(format!("{}{e}", tr("保存失败：", "Could not save: ")).into());
            return cx.notify();
        }
        self.calendar.selected = day;
        self.calendar.draft = None;
        cx.notify();
    }

    fn delete_event(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.calendar.draft.as_ref().and_then(|d| d.id.clone()) else { return };
        self.calendar.events.retain(|e| e.id != id);
        if let Err(e) = save_events(&self.calendar.events) {
            self.calendar.draft.as_mut().unwrap().error = Some(format!("{}{e}", tr("删除失败：", "Could not delete: ")).into());
            return cx.notify();
        }
        self.calendar.draft = None;
        cx.notify();
    }
}
