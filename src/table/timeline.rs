//! 时间轴 / 甘特视图: each record a bar from its start date to its end date.

use super::style::*;
use super::*;
use crate::feishu_theme::{c, ud};

fn label(s: &str) -> String { crate::settings::label(s) }

const DAY_MS: i64 = 86_400_000;

fn bar_color(i: usize) -> (Hsla, Hsla) { tag_colors((i % 8) as u8 + 4) }

pub(crate) fn render(v: &TableView, _base: &Base, t: &Table, view: &View, _window: &mut Window, cx: &Context<TableView>) -> AnyElement {
    let Some(start_field) = view.opts.date.and_then(|id| t.field(id)) else {
        return div().flex_1().flex().items_center().justify_center().text_size(px(TEXT)).text_color(Theme::text_caption())
            .child(label("在视图设置中选择开始日期字段")).into_any_element();
    };
    let end_field = view.opts.end.and_then(|id| t.field(id));
    let mut rows: Vec<(usize, i64, i64)> = vec![];
    for i in &v.result.records {
        let rec = &t.records[*i];
        let Value::Date(start) = rec.get(start_field.id) else { continue };
        let end = match end_field.map(|f| rec.get(f.id)) { Some(Value::Date(end)) => (*end).max(start + DAY_MS), _ => start + DAY_MS };
        rows.push((*i, *start, end));
    }
    if rows.is_empty() {
        return div().flex_1().flex().items_center().justify_center().text_size(px(TEXT)).text_color(Theme::text_caption())
            .child(label("没有带日期的记录")).into_any_element();
    }
    rows.sort_by_key(|(_, start, _)| *start);
    let min = rows.iter().map(|(_, s, _)| *s).min().unwrap_or(0) / DAY_MS;
    let max = rows.iter().map(|(_, _, e)| *e).max().unwrap_or(0) / DAY_MS;
    let span = ((max - min + 1).max(1)) as f32;
    let mut list = div().id("tb-timeline").flex_1().min_h_0().overflow_y_scroll().px(px(16.)).py(px(14.)).flex().flex_col().gap(px(6.));
    list = list.child(div().h(px(28.)).flex().items_center().gap(px(10.))
        .child(div().w(px(190.)).flex_none())
        .child(div().flex_1().flex().justify_between().text_size(px(11.)).text_color(Theme::text_caption())
            .child(field::display(t, start_field, &Value::Date(min * DAY_MS)))
            .child(field::display(t, start_field, &Value::Date(max * DAY_MS)))));
    for (n, (i, start, end)) in rows.iter().enumerate() {
        let rec = &t.records[*i];
        let rid = rec.id;
        let left = ((start / DAY_MS - min) as f32 / span).clamp(0., 1.);
        let width = (((end - start).max(DAY_MS) / DAY_MS) as f32 / span).clamp(0.01, 1.);
        let (bar_bg, bar_fg) = bar_color(n);
        let bar = div().id(("tb-timeline-bar", *i)).absolute().top(px(9.)).h(px(26.)).left(relative(left)).w(relative(width)).rounded(px(6.)).bg(bar_bg)
            .flex().items_center().px(px(8.)).overflow_hidden().cursor_pointer()
            .child(div().text_size(px(11.)).text_color(bar_fg).whitespace_nowrap().text_ellipsis().child(t.title(rec)))
            .on_click(cx.listener(move |v, _, _, cx| { v.expanded = Some(rid); cx.notify(); }));
        list = list.child(div().h(px(44.)).flex().items_center().gap(px(10.))
            .child(div().w(px(190.)).flex_none().text_size(px(12.)).whitespace_nowrap().text_ellipsis().child(t.title(rec)))
            .child(div().relative().flex_1().h_full().rounded(px(6.)).bg(c(ud::N50))
                .child(div().absolute().left(relative(left)).top_0().bottom_0().w(px(1.)).bg(line()))
                .child(bar)));
    }
    list.into_any_element()
}
