use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionResult {
    #[serde(rename = "decisionId")]
    pub decision_id: String,
    pub outcome: String,
    pub recommendation: String,
    #[serde(rename = "selectedOption")]
    pub selected_option: Option<String>,
    #[serde(rename = "rejectedOptions")]
    pub rejected_options: Vec<String>,
    pub confidence: f64,
    #[serde(rename = "ruleId", skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    #[serde(rename = "ruleScope", skip_serializing_if = "Option::is_none")]
    pub rule_scope: Option<String>,
    #[serde(rename = "matchScore", skip_serializing_if = "Option::is_none")]
    pub match_score: Option<f64>,
    #[serde(rename = "matchKind", skip_serializing_if = "Option::is_none")]
    pub match_kind: Option<String>,
    #[serde(rename = "riskTier")]
    pub risk_tier: String,
    #[serde(rename = "reasoningSummary")]
    pub reasoning_summary: Vec<String>,
    #[serde(rename = "matchedPolicies")]
    pub matched_policies: Vec<String>,
    #[serde(rename = "restrictionsApplied")]
    pub restrictions_applied: Vec<String>,
    #[serde(rename = "askUserQuestion")]
    pub ask_user_question: Option<String>,
    #[serde(rename = "defaultIfNoAnswer")]
    pub default_if_no_answer: Option<String>,
    #[serde(rename = "learningEvent")]
    pub learning_event: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct DecisionRequest {
    pub intent: String,
    pub situation: String,
    pub options: Vec<String>,
    pub proposed_action: String,
    pub risk: String,
    pub reversible: Option<bool>,
    pub decision_type: String,
    pub scope: String,
    pub agent_confidence: Option<f64>,
    pub dry_run: bool,
}

impl DecisionRequest {
    pub(crate) fn combined(&self) -> String {
        format!(
            "{} {} {} {} {} {}",
            self.intent,
            self.situation,
            self.options.join(" "),
            self.proposed_action,
            self.decision_type,
            self.scope
        )
    }
}

pub struct DecisionEngine<'a> {
    root: &'a Path,
}

impl<'a> DecisionEngine<'a> {
    pub fn new(root: &'a Path) -> Self {
        Self { root }
    }

    pub fn evaluate(&self, request: DecisionRequest) -> Result<DecisionResult> {
        crate::gate::evaluate_internal(self.root, request)
    }
}
