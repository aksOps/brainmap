use crate::decision_engine::{DecisionRequest as GateInput, DecisionResult as GateResponse};
use crate::{index, learning, privacy, util};
use anyhow::{Result, bail};
use serde_json::json;
use std::path::Path;
use std::time::Instant;

pub(crate) fn evaluate_internal(root: &Path, input: GateInput) -> Result<GateResponse> {
    if input.dry_run {
        let active_run = crate::dogfood::active_run_context_for_gate(root)?;
        return evaluate_with_context(root, input, active_run).map(|(response, _, _)| response);
    }

    let ledger_path = root.join("90-calibration/decision-ledger.jsonl");
    for _ in 0..3 {
        let evaluated_run = crate::dogfood::active_run_context_for_gate(root)?;
        let (response, event, evaluated_provenance) =
            evaluate_with_context(root, input.clone(), evaluated_run.clone())?;
        let mut ledger = util::lock_jsonl(&ledger_path)?;
        let append_run = crate::dogfood::active_run_context_for_gate(root)?;
        if append_run != evaluated_run {
            drop(ledger);
            continue;
        }
        if append_run.is_some() {
            let Some(expected_provenance) = &evaluated_provenance else {
                drop(ledger);
                continue;
            };
            if crate::dogfood::capture_gate_provenance_version(root)? != *expected_provenance {
                drop(ledger);
                continue;
            }
        }
        ledger.append(
            event
                .as_ref()
                .expect("recorded gate must create a ledger event"),
        )?;
        return Ok(response);
    }
    bail!("dogfood run state changed repeatedly while recording the gate; retry the decision")
}

