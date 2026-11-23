//! 模板: user-defined note templates, inserted at the caret with a shortcut
//! (the Obsidian Templater idea, kept local to the library's preferences).
//!
//! A template is a name and a Markdown body. `{{date}}`, `{{time}}`,
//! `{{title}}` and friends are filled in when it is inserted, so a daily note
//! template stays useful without editing it.

use serde::{Deserialize, Serialize};

/// One 模板, as saved in `preferences.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Template {
    /// The row's label in the picker and 设置.
    pub name: String,
    /// Markdown: headings, lists, to-dos and quotes all parse on insert.
    pub body: String,
}

/// Placeholders a body may use; shown in 设置 so they can be found.
pub const VARIABLES: [(&str, &str); 5] = [
    ("{{date}}", "2026-11-23"),
    ("{{time}}", "14:05"),
    ("{{weekday}}", "星期一"),
    ("{{title}}", "The note's title"),
    ("{{clipboard}}", "The text on the clipboard"),
];

/// The starters offered until the user saves a list of their own.
pub fn builtin() -> Vec<Template> {
    vec![
        Template {
            name: crate::settings::tr("每日笔记", "Daily note").to_string(),
            body: "# {{date}} {{weekday}}\n\n## 待办\n\n- [ ] \n\n## 记录\n\n".to_string(),
        },
        Template {
            name: crate::settings::tr("会议记录", "Meeting notes").to_string(),
            body: "# {{title}}\n\n- 日期：{{date}}\n- 参与人：\n\n## 议题\n\n## 结论\n\n- [ ] \n".to_string(),
        },
    ]
}

/// Expand the placeholders in `body`. `title` is the open document's title,
/// `clipboard` its plain text or "".
pub fn expand(body: &str, title: &str, clipboard: Option<&str>) -> String {
    if !body.contains("{{") {
        return body.to_string();
    }
    let (y, mo, d, h, mi) = crate::time::civil(crate::time::now());
    let weekday = crate::settings::text(WEEKDAYS[crate::time::weekday(y, mo, d) as usize]);
    body
        .replace("{{date}}", &format!("{y:04}-{mo:02}-{d:02}"))
        .replace("{{time}}", &format!("{h:02}:{mi:02}"))
        .replace("{{weekday}}", weekday)
        .replace("{{title}}", title)
        .replace("{{clipboard}}", clipboard.unwrap_or("").trim())
}

/// 星期一 … 星期日, `time::weekday`'s order (0 is Monday).
const WEEKDAYS: [&str; 7] = ["星期一", "星期二", "星期三", "星期四", "星期五", "星期六", "星期日"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_filled_in() {
        let out = expand("{{title}} on {{date}}", "Plan", None);
        assert!(out.starts_with("Plan on "));
        assert_eq!(out.len(), "Plan on 2026-11-23".len(), "{out}");
        assert_eq!(expand("no placeholders", "x", None), "no placeholders");
        assert_eq!(expand("{{clipboard}}", "x", Some("  pasted \n")), "pasted");
        assert_eq!(expand("{{clipboard}}", "x", None), "");
    }

    #[test]
    fn placeholders_survive_an_unknown_name() {
        assert_eq!(expand("{{unknown}}", "x", None), "{{unknown}}");
    }

    #[test]
    fn built_in_templates_are_usable() {
        let list = builtin();
        assert!(!list.is_empty());
        for t in &list {
            assert!(!t.name.trim().is_empty(), "a built-in template has no name");
            assert!(!t.body.trim().is_empty(), "{} has no body", t.name);
        }
    }
}
