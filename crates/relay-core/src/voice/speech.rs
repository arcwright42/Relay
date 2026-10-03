//! Shared policy for turning Agent Markdown replies into bounded spoken prose.
use crate::settings::Language;

pub fn spoken_text(text: &str, language: Language) -> String {
    let mut code = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            code = !code;
            continue;
        }
        if !code && !line.is_empty() {
            lines.push(
                line.trim_start_matches(['#', '>', '-', '*', ' '])
                    .replace('`', ""),
            );
        }
    }
    let text = lines.join("\n");
    let suffix = match language {
        Language::SimplifiedChinese => "完整内容已显示在项目会话中。",
        Language::English => "The full response is available in the project conversation.",
    };
    if text.is_empty() || text.chars().count() > 3_000 {
        format!("{}\n{suffix}", text.chars().take(3_000).collect::<String>())
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn code_is_not_read_as_prose_and_long_unicode_answers_are_bounded() {
        assert_eq!(
            spoken_text(
                "# 你好\n```rust\nsecret();\n```\n- 完成",
                Language::SimplifiedChinese
            ),
            "你好\n完成"
        );
        let spoken = spoken_text(&"字".repeat(10_000), Language::SimplifiedChinese);
        assert!(spoken.chars().count() < 3_100);
        assert!(spoken.ends_with("项目会话中。"));
    }
}
