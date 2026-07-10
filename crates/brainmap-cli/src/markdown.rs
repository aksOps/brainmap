use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

const DECISION_RULE_PREFIX: &str = "<!-- brainmap-decision-rule:v1 ";
const DECISION_RULE_SUFFIX: &str = " -->";
const DECISION_RULE_SENTINEL: &str = "<!-- brainmap-decision-rule:";

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
    let markers = text
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with(DECISION_RULE_SENTINEL))
        .collect::<Vec<_>>();
    if markers.is_empty() {
        return Ok(None);
    }
    if markers.len() != 1 {
        return Err("executable note must contain exactly one decision rule marker".into());
    }
    let line = markers[0];
    if !line.starts_with(DECISION_RULE_PREFIX) {
        return Err("unsupported decision rule marker version; expected v1".into());
    }
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
    if let Some(decision_type) = rule.decision_type.as_deref() {
        validate_decision_type(decision_type)?;
    }
    if let Some(scope) = rule.scope.as_deref() {
        validate_scope(scope)?;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutableStatus {
    Seed,
    Tested,
    Reliable,
    Retired,
    Stale,
    Contradicted,
}

pub const ACTIVE_EXECUTABLE_STATUSES: [ExecutableStatus; 3] = [
    ExecutableStatus::Seed,
    ExecutableStatus::Tested,
    ExecutableStatus::Reliable,
];

impl ExecutableStatus {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "seed" => Ok(Self::Seed),
            "tested" => Ok(Self::Tested),
            "reliable" => Ok(Self::Reliable),
            "retired" => Ok(Self::Retired),
            "stale" => Ok(Self::Stale),
            "contradicted" => Ok(Self::Contradicted),
            _ => Err(format!("unsupported executable status {value:?}")),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Seed => "seed",
            Self::Tested => "tested",
            Self::Reliable => "reliable",
            Self::Retired => "retired",
            Self::Stale => "stale",
            Self::Contradicted => "contradicted",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExecutableRuleMetadata {
    pub is_control: bool,
    pub priority: i64,
    pub status: ExecutableStatus,
}

#[derive(Debug, Clone, Copy)]
struct ExecutableRuleTypeMetadata {
    is_control: bool,
    priority: i64,
}

pub fn validate_executable_rule_metadata(
    note_type: &str,
    status: &str,
    decision_type: &str,
    scope: &str,
) -> Result<ExecutableRuleMetadata, String> {
    let rule_type = executable_rule_type(note_type)
        .ok_or_else(|| format!("unsupported executable note type {note_type:?}"))?;
    let status = ExecutableStatus::parse(status)?;
    validate_decision_type(decision_type)?;
    validate_scope(scope)?;
    Ok(ExecutableRuleMetadata {
        is_control: rule_type.is_control,
        priority: rule_type.priority,
        status,
    })
}

pub fn validate_executable_note(
    note: &Note,
    decision_type: &str,
    scope: &str,
) -> Result<ExecutableRuleMetadata, String> {
    for field in [
        "id",
        "type",
        "status",
        "confidence",
        "risk_tier",
        "sensitivity",
    ] {
        if !note.frontmatter.contains_key(field) {
            return Err(format!(
                "missing required executable frontmatter field {field:?}"
            ));
        }
    }
    if note.id.is_empty()
        || note.id.len() > 160
        || !note
            .id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("executable note id is invalid".into());
    }
    if !matches!(note.confidence.as_str(), "low" | "medium" | "high") {
        return Err(format!(
            "unsupported executable confidence {:?}",
            note.confidence
        ));
    }
    if !matches!(
        note.risk_tier.as_str(),
        "suggest-only"
            | "ask-before-action"
            | "reversible-auto"
            | "approval-required"
            | "never-auto"
    ) {
        return Err(format!(
            "unsupported executable risk tier {:?}",
            note.risk_tier
        ));
    }
    if !matches!(note.sensitivity.as_str(), "public" | "personal" | "private") {
        return Err(format!(
            "unsupported executable sensitivity {:?}",
            note.sensitivity
        ));
    }
    validate_executable_rule_metadata(&note.note_type, &note.status, decision_type, scope)
}

pub fn executable_rule_type_is_control(note_type: &str) -> bool {
    executable_rule_type(note_type).is_some_and(|metadata| metadata.is_control)
}

fn executable_rule_type(note_type: &str) -> Option<ExecutableRuleTypeMetadata> {
    match note_type {
        "corrected-decision" => Some(ExecutableRuleTypeMetadata {
            is_control: false,
            priority: 300,
        }),
        "decision-policy" | "hard-constraint" | "approval-rule" | "meta-rule" => {
            Some(ExecutableRuleTypeMetadata {
                is_control: true,
                priority: 200,
            })
        }
        "decision-example" => Some(ExecutableRuleTypeMetadata {
            is_control: false,
            priority: 100,
        }),
        _ => None,
    }
}

fn validate_decision_type(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || !value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_')
        })
    {
        return Err("decision rule decision type is invalid".into());
    }
    Ok(())
}