fn evaluate_with_context(
    root: &Path,
    input: GateInput,
    active_run: Option<crate::dogfood::DogfoodRunContext>,
) -> Result<(
    GateResponse,
    Option<serde_json::Value>,
    Option<crate::dogfood::GateProvenanceVersion>,
)> {
    let evaluation_started = Instant::now();
    let provenance_before = active_run
        .as_ref()
        .map(|_| crate::dogfood::capture_gate_provenance_version(root))
        .transpose()?;
    let combined = privacy::redact(&input.combined());
    let lower = combined.to_lowercase();
    let decision_id = util::id("dec", &combined);
    let autopilot = learning::autopilot_config(root);
    let configured_gate_mode = learning::gate_mode_config(root);
    let provenance_after = active_run
        .as_ref()
        .map(|_| crate::dogfood::capture_gate_provenance_version(root))
        .transpose()?;
    if provenance_before != provenance_after {
        bail!("active dogfood gate provenance changed during evaluation");
    }
    if let Some(run) = &active_run {
        crate::dogfood::validate_gate_provenance_snapshot(
            run,
            &configured_gate_mode,
            &autopilot,
            provenance_after
                .as_ref()
                .expect("active dogfood run must have provenance"),
        )?;
    }
    let dogfood_run_id = active_run.as_ref().map(|run| run.run_id.clone());
    let threshold = autopilot.threshold;
    let learned_resolution = index::resolve_decision_rule(
        root,
        &input.situation,
        &input.decision_type,
        &input.scope,
        &input.options,
    )?;
    let mut matched = Vec::new();
    let mut restrictions = Vec::new();
    let mut outcome = "ask_user".to_string();
    let mut recommendation = "Ask a focused question before proceeding.".to_string();
    let mut selected = None;
    let mut confidence = input.agent_confidence.unwrap_or(0.56).clamp(0.0, 1.0);
    let mut rule_id = None;
    let mut rule_scope = None;
    let mut match_score = None;
    let mut match_kind = None;
    let mut match_margin = None;
    let mut candidate_collision = false;
    let mut risk_tier = "ask_before_action".to_string();
    let mut summary = vec!["Applied deterministic policy precedence.".to_string()];
    let mut learned_question = None;
    let reversible = input.reversible.unwrap_or(false);
    let options = input.options.clone();

    if combined.trim().is_empty()
        || (input.situation.trim().is_empty() && input.proposed_action.trim().is_empty())
    {
        outcome = "needs_more_context".into();
        recommendation = "Provide situation, proposed action, or options.".into();
        confidence = 0.2;
        summary.push("Situation was under-specified.".into());
    } else if privacy::contains_secret(&input.combined()) {
        outcome = "block".into();
        recommendation = "Blocked because the request contains secret-like material.".into();
        confidence = 0.99;
        risk_tier = "never_auto".into();
        restrictions.push("[[40-restrictions/hard-no-rules.md]]".into());
        summary.push("Secrets and safety rules outrank all policies.".into());
    } else if hard_no(&lower) {
        outcome = "block".into();
        recommendation = "Blocked by hard-no or never-auto policy.".into();
        confidence = 0.96;
        risk_tier = "never_auto".into();
        restrictions.push("[[40-restrictions/never-auto.md]]".into());
        summary.push("Request matches never-auto safety class.".into());
    } else if privacy_boundary(&lower) {
        outcome = "ask_user".into();
        recommendation =
            "Ask for explicit approval before crossing privacy or remote-use boundary.".into();
        confidence = 0.9;
        risk_tier = "approval_required".into();
        restrictions.push("[[40-restrictions/privacy-boundaries.md]]".into());
        summary.push("Privacy boundary may apply.".into());
    } else if input.risk.eq_ignore_ascii_case("critical")
        || (input.risk.eq_ignore_ascii_case("high") && !reversible)
    {
        outcome = "ask_user".into();
        recommendation = "Ask before high-risk or irreversible action.".into();
        confidence = 0.86;
        risk_tier = "approval_required".into();
        restrictions.push("[[40-restrictions/approval-required.md]]".into());
        summary.push("Irreversible/high-risk action requires approval.".into());
    } else if input.decision_type == "agent-harness"
        && input.intent.to_ascii_lowercase().contains("pretooluse")
    {
        outcome = "proceed".into();
        selected = options
            .iter()
            .find(|option| option.eq_ignore_ascii_case("proceed"))
            .cloned();
        confidence = 0.95;
        risk_tier = "reversible_auto".into();
        recommendation = "Routine reversible tool action passed the safety-only hook.".into();
        matched.push("[[00-control/approval-policy.md]]".into());
        summary.push(
            "The host hook evaluated only safety and approval boundaries; it did not infer a personal choice."
                .into(),
        );
    } else if input.decision_type == "agent-harness" {
        outcome = "no_action".into();
        confidence = 0.95;
        risk_tier = "suggest_only".into();
        recommendation =
            "No safety intervention; use a structured gate request for personal choices.".into();
        summary.push(
            "The host event carried no structured personal decision for the hook to evaluate."
                .into(),
        );
    } else if let index::DecisionRuleResolution::Applicable(rule) = &learned_resolution {
        let rule_link = format!("[[{}]]", rule.path);
        if !matched.contains(&rule_link) {
            matched.insert(0, rule_link);
        }
        selected = choose_learned_option(&options, &rule.chosen, &rule.rejected);
        confidence = rule.calibrated_confidence();
        rule_id = Some(rule.rule_id.clone());
        rule_scope = Some(rule.scope.clone());
        match_score = Some(rule.score);
        match_kind = Some(rule.match_kind.into());
        match_margin = rule.margin;
        if learned_rule_requires_ask(&rule.chosen) {
            outcome = "ask_user".into();
            recommendation = format!("Follow the learned preference to {}.", rule.chosen);
            risk_tier = "ask_before_action".into();
            learned_question = Some(format!(
                "Should I follow your learned preference to {}?",
                rule.chosen
            ));
        } else if let Some(choice) = &selected {
            outcome = "proceed".into();
            recommendation = format!("Proceed with {choice}; it matches a learned decision.");
            risk_tier = "reversible_auto".into();
        } else {
            outcome = "ask_user".into();
            recommendation = format!(
                "The learned choice '{}' is not among the current options.",
                rule.chosen
            );
            learned_question = Some(format!(
                "Use the learned choice '{}', or select one of the current options?",
                rule.chosen
            ));
        }
        summary.push(format!(
            "Applied compiled learned decision rule with score {:.2}.",
            rule.score
        ));
    } else if let index::DecisionRuleResolution::OptionMismatch(rule) = &learned_resolution {
        let rule_link = format!("[[{}]]", rule.path);
        if !matched.contains(&rule_link) {
            matched.insert(0, rule_link);
        }
        confidence = rule.calibrated_confidence();
        rule_id = Some(rule.rule_id.clone());
        rule_scope = Some(rule.scope.clone());
        match_score = Some(rule.score);
        match_kind = Some(rule.match_kind.into());
        match_margin = rule.margin;
        outcome = "ask_user".into();
        recommendation = format!(
            "The learned choice '{}' is not among the current options.",
            rule.chosen
        );
        learned_question = Some(format!(
            "The available options changed. Which current option should replace '{}'?",
            rule.chosen
        ));
        summary.push(format!(
            "An applicable {} learned rule scored {:.2}, but its choice is unavailable.",
            rule.match_kind, rule.score
        ));
    } else if let index::DecisionRuleResolution::Ambiguous { best, alternative } =
        &learned_resolution
    {
        for rule in [best, alternative] {
            let rule_link = format!("[[{}]]", rule.path);
            if !matched.contains(&rule_link) {
                matched.push(rule_link);
            }
        }
        confidence = best.calibrated_confidence().min(0.65);
        rule_id = Some(best.rule_id.clone());
        rule_scope = Some(best.scope.clone());
        match_score = Some(best.score);
        match_kind = Some("ambiguous".into());
        match_margin = Some((best.score - alternative.score).max(0.0));
        candidate_collision = true;
        outcome = "ask_user".into();
        recommendation = "Conflicting learned decisions are equally relevant.".into();
        learned_question = Some(format!(
            "Should I choose '{}' or '{}' for this situation?",
            best.chosen, alternative.chosen
        ));
        summary.push(format!(
            "Learned rule conflict: {} and {} both scored {:.2}.",
            best.rule_id, alternative.rule_id, best.score
        ));
    } else if ambiguous(&lower, options.len()) {
        outcome = "ask_user".into();
        recommendation = "Ask one narrower question; current context is ambiguous.".into();
        confidence = 0.58;
        matched.push("[[70-question-triggers/ask-when-uncertain.md]]".into());
        summary.push("Ambiguity detected.".into());
    } else if input.intent == "would-ask-user" && lower.contains("redundant") {
        outcome = "no_action".into();
        recommendation = "Suppress redundant question.".into();
        confidence = 0.84;
        risk_tier = "suggest_only".into();
        matched.push("[[70-question-triggers/suppress-redundant-questions.md]]".into());
        summary.push("Question appears redundant.".into());
    } else if input.risk.eq_ignore_ascii_case("low") && reversible {
        outcome = "ask_user".into();
        recommendation =
            "No learned or executable policy selects among the current options.".into();
        summary.push(
            "Option order is non-authoritative; the confidence threshold cannot select a default."
                .into(),
        );
    } else if confidence >= threshold && reversible {
        outcome = "ask_user".into();
        recommendation =
            "Confidence alone cannot select an option without an applicable policy.".into();
        summary.push(
            "Confidence exceeded threshold, but no executable policy selected an option.".into(),
        );
    }

    if outcome == "proceed" && confidence < threshold {
        outcome = "ask_user".into();
        recommendation = format!(
            "Ask before proceeding; confidence {:.2} is below the configured {:.2} threshold.",
            confidence, threshold
        );
        risk_tier = "ask_before_action".into();
        summary.push("Configured confidence threshold forced ask_user.".into());
    }

    let predicted_outcome = outcome.clone();
    let predicted_selected = selected.clone();
    let shadow_mode = configured_gate_mode == "shadow" || autopilot.mode == "shadow";

    if shadow_mode && matches!(outcome.as_str(), "proceed" | "ask_user") {
        if outcome == "proceed" {
            outcome = "ask_user".into();
            recommendation =
                "Shadow mode requires an independent user choice before proceeding.".into();
        }
        selected = None;
        risk_tier = "ask_before_action".into();
        summary.push("Shadow mode recorded the prediction without enforcing its choice.".into());
    } else if (std::env::var("BRAINMAP_DISABLE_AUTOPILOT").ok().as_deref() == Some("1")
        || matches!(
            std::env::var("BRAINMAP_GATE_MODE").ok().as_deref(),
            Some("ask-always" | "suggest-only")
        )
        || autopilot.mode == "disabled"
        || matches!(configured_gate_mode.as_str(), "ask-always" | "suggest-only"))
        && outcome == "proceed"
    {
        outcome = "ask_user".into();
        recommendation = "Autopilot/gate enforcement disabled; ask user.".into();
        risk_tier = "ask_before_action".into();
        summary.push("Kill switch forced ask_user.".into());
    }

    let rejected = options
        .iter()
        .filter(|o| Some(*o) != selected.as_ref())
        .cloned()
        .collect::<Vec<_>>();
    let question = match outcome.as_str() {
        "ask_user" => {
            learned_question.or_else(|| Some(focused_question(&input, selected.as_deref())))
        }
        "needs_more_context" => {
            Some("What situation, options, and risk should I use for this decision?".into())
        }
        _ => None,
    };
    let default_if_no_answer = if outcome == "ask_user" {
        Some("defer or take the cheapest reversible step".into())
    } else {
        None
    };
    let learning_chosen = predicted_selected.clone();
    let response = GateResponse {
        decision_id: decision_id.clone(),
        outcome: outcome.clone(),
        predicted_outcome: predicted_outcome.clone(),
        recommendation,
        selected_option: selected,
        predicted_selected_option: predicted_selected,
        rejected_options: rejected,
        confidence,
        rule_id,
        rule_scope,
        match_score,
        match_kind,
        match_margin,
        candidate_collision,
        risk_tier,
        reasoning_summary: summary,
        matched_policies: matched.clone(),
        applied_policies: matched,
        restrictions_applied: restrictions,
        ask_user_question: question,
        default_if_no_answer,
        gate_mode: configured_gate_mode.clone(),
        autopilot_mode: autopilot.mode.clone(),
        dogfood_run_id,
        learning_event: json!({
            "shouldRecord": !input.dry_run,
            "kind": "decision-gate",
            "situation": privacy::redact(&input.situation),
            "chosen": learning_chosen,
            "confidence": confidence
        }),
    };
    let event = (!input.dry_run).then(|| {
        json!({
            "id": decision_id,
            "createdAt": util::now_iso(),
            "kind": "decision-gate",
            "outcome": outcome,
            "predictedOutcome": response.predicted_outcome,
            "confidence": confidence,
            "intent": privacy::redact(&input.intent),
            "situation": privacy::redact(&input.situation),
            "options": input
                .options
                .iter()
                .map(|option| privacy::redact(option))
                .collect::<Vec<_>>(),
            "proposedAction": privacy::redact(&input.proposed_action),
            "risk": privacy::redact(&input.risk),
            "reversible": input.reversible,
            "decisionType": privacy::redact(&input.decision_type),
            "scope": privacy::redact(&input.scope),
            "selectedOption": response.selected_option.as_deref().map(privacy::redact),
            "predictedSelectedOption": response
                .predicted_selected_option
                .as_deref()
                .map(privacy::redact),
            "rejectedOptions": response
                .rejected_options
                .iter()
                .map(|option| privacy::redact(option))
                .collect::<Vec<_>>(),
            "ruleId": response.rule_id.clone(),
            "ruleScope": response.rule_scope.clone(),
            "matchScore": response.match_score,
            "matchKind": response.match_kind.clone(),
            "matchMargin": response.match_margin,
            "candidateCollision": response.candidate_collision,
            "gateMode": response.gate_mode,
            "autopilotMode": response.autopilot_mode,
            "autopilotLevel": autopilot.level,
            "dogfoodRunId": response.dogfood_run_id,
            "dogfoodThreshold": active_run.as_ref().map(|run| run.threshold),
            "dogfoodCandidateBinarySha256": active_run
                .as_ref()
                .map(|run| run.candidate_binary_sha256.as_str()),
            "evaluationLatencyMicros": evaluation_started.elapsed().as_micros(),
            "appliedPolicies": response.applied_policies.clone(),
            "restrictionsApplied": response.restrictions_applied.clone()
        })
    });
    Ok((response, event, provenance_after))
}

