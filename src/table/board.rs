//! 看板视图: one column per option of the view's 单选/人员 field, cards under it.

use super::style::*;
use super::*;
use crate::feishu_theme::{c, ud};

fn label(s: &str) -> String { crate::settings::label(s) }

pub(crate) fn render(_v: &TableView, base: &Base, t: &Table, view: &View, _window: &mut Window, cx: &Context<TableView>) -> AnyElement {
    let columns = query::kanban(base, t, view);
    let stack = view.opts.stack.and_then(|id| t.field(id));
    let mut board = div().id("tb-board").flex_1().min_h_0().overflow_x_scroll().flex().gap(px(12.)).px(px(16.)).py(px(16.));
    for (opt, records) in columns {
        let (name, slot) = match (stack, opt) {
            (Some(f), Some(id)) => f.option(id).map(|o| (o.name.clone(), o.color)).unwrap_or_else(|| (label("未分类"), 9)),
            _ => (label("未分类"), 9),
        };
        let mut column = div().flex_none().w(px(270.)).h_full().flex().flex_col().gap(px(8.)).rounded(px(10.)).bg(c(ud::N50)).p(px(8.));
        column = column.child(div().h(px(30.)).px(px(4.)).flex().items_center().gap(px(6.))
            .child(tag(&name, slot))
            .child(div().text_size(px(11.)).text_color(Theme::text_caption()).child(records.len().to_string())));
        let mut list = div().id(("tb-board-list", opt.map(|o| o as usize).unwrap_or(0))).flex_1().min_h_0().overflow_y_scroll().flex().flex_col().gap(px(8.));
        for i in records {
            let rec = &t.records[i];
            let rid = rec.id;
            let mut card = div().id(("tb-card", rid)).w_full().flex().flex_col().gap(px(6.)).p(px(10.)).rounded(px(8.))
                .bg(Theme::bg_body()).border_1().border_color(line()).cursor_pointer().hover(|s| s.bg(Theme::fill_hover()))
                .child(div().text_size(px(TEXT)).font_weight(FontWeight::MEDIUM).line_height(px(18.)).child(t.title(rec)));
            for f in t.visible_fields(view).into_iter().filter(|f| Some(f.id) != t.primary().map(|p| p.id)).take(2) {
                let value = rec.get(f.id);
                if value.is_empty() { continue }
                card = card.child(div().flex().items_center().gap(px(6.))
                    .child(div().w(px(58.)).flex_none().text_size(px(10.)).text_color(Theme::text_caption()).truncate().child(f.name.clone()))
                    .child(div().flex_1().min_w_0().text_size(px(11.)).child(cell::value_view(base, f, value, false))));
            }
            list = list.child(card.on_click(cx.listener(move |v, _, _, cx| { v.expanded = Some(rid); cx.notify(); })));
        }
        if let Some(field) = stack {
            let (fid, opt) = (field.id, opt);
            list = list.child(div().id("tb-board-add").h(px(30.)).px(px(6.)).flex().items_center().gap(px(6.)).rounded(px(6.)).cursor_pointer()
                .text_size(px(11.)).text_color(Theme::text_caption()).hover(|s| s.bg(Theme::fill_hover()))
                .child(icon("plus", 13., Theme::icon())).child(label("新建"))
                .on_click(cx.listener(move |v, _, _, cx| {
                    let value = opt.map(|o| Value::Select(vec![o])).unwrap_or_default();
                    let with = if value.is_empty() { vec![] } else { vec![(fid, value)] };
                    let tid = v.table;
                    v.edit(cx, |d| d.add_records(tid, None, 1, &with));
                    v.sync_now(cx);
                    cx.notify();
                })));
        }
        board = board.child(column.child(list));
    }
    board.into_any_element()
}
