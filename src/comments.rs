//! 批注: annotations attached to selected text. They live beside the library in
//! `.quiet/comments/<document-id>.json`, so they sync with the library and do
//! not change the document's DocxXML. The right-hand panel edits and deletes
//! them; the selection toolbar creates them.

use crate::app::Workspace;
use crate::theme::Theme;
use crate::ui::{self, icon};
use gpui::prelude::FluentBuilder;
use gpui::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    /// The text that was selected when the comment was made.
    #[serde(default)]
    pub quote: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub created: u64,
}

fn path_at(root: &Path, id: &str) -> PathBuf { root.join("comments").join(format!("{id}.json")) }

/// The round check mark used only while the comments panel is managing.
fn round_check(on: bool) -> Div {
    let box_ = div().flex_none().size(px(16.)).rounded_full().border_1().flex().items_center().justify_center();
    if on {
        box_.bg(Theme::primary()).border_color(Theme::primary()).child(icon("check", 10., gpui::white()))
    } else {
        box_.border_color(Theme::line_border())
    }
}

fn load_at(root: &Path, id: &str) -> Vec<Comment> {
    std::fs::read(path_at(root, id)).ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_at(root: &Path, id: &str, comments: &[Comment]) -> anyhow::Result<()> {
    let dir = root.join("comments");
    std::fs::create_dir_all(&dir)?;
    let path = path_at(root, id);
    let tmp = path.with_file_name(format!(".{id}.json.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(comments)?)?;
    if path.exists() { std::fs::remove_file(&path)?; }
    std::fs::rename(tmp, path)?;
    Ok(())
}

impl Workspace {
    pub(crate) fn load_comments(&mut self, id: &str, cx: &mut Context<Self>) {
        self.comments = load_at(&crate::store::Store::internal_dir(), id);
        self.comments_open = !self.comments.is_empty();
        self.comment_editing = None;
        self.comment_selected.clear();
        self.comment_managing = false;
        self.comment_field.update(cx, |f, cx| f.set_text("", cx));
        cx.notify();
    }

    fn save_comments(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.current.as_deref() else { return };
        if let Err(e) = save_at(&crate::store::Store::internal_dir(), id, &self.comments) {
            self.show_toast_kind(ui::ToastKind::Error, &format!("{}{e}", crate::settings::tr("批注保存失败：", "Could not save the comment: ")), cx);
        }
    }

    pub(crate) fn begin_comment(&mut self, quote: String, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_comment(cx);
        let quote = quote.trim().to_string();
        if quote.is_empty() { return }
        self.comments.push(Comment {
            id: uuid::Uuid::new_v4().to_string(),
            quote,
            text: String::new(),
            created: crate::time::now(),
        });
        self.comment_editing = Some(self.comments.len() - 1);
        self.comment_field.update(cx, |f, cx| f.set_text("", cx));
        self.comments_open = true;
        window.focus(&self.comment_field.focus_handle(cx), cx);
        cx.notify();
    }

    pub(crate) fn edit_comment(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.comments.get(index).map(|c| c.id.clone()) else { return };
        self.finish_comment(cx);
        let Some(index) = self.comments.iter().position(|c| c.id == id) else { return };
        let text = self.comments[index].text.clone();
        self.comment_editing = Some(index);
        self.comment_field.update(cx, |f, cx| f.set_text(&text, cx));
        window.focus(&self.comment_field.focus_handle(cx), cx);
        cx.notify();
    }

    /// Save the open field. An empty comment is dropped, as are comments whose
    /// text was cleared. Called by Enter, Escape and losing focus.
    pub(crate) fn finish_comment(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.comment_editing.take() else { return };
        if index >= self.comments.len() { return }
        let text = self.comment_field.read(cx).text.trim().to_string();
        self.comments[index].text = text;
        if self.comments[index].text.is_empty() {
            let id = self.comments[index].id.clone();
            self.comments.remove(index);
            self.comment_selected.remove(&id);
        }
        self.comments_open = !self.comments.is_empty();
        self.save_comments(cx);
        cx.notify();
    }

    pub(crate) fn delete_comment(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(id) = self.comments.get(index).map(|c| c.id.clone()) else { return };
        self.finish_comment(cx);
        if let Some(index) = self.comments.iter().position(|c| c.id == id) {
            self.comments.remove(index);
        }
        self.comment_selected.remove(&id);
        self.comments_open = !self.comments.is_empty();
        self.save_comments(cx);
        cx.notify();
    }

    pub(crate) fn toggle_comment_selected(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.comment_selected.remove(id) { self.comment_selected.insert(id.to_string()); }
        cx.notify();
    }

    pub(crate) fn toggle_comment_managing(&mut self, cx: &mut Context<Self>) {
        self.comment_managing = !self.comment_managing;
        if !self.comment_managing { self.comment_selected.clear(); }
        cx.notify();
    }

    pub(crate) fn delete_selected_comments(&mut self, cx: &mut Context<Self>) {
        self.finish_comment(cx);
        let selected = std::mem::take(&mut self.comment_selected);
        self.comments.retain(|c| !selected.contains(&c.id));
        self.comments_open = !self.comments.is_empty();
        self.save_comments(cx);
        cx.notify();
    }

    pub(crate) fn render_comments(&self, _window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.comment_selected.len();
        let list = if self.comments.is_empty() {
            div().flex_1().px(px(14.)).py(px(24.)).text_size(px(12.)).line_height(px(18.)).text_color(Theme::text_caption())
                .child(crate::settings::tr("还没有批注。选择文字后点击工具栏上的批注按钮。", "No comments yet. Select text and use the comment button in the toolbar."))
                .into_any_element()
        } else {
            let mut list = div().id("comments-list").flex_1().min_h_0().overflow_y_scroll().px(px(12.)).py(px(12.)).flex().flex_col().gap(px(10.));
            for (i, c) in self.comments.iter().enumerate() {
                let on = self.comment_selected.contains(&c.id);
                let cid = c.id.clone();
                let editing = self.comment_editing == Some(i);
                let body = if editing {
                    div().h(px(30.)).px(px(7.)).flex().items_center().rounded(px(7.)).bg(Theme::fill_hover()).child(self.comment_field.clone()).into_any_element()
                } else {
                    let text: SharedString = if c.text.trim().is_empty() { crate::settings::tr("点击添加批注", "Click to add a comment").into() } else { c.text.clone().into() };
                    let color = if c.text.trim().is_empty() { Theme::text_placeholder() } else { Theme::text() };
                    div().id(("comment-text", i)).text_size(px(12.)).line_height(px(18.)).text_color(color).child(text)
                        .on_click(cx.listener(move |w, _, window, cx| w.edit_comment(i, window, cx)))
                        .into_any_element()
                };
                list = list.child(div().id(("comment", i)).flex().items_start().gap(px(7.)).p(px(8.)).rounded(px(8.)).bg(Theme::fill_hover())
                    .when(self.comment_managing, |d| d.child(div().id(("comment-check", i)).cursor_pointer().child(round_check(on))
                        .on_click(cx.listener(move |w, _, _, cx| w.toggle_comment_selected(&cid, cx)))))
                    .child(div().flex_1().min_w_0().flex().flex_col().gap(px(5.))
                        .child(div().text_size(px(11.)).line_height(px(16.)).text_color(Theme::text_caption()).line_clamp(3).child(c.quote.clone()))
                        .child(body))
                    .child(div().id(("comment-del", i)).p(px(3.)).rounded(px(4.)).cursor_pointer().hover(|s| s.bg(Theme::fill_hover_strong()))
                        .child(icon("bin", 14., Theme::icon()))
                        .on_click(cx.listener(move |w, _, _, cx| w.delete_comment(i, cx)))));
            }
            list.into_any_element()
        };
        let header = div().h(px(48.)).px(px(14.)).flex_none().flex().items_center().justify_between().border_b_1().border_color(Theme::line_border())
            .child(div().flex().items_center().gap(px(6.))
                .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(crate::settings::tr("批注", "Comments")))
                .child(div().text_size(px(11.)).text_color(Theme::text_caption()).child(self.comments.len().to_string())))
            .child(div().flex().items_center().gap(px(8.))
                .child(div().id("comments-manage").h(px(24.)).px(px(8.)).flex().items_center().gap(px(4.)).rounded(px(6.)).cursor_pointer()
                    .text_size(px(11.)).text_color(if self.comment_managing { Theme::primary() } else { Theme::text_caption() })
                    .when(self.comment_managing, |d| d.bg(Theme::fill_hover()))
                    .child(icon("batch", 14., if self.comment_managing { Theme::primary() } else { Theme::icon() }))
                    .child(crate::settings::tr(if self.comment_managing { "完成" } else { "管理" }, if self.comment_managing { "Done" } else { "Manage" }))
                    .on_click(cx.listener(|w, _, _, cx| w.toggle_comment_managing(cx))))
                .when(selected > 0, |d| d.child(
                    div().id("comments-delete-selected").cursor_pointer().text_size(px(11.)).text_color(Theme::danger())
                        .child(crate::localized_format!("删除选中 {}", "Delete {}", selected))
                        .on_click(cx.listener(|w, _, _, cx| w.delete_selected_comments(cx))),
                )));
        div().w(px(300.)).flex_none().h_full().flex().flex_col().border_l_1().border_color(Theme::line_border()).bg(Theme::bg_body())
            .child(header).child(list).into_any_element()
    }
}