fn hard_no(lower: &str) -> bool {
    let destructive =
        lower.contains("delete") || lower.contains("rm -rf") || lower.contains("destroy");
    let disable_privacy = lower.contains("disable privacy") || lower.contains("ignore privacy");
    let money =
        lower.contains("spend money") || lower.contains("payment") || lower.contains("credit card");
    let remote_private = lower.contains("private memory")
        && (lower.contains("remote") || lower.contains("external"));
    destructive && lower.contains("irreversible") || disable_privacy || money || remote_private
}

fn privacy_boundary(lower: &str) -> bool {
    let remote =
        lower.contains("remote") || lower.contains("external") || lower.contains("model-call");
    let sensitive = lower.contains("private")
        || lower.contains("memory")
        || lower.contains("credential")
        || lower.contains("secret")
        || lower.contains("personal data");
    remote && sensitive
}

fn ambiguous(lower: &str, option_count: usize) -> bool {
    lower.contains("ambiguous")
        || lower.contains("unclear")
        || lower.contains("not sure")
        || (option_count == 0 && lower.len() < 24)
}

fn choose_learned_option(options: &[String], chosen: &str, rejected: &[String]) -> Option<String> {
    let chosen_lower = chosen.to_lowercase();
    options
        .iter()
        .filter(|option| {
            !rejected
                .iter()
                .any(|rejected| option.eq_ignore_ascii_case(rejected))
        })
        .find(|option| {
            let option_lower = option.to_lowercase();
            option_lower == chosen_lower
                || chosen_lower.contains(&option_lower)
                || option_lower.contains(&chosen_lower)
        })
        .cloned()
}