fn validate_scope(value: &str) -> Result<(), String> {
    if value == "global" {
        return Ok(());
    }
    let Some((namespace, identifier)) = value.split_once(':') else {
        return Err("decision rule scope must be global, project:<id>, or repository:<id>".into());
    };
    if !matches!(namespace, "project" | "repository")
        || identifier.is_empty()
        || identifier == "auto"
        || value.len() > 160
        || !identifier.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '/' | '.')
        })
    {
        return Err("decision rule scope must be global, project:<id>, or repository:<id>".into());
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

    #[test]
    fn multiple_decision_rule_markers_are_rejected() {
        let marker = decision_rule_marker(&DecisionRule {
            situation: "Choose a formatter".into(),
            decision_type: Some("tooling".into()),
            scope: Some("project:alpha".into()),
            options: vec!["biome".into(), "prettier".into()],
            chosen: "biome".into(),
            rejected: vec!["prettier".into()],
        })
        .unwrap();
        let error = parse_decision_rule_result(&format!("{marker}\n{marker}\n")).unwrap_err();

        assert!(error.contains("exactly one"));
    }

    #[test]
    fn unsupported_scope_namespace_is_rejected() {
        let error = decision_rule_marker(&DecisionRule {
            situation: "Choose a formatter".into(),
            decision_type: Some("tooling".into()),
            scope: Some("team:alpha".into()),
            options: vec!["biome".into(), "prettier".into()],
            chosen: "biome".into(),
            rejected: vec!["prettier".into()],
        })
        .unwrap_err();

        assert!(error.to_string().contains("scope"));
    }

    #[test]
    fn executable_statuses_follow_the_canonical_ontology() {
        assert!(
            validate_executable_rule_metadata(
                "decision-example",
                "tested",
                "tooling",
                "project:alpha"
            )
            .is_ok()
        );
        assert!(
            validate_executable_rule_metadata(
                "decision-example",
                "active",
                "tooling",
                "project:alpha"
            )
            .is_err()
        );
    }

    #[test]
    fn executable_status_domain_has_an_explicit_active_whitelist() {
        assert_eq!(
            ACTIVE_EXECUTABLE_STATUSES.map(ExecutableStatus::as_str),
            ["seed", "tested", "reliable"]
        );
        for status in ["seed", "tested", "reliable"] {
            assert!(ACTIVE_EXECUTABLE_STATUSES.contains(&ExecutableStatus::parse(status).unwrap()));
        }
        for status in ["retired", "stale", "contradicted"] {
            assert!(
                !ACTIVE_EXECUTABLE_STATUSES.contains(&ExecutableStatus::parse(status).unwrap())
            );
        }
        assert!(ExecutableStatus::parse("future-status").is_err());
    }

    #[test]
    fn unsupported_decision_rule_marker_version_is_rejected() {
        let error =
            parse_decision_rule_result("<!-- brainmap-decision-rule:v2 {\"situation\":\"x\"} -->")
                .unwrap_err();

        assert!(error.contains("unsupported decision rule marker version"));
    }
}
