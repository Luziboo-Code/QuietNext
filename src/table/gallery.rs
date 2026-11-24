//! 画廊视图: each record a card, the filtered/sorted set laid out in a wrapping grid.

use super::style::*;
use super::*;
use crate::feishu_theme::{c, ud};

fn label(s: &str) -> String { crate::settings::label(s) }

pub(crate) fn render(v: &TableView, base: &Base, t: &Table, view: &View, _window: &mut Window, cx: &Context<TableView>) -> AnyElement {
    let mut grid = div().id("tb-gallery").flex_1().min_h_0().overflow_y_scroll().flex().flex_wrap().gap(px(12.)).px(px(16.)).py(px(16.));
    if v.result.records.is_empty() {
        return grid.child(div().text_size(px(TEXT)).text_color(Theme::text_caption()).child(label("没有记录"))).into_any_element();
    }
    for i in &v.result.records {
        let rec = &t.records[*i];
        let rid = rec.id;
        let mut props = div().w_full().flex().flex_col().gap(px(4.));
        for f in t.visible_fields(view).into_iter().filter(|f| Some(f.id) != t.primary().map(|p| p.id)).take(3) {
            let value = rec.get(f.id);
            if value.is_empty() { continue }
            props = props.child(div().w_full().flex().items_center().gap(px(6.))
                .child(div().w(px(62.)).flex_none().text_size(px(10.)).text_color(Theme::text_caption()).truncate().child(f.name.clone()))
                .child(div().flex_1().min_w_0().text_size(px(11.)).child(cell::value_view(base, f, value, false))));
        }
        let card = div().id(("tb-gallery-card", rid)).flex_none().w(px(218.)).min_h(px(150.)).flex().flex_col().rounded(px(9.))
            .bg(Theme::bg_body()).border_1().border_color(line()).overflow_hidden().cursor_pointer().hover(|s| s.bg(Theme::fill_hover()))
            .child(div().h(px(64.)).w_full().bg(c(ud::N100)).flex().items_center().justify_center()
                .child(icon("view-card", 24., c(ud::N400))))
            .child(div().p(px(10.)).flex().flex_col().gap(px(8.))
                .child(div().text_size(px(TEXT)).font_weight(FontWeight::MEDIUM).line_height(px(18.)).child(t.title(rec)))
                .child(props))
            .on_click(cx.listener(move |v, _, _, cx| { v.expanded = Some(rid); cx.notify(); }));
        grid = grid.child(card);
    }
    grid.into_any_element()
}