fn learned_rule_requires_ask(chosen: &str) -> bool {
    let lower = chosen.to_lowercase();
    lower.contains("ask user")
        || lower.contains("ask me")
        || lower.contains("ask before")
        || lower.contains("approval")
}

fn focused_question(input: &GateInput, selected: Option<&str>) -> String {
    if input.risk.eq_ignore_ascii_case("high") || input.risk.eq_ignore_ascii_case("critical") {
        return "Do you explicitly approve this high-risk or irreversible action?".into();
    }
    if let Some(selected) = selected {
        format!("Proceed with {selected}, or choose another option?")
    } else if input.options.is_empty() {
        "What option should I compare against the default reversible path?".into()
    } else {
        format!(
            "Which option should be chosen: {}?",
            input.options.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{gate::evaluate, vault};
    use std::fs;

    fn temp_vault() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        crate::learning::autopilot_set(Some(root.clone()), "conservative", "conservative", None)
            .unwrap();
        crate::learning::gate_mode(Some(root.clone()), "active").unwrap();
        index::rebuild(&root).unwrap();
        (tmp, root)
    }

    #[test]
    fn low_risk_storage_proceeds() {
        let (_tmp, root) = temp_vault();
        let res = evaluate(
            &root,
            GateInput {
                intent: "would-ask-user".into(),
                situation: "Choose v1 storage".into(),
                options: vec![
                    "Markdown+JSONL".into(),
                    "SQLite".into(),
                    "External Vector DB".into(),
                ],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "architecture".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();
        assert_eq!(res.outcome, "proceed");
        assert_eq!(res.selected_option.as_deref(), Some("Markdown+JSONL"));
    }

    #[test]
    fn shadow_mode_reports_proceed_prediction_without_enforcing_it() {
        let (_tmp, root) = temp_vault();
        crate::learning::autopilot_set(Some(root.clone()), "shadow", "conservative", None).unwrap();
        crate::learning::gate_mode(Some(root.clone()), "shadow").unwrap();
        let res = evaluate(
            &root,
            GateInput {
                intent: "would-ask-user".into(),
                situation: "Choose v1 storage".into(),
                options: vec![
                    "Markdown+JSONL".into(),
                    "SQLite".into(),
                    "External Vector DB".into(),
                ],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "architecture".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.predicted_outcome, "proceed");
        assert_eq!(
            res.predicted_selected_option.as_deref(),
            Some("Markdown+JSONL")
        );
        assert_eq!(res.outcome, "ask_user");
        assert_eq!(res.selected_option, None);
        assert_eq!(res.gate_mode, "shadow");
        assert_eq!(res.autopilot_mode, "shadow");
        assert_eq!(res.dogfood_run_id, None);
    }

    #[test]
    fn active_dogfood_run_tags_shadow_response_and_ledger() {
        let (_tmp, root) = temp_vault();
        crate::learning::autopilot_set(Some(root.clone()), "shadow", "conservative", None).unwrap();
        crate::learning::gate_mode(Some(root.clone()), "shadow").unwrap();
        let ledger = root.join("90-calibration/decision-ledger.jsonl");
        let boundary = fs::metadata(&ledger)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let started_at = chrono::Utc::now();
        let candidate_binary_sha256 = crate::qualification::running_candidate_hashes()
            .unwrap()
            .brainmap_sha256;
        let candidate_binary_identity = crate::dogfood::current_binary_identity().unwrap();
        util::write_atomic(
            &root.join(".brainmap/dogfood.json"),
            serde_json::to_vec_pretty(&json!({
                "format": "brainmap-dogfood-runs",
                "version": 3,
                "runs": [{
                    "runId": "dogfood_test_run",
                    "status": "active",
                    "candidateCommit": "1111111111111111111111111111111111111111",
                    "candidateBinarySha256": candidate_binary_sha256,
                    "candidateBrainmapdSha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                    "candidateBinaryIdentity": candidate_binary_identity,
                    "host": { "os": std::env::consts::OS, "arch": std::env::consts::ARCH },
                    "adapter": "codex",
                    "startedAt": started_at,
                    "mode": "shadow",
                    "gateMode": "shadow",
                    "autopilotMode": "shadow",
                    "autopilotLevel": "conservative",
                    "threshold": 0.82,
                    "startBackup": { "relativePath": "99-meta/backups/dogfood_test_run-start.brainmap.tar.zst", "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" },
                    "qualificationBundleSha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                    "qualificationManifestSha256": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                    "qualificationBundleRelativePath": ".brainmap/dogfood/dogfood_test_run/qualification",
                    "ledgerBoundaryBytes": boundary,
                    "ledgerBoundaryLines": 0,
                    "ledgerBoundarySha256": util::sha256_hex(&fs::read(&ledger).unwrap_or_default())
                }]
            }))
            .unwrap()
            .as_slice(),
        )
        .unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "would-ask-user".into(),
                situation: "Choose v1 storage".into(),
                options: vec!["Markdown+JSONL".into(), "SQLite".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "architecture".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: false,
            },
        )
        .unwrap();

        assert_eq!(res.dogfood_run_id.as_deref(), Some("dogfood_test_run"));
        let ledger = fs::read_to_string(ledger).unwrap();
        let event: serde_json::Value =
            serde_json::from_str(ledger.lines().last().unwrap()).unwrap();
        assert_eq!(event["dogfoodRunId"], "dogfood_test_run");
        assert_eq!(event["predictedOutcome"], "proceed");
        assert_eq!(event["predictedSelectedOption"], "Markdown+JSONL");
        assert_eq!(event["outcome"], "ask_user");
        assert_eq!(event["selectedOption"], serde_json::Value::Null);
        assert_eq!(event["gateMode"], "shadow");
        assert_eq!(event["autopilotMode"], "shadow");
    }

    #[test]
    fn shadow_false_proceed_feedback_validates_against_the_prediction() {
        let (_tmp, root) = temp_vault();
        crate::learning::autopilot_set(Some(root.clone()), "shadow", "conservative", None).unwrap();
        crate::learning::gate_mode(Some(root.clone()), "shadow").unwrap();
        let response = evaluate(
            &root,
            GateInput {
                intent: "would-ask-user".into(),
                situation: "Choose v1 storage".into(),
                options: vec!["Markdown+JSONL".into(), "SQLite".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "architecture".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: false,
            },
        )
        .unwrap();
        assert_eq!(response.predicted_outcome, "proceed");
        assert_eq!(response.outcome, "ask_user");

        let packet = crate::learning::learn_feedback_quiet(crate::cli::LearnFeedbackArgs {
            decision_id: response.decision_id,
            correction: None,
            chosen: Some("SQLite".into()),
            rejected: Some("Markdown+JSONL".into()),
            incident: Some(crate::cli::FeedbackIncident::FalseProceed),
            vault: Some(root),
        })
        .unwrap();

        assert!(packet.is_some());
    }

    #[test]
    fn seed_storage_preference_is_a_retirable_compiled_markdown_policy() {
        let (_tmp, root) = temp_vault();
        let request = || GateInput {
            intent: "would-ask-user".into(),
            situation: "Choose v1 storage".into(),
            options: vec![
                "Markdown+JSONL".into(),
                "SQLite".into(),
                "External Vector DB".into(),
            ],
            proposed_action: String::new(),
            risk: "low".into(),
            reversible: Some(true),
            decision_type: "architecture".into(),
            scope: "global".into(),
            agent_confidence: None,
            dry_run: true,
        };

        let active = evaluate(&root, request()).unwrap();
        assert_eq!(
            active.rule_id.as_deref(),
            Some("20-decision-frames-architecture-decisions")
        );
        assert_eq!(
            active.applied_policies,
            vec!["[[20-decision-frames/architecture-decisions.md]]"]
        );

        let policy = root.join("20-decision-frames/architecture-decisions.md");
        let retired = fs::read_to_string(&policy)
            .unwrap()
            .replace("status: seed", "status: retired");
        util::write_atomic(&policy, retired.as_bytes()).unwrap();
        index::rebuild(&root).unwrap();

        let inactive = evaluate(&root, request()).unwrap();
        assert_eq!(inactive.outcome, "ask_user");
        assert_eq!(inactive.selected_option, None);
        assert!(inactive.rule_id.is_none());
        assert!(inactive.applied_policies.is_empty());
    }

    #[test]
    fn unlearned_low_risk_decision_does_not_select_the_first_option() {
        let (_tmp, root) = temp_vault();
        let res = evaluate(
            &root,
            GateInput {
                intent: "would-ask-user".into(),
                situation: "Choose a package manager for a new JavaScript project".into(),
                options: vec!["npm".into(), "pnpm".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "tooling".into(),
                scope: "project:example".into(),
                agent_confidence: Some(0.9),
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.outcome, "ask_user");
        assert_eq!(res.selected_option, None);
    }

    #[test]
    fn secrets_block() {
        let (_tmp, root) = temp_vault();
        let res = evaluate(
            &root,
            GateInput {
                intent: "privacy".into(),
                situation: "store api_key=abcdef1234567890".into(),
                options: vec![],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "privacy".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();
        assert_eq!(res.outcome, "block");
    }

    #[test]
    fn ambiguous_asks() {
        let (_tmp, root) = temp_vault();
        let res = evaluate(
            &root,
            GateInput {
                intent: "unknown".into(),
                situation: "unclear thing".into(),
                options: vec![],
                proposed_action: String::new(),
                risk: "medium".into(),
                reversible: Some(false),
                decision_type: "general".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();
        assert_eq!(res.outcome, "ask_user");
        assert!(!res.ask_user_question.unwrap().contains("Brainmap"));
    }

    #[test]
    fn empty_options_question_hides_policy_layer() {
        let (_tmp, root) = temp_vault();
        let res = evaluate(
            &root,
            GateInput {
                intent: "would-ask-user".into(),
                situation: "Need choose publishing flow for finished work".into(),
                options: vec![],
                proposed_action: String::new(),
                risk: "medium".into(),
                reversible: Some(false),
                decision_type: "workflow".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();
        assert_eq!(res.outcome, "ask_user");
        assert!(!res.ask_user_question.unwrap().contains("Brainmap"));
    }

    #[test]
    fn learned_ask_decision_overrides_reversible_default() {
        let (_tmp, root) = temp_vault();
        crate::learning::learn_decision(crate::cli::LearnDecisionArgs {
            situation: "renaming local temporary notes".into(),
            options: "rename automatically|ask user".into(),
            chosen: "ask user".into(),
            rejected: Some("rename automatically".into()),
            rationale: Some("explicit user preference".into()),
            decision_type: "workflow".into(),
            scope: "global".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "renaming local temporary notes".into(),
                options: vec!["rename automatically".into(), "ask user".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "workflow".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.outcome, "ask_user");
        assert_eq!(res.selected_option.as_deref(), Some("ask user"));
    }

    #[test]
    fn learned_rule_does_not_leak_to_a_nearby_decision() {
        let (_tmp, root) = temp_vault();
        crate::learning::learn_decision(crate::cli::LearnDecisionArgs {
            situation: "Choose formatter for a Rust repository".into(),
            options: "rustfmt|a custom formatter".into(),
            chosen: "a custom formatter".into(),
            rejected: Some("rustfmt".into()),
            rationale: Some("repository-specific formatting rules".into()),
            decision_type: "general".into(),
            scope: "global".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "Choose a database for a Rust repository".into(),
                options: vec!["SQLite".into(), "PostgreSQL".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "general".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert!(
            res.matched_policies
                .iter()
                .all(|policy| !policy.contains("60-decision-examples")),
            "an unrelated learned formatter rule was reported as applied: {res:#?}"
        );
    }

    #[test]
    fn learned_rule_applies_to_a_supported_paraphrase() {
        let (_tmp, root) = temp_vault();
        crate::learning::learn_decision(crate::cli::LearnDecisionArgs {
            situation: "Choose formatter for a Rust repository".into(),
            options: "rustfmt|a custom formatter".into(),
            chosen: "a custom formatter".into(),
            rejected: Some("rustfmt".into()),
            rationale: Some("repository-specific formatting rules".into()),
            decision_type: "general".into(),
            scope: "global".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "What formatting tool should this Rust codebase use?".into(),
                options: vec!["rustfmt".into(), "a custom formatter".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "general".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.outcome, "proceed");
        assert_eq!(res.selected_option.as_deref(), Some("a custom formatter"));
    }

    #[test]
    fn option_order_and_repeated_evaluation_do_not_change_substantive_results() {
        let (_tmp, root) = temp_vault();
        crate::learning::learn_decision(crate::cli::LearnDecisionArgs {
            situation: "Choose formatter for deterministic project".into(),
            options: "biome|prettier".into(),
            chosen: "biome".into(),
            rejected: Some("prettier".into()),
            rationale: None,
            decision_type: "tooling".into(),
            scope: "project:deterministic".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();
        let request = |options: Vec<String>| GateInput {
            intent: "plan".into(),
            situation: "Choose formatter for deterministic project".into(),
            options,
            proposed_action: String::new(),
            risk: "low".into(),
            reversible: Some(true),
            decision_type: "tooling".into(),
            scope: "project:deterministic".into(),
            agent_confidence: None,
            dry_run: true,
        };

        let first = evaluate(&root, request(vec!["biome".into(), "prettier".into()])).unwrap();
        let reordered = evaluate(&root, request(vec!["prettier".into(), "biome".into()])).unwrap();
        let repeated = evaluate(&root, request(vec!["biome".into(), "prettier".into()])).unwrap();

        for result in [&first, &reordered, &repeated] {
            assert_eq!(result.selected_option.as_deref(), Some("biome"));
            assert!(
                result
                    .selected_option
                    .as_ref()
                    .is_some_and(|selected| ["biome", "prettier"].contains(&selected.as_str()))
            );
        }
        for result in [&reordered, &repeated] {
            assert_eq!(result.outcome, first.outcome);
            assert_eq!(result.selected_option, first.selected_option);
            assert_eq!(result.confidence, first.confidence);
            assert_eq!(result.rule_id, first.rule_id);
            assert_eq!(result.rule_scope, first.rule_scope);
            assert_eq!(result.match_score, first.match_score);
            assert_eq!(result.match_kind, first.match_kind);
            assert_eq!(result.applied_policies, first.applied_policies);
            assert_eq!(result.restrictions_applied, first.restrictions_applied);
        }
    }

    #[test]
    fn fuzzy_learned_result_exposes_match_evidence_and_calibrated_confidence() {
        let (_tmp, root) = temp_vault();
        crate::learning::learn_decision(crate::cli::LearnDecisionArgs {
            situation: "Choose formatter for a Rust repository".into(),
            options: "rustfmt|a custom formatter".into(),
            chosen: "a custom formatter".into(),
            rejected: Some("rustfmt".into()),
            rationale: Some("project formatting rules".into()),
            decision_type: "tooling".into(),
            scope: "project:alpha".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "Choose formatter for Rust".into(),
                options: vec!["rustfmt".into(), "a custom formatter".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "tooling".into(),
                scope: "project:alpha".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.match_kind.as_deref(), Some("fuzzy"));
        assert_eq!(res.match_score, Some(0.75));
        assert_eq!(res.rule_scope.as_deref(), Some("project:alpha"));
        assert!(res.rule_id.is_some());
        assert!(res.confidence < 0.9, "{res:#?}");
    }

    #[test]
    fn equally_relevant_conflicting_rules_return_ambiguity() {
        let (_tmp, root) = temp_vault();
        for (situation, chosen, rejected) in [
            (
                "Choose primary formatter for a Rust repository",
                "rustfmt",
                "a custom formatter",
            ),
            (
                "Choose preferred formatter for a Rust repository",
                "a custom formatter",
                "rustfmt",
            ),
        ] {
            crate::learning::learn_decision(crate::cli::LearnDecisionArgs {
                situation: situation.into(),
                options: "rustfmt|a custom formatter".into(),
                chosen: chosen.into(),
                rejected: Some(rejected.into()),
                rationale: Some("conflicting learned evidence".into()),
                decision_type: "tooling".into(),
                scope: "project:alpha".into(),
                vault: Some(root.clone()),
            })
            .unwrap();
        }
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "Choose formatter for a Rust repository".into(),
                options: vec!["rustfmt".into(), "a custom formatter".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "tooling".into(),
                scope: "project:alpha".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.outcome, "ask_user");
        assert_eq!(res.selected_option, None);
        assert_eq!(res.match_kind.as_deref(), Some("ambiguous"));
        assert!(
            res.reasoning_summary
                .iter()
                .any(|line| line.contains("conflict"))
        );
    }

    #[test]
    fn project_scoped_rule_does_not_leak_to_another_project() {
        let (_tmp, root) = temp_vault();
        crate::learning::learn_decision(crate::cli::LearnDecisionArgs {
            situation: "Choose formatter for a Rust repository".into(),
            options: "rustfmt|a custom formatter".into(),
            chosen: "a custom formatter".into(),
            rejected: Some("rustfmt".into()),
            rationale: Some("project alpha formatting rules".into()),
            decision_type: "tooling".into(),
            scope: "project:alpha".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "Choose formatter for a Rust repository".into(),
                options: vec!["rustfmt".into(), "a custom formatter".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "tooling".into(),
                scope: "project:beta".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert!(
            res.matched_policies
                .iter()
                .all(|policy| !policy.contains("60-decision-examples")),
            "a project-scoped rule leaked across projects: {res:#?}"
        );
    }

    #[test]
    fn hundred_nearby_decisions_do_not_apply_a_formatter_rule() {
        let (_tmp, root) = temp_vault();
        crate::learning::learn_decision(crate::cli::LearnDecisionArgs {
            situation: "Choose formatter for a Rust repository".into(),
            options: "rustfmt|a custom formatter".into(),
            chosen: "a custom formatter".into(),
            rejected: Some("rustfmt".into()),
            rationale: Some("project formatting rules".into()),
            decision_type: "tooling".into(),
            scope: "project:alpha".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let operations = [
            "database",
            "logging library",
            "package manager",
            "test runner",
            "build system",
            "linter",
            "deployment target",
            "cache",
            "message queue",
            "serializer",
        ];
        let contexts = [
            "Rust repository",
            "Python repository",
            "JavaScript repository",
            "Go repository",
            "Java repository",
            "mobile application",
            "web application",
            "command line project",
            "backend project",
            "frontend project",
        ];

        let mut evaluated = 0usize;
        for operation in operations {
            for context in contexts {
                let res = evaluate(
                    &root,
                    GateInput {
                        intent: "plan".into(),
                        situation: format!("Choose a {operation} for a {context}"),
                        options: vec!["option A".into(), "option B".into()],
                        proposed_action: String::new(),
                        risk: "low".into(),
                        reversible: Some(true),
                        decision_type: "tooling".into(),
                        scope: "project:alpha".into(),
                        agent_confidence: None,
                        dry_run: true,
                    },
                )
                .unwrap();
                assert!(
                    res.rule_id.is_none(),
                    "formatter rule leaked into {operation}/{context}: {res:#?}"
                );
                assert_eq!(res.outcome, "ask_user");
                assert_eq!(res.selected_option, None);
                evaluated += 1;
            }
        }
        assert_eq!(evaluated, 100);
    }

    #[test]
    fn structured_markdown_policy_is_causal_and_retirable() {
        let (_tmp, root) = temp_vault();
        let path = root.join("20-decision-frames/package-manager-policy.md");
        let marker = crate::markdown::decision_rule_marker(&crate::markdown::DecisionRule {
            situation: "Choose package manager for a JavaScript project".into(),
            decision_type: Some("tooling".into()),
            scope: Some("global".into()),
            options: vec!["npm".into(), "pnpm".into()],
            chosen: "pnpm".into(),
            rejected: vec!["npm".into()],
        })
        .unwrap();
        let active = format!(
            "{}# Package Manager Policy\n\n## Deterministic Rule\n\n{}\n",
            crate::markdown::frontmatter(
                "package-manager-policy",
                "decision-policy",
                "reversible-auto",
                "personal",
            )
            .replace("status: seed", "status: reliable"),
            marker
        );
        util::write_atomic(&path, active.as_bytes()).unwrap();
        index::rebuild(&root).unwrap();

        let request = || GateInput {
            intent: "plan".into(),
            situation: "Choose package manager for a JavaScript project".into(),
            options: vec!["npm".into(), "pnpm".into()],
            proposed_action: String::new(),
            risk: "low".into(),
            reversible: Some(true),
            decision_type: "tooling".into(),
            scope: "global".into(),
            agent_confidence: None,
            dry_run: true,
        };
        let active_result = evaluate(&root, request()).unwrap();
        assert_eq!(active_result.selected_option.as_deref(), Some("pnpm"));
        assert_eq!(
            active_result.applied_policies,
            vec!["[[20-decision-frames/package-manager-policy.md]]"]
        );

        let prose_only = active.replace(
            "# Package Manager Policy\n",
            "# Package Manager Policy\n\nHuman context only; see [[30-tradeoff-models/simplicity-vs-power]].\n",
        );
        util::write_atomic(&path, prose_only.as_bytes()).unwrap();
        index::rebuild(&root).unwrap();
        let prose_result = evaluate(&root, request()).unwrap();
        assert_eq!(prose_result.outcome, active_result.outcome);
        assert_eq!(prose_result.selected_option, active_result.selected_option);
        assert_eq!(prose_result.rule_id, active_result.rule_id);
        assert_eq!(prose_result.match_score, active_result.match_score);
        assert_eq!(
            prose_result.applied_policies,
            active_result.applied_policies
        );
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("[[30-tradeoff-models/simplicity-vs-power]]")
        );

        util::write_atomic(
            &path,
            prose_only
                .replace("status: reliable", "status: retired")
                .as_bytes(),
        )
        .unwrap();
        index::rebuild(&root).unwrap();
        let retired_result = evaluate(&root, request()).unwrap();
        assert_eq!(retired_result.outcome, "ask_user");
        assert_eq!(retired_result.selected_option, None);
        assert!(retired_result.applied_policies.is_empty());
    }

    #[test]
    fn decision_ledger_records_replayable_context() {
        let (_tmp, root) = temp_vault();
        let result = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "Choose v1 storage".into(),
                options: vec![
                    "Markdown+JSONL".into(),
                    "SQLite".into(),
                    "External Vector DB".into(),
                ],
                proposed_action: "Use Markdown+JSONL".into(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "architecture".into(),
                scope: "project:brainmap".into(),
                agent_confidence: Some(0.91),
                dry_run: false,
            },
        )
        .unwrap();
        let ledger = fs::read_to_string(root.join("90-calibration/decision-ledger.jsonl")).unwrap();
        let event: serde_json::Value =
            serde_json::from_str(ledger.lines().last().unwrap()).unwrap();

        assert_eq!(event["id"], result.decision_id);
        assert_eq!(event["decisionType"], "architecture");
        assert_eq!(event["scope"], "project:brainmap");
        assert_eq!(event["selectedOption"], "Markdown+JSONL");
        assert_eq!(event["options"].as_array().unwrap().len(), 3);
        assert_eq!(event["appliedPolicies"].as_array().unwrap().len(), 1);
        assert_eq!(event["proposedAction"], "Use Markdown+JSONL");
    }

    #[test]
    fn structured_feedback_preserves_the_original_decision_scope() {
        let (_tmp, root) = temp_vault();
        let request = |scope: &str, dry_run: bool| GateInput {
            intent: "plan".into(),
            situation: "Choose package manager for a JavaScript project".into(),
            options: vec!["npm".into(), "pnpm".into()],
            proposed_action: String::new(),
            risk: "low".into(),
            reversible: Some(true),
            decision_type: "tooling".into(),
            scope: scope.into(),
            agent_confidence: None,
            dry_run,
        };
        let original = evaluate(&root, request("project:alpha", false)).unwrap();

        crate::learning::learn_feedback(crate::cli::LearnFeedbackArgs {
            decision_id: original.decision_id,
            correction: None,
            chosen: Some("npm".into()),
            rejected: Some("pnpm".into()),
            incident: None,
            vault: Some(root.clone()),
        })
        .unwrap();
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let corrected = evaluate(&root, request("project:alpha", true)).unwrap();
        assert_eq!(corrected.outcome, "proceed");
        assert_eq!(corrected.selected_option.as_deref(), Some("npm"));
        assert_eq!(corrected.rule_scope.as_deref(), Some("project:alpha"));

        let other_project = evaluate(&root, request("project:beta", true)).unwrap();
        assert_eq!(other_project.outcome, "ask_user");
        assert_eq!(other_project.rule_id, None);
    }

    #[test]
    fn corrected_feedback_compiles_explicit_choice_and_rejection() {
        let (_tmp, root) = temp_vault();
        let input = || GateInput {
            intent: "plan".into(),
            situation: "renaming local temporary notes".into(),
            options: vec!["rename automatically".into(), "ask user".into()],
            proposed_action: String::new(),
            risk: "low".into(),
            reversible: Some(true),
            decision_type: "workflow".into(),
            scope: "global".into(),
            agent_confidence: None,
            dry_run: false,
        };
        let original = evaluate(&root, input()).unwrap();
        crate::learning::learn_feedback(crate::cli::LearnFeedbackArgs {
            decision_id: original.decision_id,
            correction: Some("never rename automatically; always ask user".into()),
            chosen: None,
            rejected: None,
            incident: None,
            vault: Some(root.clone()),
        })
        .unwrap();
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let corrected = evaluate(
            &root,
            GateInput {
                dry_run: true,
                ..input()
            },
        )
        .unwrap();

        assert_eq!(corrected.outcome, "ask_user");
        assert_eq!(corrected.selected_option.as_deref(), Some("ask user"));
        assert!(
            corrected
                .rejected_options
                .contains(&"rename automatically".into())
        );
    }

    #[test]
    fn persisted_autopilot_disable_forces_ask() {
        let (_tmp, root) = temp_vault();
        crate::learning::autopilot_set(Some(root.clone()), "disabled", "off", None).unwrap();
        crate::learning::autopilot_set_threshold(Some(root.clone()), 0.95).unwrap();
        let config = crate::learning::autopilot_config(&root);
        assert_eq!(config.mode, "disabled");
        assert_eq!(config.level, "off");
        assert_eq!(config.threshold, 0.95);

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "rename a local temporary note".into(),
                options: vec!["rename now".into(), "ask user".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "workflow".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.outcome, "ask_user");
    }

    #[test]
    fn persisted_ask_always_gate_mode_forces_ask() {
        let (_tmp, root) = temp_vault();
        crate::learning::gate_mode(Some(root.clone()), "ask-always").unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "rename a local temporary note".into(),
                options: vec!["rename now".into(), "ask user".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "workflow".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.outcome, "ask_user");
    }

    #[test]
    fn configured_threshold_applies_to_all_proceed_outcomes() {
        let (_tmp, root) = temp_vault();
        crate::learning::autopilot_set(Some(root.clone()), "shadow", "conservative", Some(0.95))
            .unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "rename a local temporary note".into(),
                options: vec!["rename now".into(), "ask user".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "workflow".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.outcome, "ask_user");
        assert!(
            res.reasoning_summary
                .iter()
                .any(|reason| reason.contains("threshold"))
        );
    }
}
