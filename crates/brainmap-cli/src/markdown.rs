use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub path: PathBuf,
    pub id: String,
    pub note_type: String,
    pub status: String,
    pub confidence: String,
    pub risk_tier: String,
    pub sensitivity: String,
    pub title: String,
    pub body: String,
    pub links: Vec<String>,
    pub frontmatter: HashMap<String, String>,
}

pub fn parse_note(path: PathBuf, text: &str) -> Option<Note> {
    if !text.starts_with("---\n") {
        return None;
    }
    let rest = &text[4..];
    let end = rest.find("\n---\n")?;
    let fm_text = &rest[..end];
    let body = rest[end + 5..].to_string();
    let mut fm = HashMap::new();
    for line in fm_text.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let value = v.trim().trim_matches('"').trim_matches('\'').to_string();
            if !value.is_empty() && !line.starts_with(' ') {
                fm.insert(k.trim().to_string(), value);
            }
        }
    }
    let title = body
        .lines()
        .find_map(|line| line.strip_prefix("# ").map(str::to_string))
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("note")
                .replace('-', " ")
        });
    let id = fm.get("id").cloned().unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("note")
            .to_string()
    });
    Some(Note {
        path,
        id,
        note_type: fm
            .get("type")
            .cloned()
            .unwrap_or_else(|| "meta-rule".into()),
        status: fm.get("status").cloned().unwrap_or_else(|| "seed".into()),
        confidence: fm
            .get("confidence")
            .cloned()
            .unwrap_or_else(|| "medium".into()),
        risk_tier: fm
            .get("risk_tier")
            .cloned()
            .unwrap_or_else(|| "suggest-only".into()),
        sensitivity: fm
            .get("sensitivity")
            .cloned()
            .unwrap_or_else(|| "personal".into()),
        links: parse_wikilinks(&body),
        body,
        title,
        frontmatter: fm,
    })
}

pub fn parse_wikilinks(text: &str) -> Vec<String> {
    static LINKS: OnceLock<Regex> = OnceLock::new();
    let re = LINKS.get_or_init(|| Regex::new(r"\[\[([^\]|\n]+)(?:\|[^\]\n]+)?\]\]").unwrap());
    re.captures_iter(text)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().trim().to_string()))
        .collect()
}

pub fn frontmatter(id: &str, note_type: &str, risk_tier: &str, sensitivity: &str) -> String {
    let today = crate::util::today();
    format!(
        r#"---
id: {id}
type: {note_type}
status: seed
confidence: medium
risk_tier: {risk_tier}
sensitivity: {sensitivity}
created: {today}
updated: {today}
last_confirmed:
decay: normal
tags:
  - brainmap
sources: []
---
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wikilinks() {
        assert_eq!(
            parse_wikilinks("[[foo]] [[foo|bar]] [[dir/foo]]"),
            vec!["foo", "foo", "dir/foo"]
        );
    }
}
