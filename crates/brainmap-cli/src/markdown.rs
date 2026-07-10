use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

const DECISION_RULE_PREFIX: &str = "<!-- brainmap-decision-rule:v1 ";
const DECISION_RULE_SUFFIX: &str = " -->";

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRule {
    pub situation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default)]
    pub options: Vec<String>,
    pub chosen: String,
    #[serde(default)]
    pub rejected: Vec<String>,
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

pub fn decision_rule_marker(rule: &DecisionRule) -> anyhow::Result<String> {
    validate_decision_rule(rule).map_err(anyhow::Error::msg)?;
    Ok(format!(
        "{DECISION_RULE_PREFIX}{}{DECISION_RULE_SUFFIX}",
        serde_json::to_string(rule)?
    ))
}

pub fn parse_decision_rule_result(text: &str) -> Result<Option<DecisionRule>, String> {
    let Some(line) = text
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(DECISION_RULE_PREFIX))
    else {
        return Ok(None);
    };
    let json = line
        .strip_prefix(DECISION_RULE_PREFIX)
        .and_then(|value| value.strip_suffix(DECISION_RULE_SUFFIX))
        .ok_or_else(|| "decision rule marker is missing its closing delimiter".to_string())?;
    let rule: DecisionRule = serde_json::from_str(json)
        .map_err(|error| format!("invalid decision rule JSON: {error}"))?;
    validate_decision_rule(&rule)?;
    Ok(Some(rule))
}

fn validate_decision_rule(rule: &DecisionRule) -> Result<(), String> {
    if rule.situation.trim().is_empty() {
        return Err("decision rule situation is empty".into());
    }
    if rule.chosen.trim().is_empty() {
        return Err("decision rule chosen option is empty".into());
    }
    for (label, value) in [
        ("decision type", rule.decision_type.as_deref()),
        ("scope", rule.scope.as_deref()),
    ] {
        if let Some(value) = value
            && (value.is_empty()
                || value.len() > 160
                || !value.chars().all(|character| {
                    character.is_ascii_alphanumeric()
                        || matches!(character, '-' | '_' | ':' | '/' | '.')
                }))
        {
            return Err(format!("decision rule {label} is invalid"));
        }
    }
    if !rule.options.is_empty()
        && !rule
            .options
            .iter()
            .any(|option| option.eq_ignore_ascii_case(rule.chosen.trim()))
    {
        return Err("decision rule chosen value is not present in options".into());
    }
    if rule
        .rejected
        .iter()
        .any(|choice| choice.eq_ignore_ascii_case(rule.chosen.trim()))
    {
        return Err("decision rule cannot both choose and reject the same option".into());
    }
    Ok(())
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

    #[test]
    fn decision_rule_marker_round_trips_structured_rule() {
        let rule = DecisionRule {
            situation: "publishing finished work: docs".into(),
            decision_type: Some("workflow".into()),
            scope: Some("global".into()),
            options: vec!["publish".into(), "ask user".into()],
            chosen: "ask user".into(),
            rejected: vec!["publish".into()],
        };
        let marker = decision_rule_marker(&rule).unwrap();

        assert_eq!(parse_decision_rule_result(&marker).unwrap(), Some(rule));
    }
}
