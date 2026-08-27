//! Canonical tool manifests + deterministic per-platform generators.
//!
//! One canonical list drives every export format (OpenAI, Anthropic, Gemini,
//! OpenAPI, generic JSON Schema) so definitions cannot drift. Metadata a
//! platform cannot express (risk, approval, side effects) is preserved in a
//! companion policy document emitted alongside each export.

use interaction_core::{
    Availability, OperationId, RiskClass, ToolId, ToolOperationManifest, ToolRole, SCHEMA_VERSION,
};
use serde_json::{json, Map, Value};

pub const TOOL_NAMESPACE: &str = "interaction";

fn schema_of<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or_else(|_| json!({"type": "object"}))
}

#[allow(clippy::too_many_arguments)]
fn tool(
    op: &str,
    description: &str,
    roles: &[ToolRole],
    risk: RiskClass,
    reversible: bool,
    external: bool,
    input_schema: Value,
    output_schema: Value,
) -> ToolOperationManifest {
    ToolOperationManifest {
        tool: ToolId::new(TOOL_NAMESPACE),
        operation: OperationId::new(op),
        name: format!("{TOOL_NAMESPACE}.{op}"),
        description: description.to_string(),
        roles: roles.to_vec(),
        input_schema,
        output_schema,
        risk,
        reversible,
        external_side_effect: external,
        requires_approval: false,
        permissions: vec![],
        cost: None,
        availability: Availability::Available,
        schema_version: SCHEMA_VERSION.to_string(),
        human: None,
    }
}

/// The canonical tool surface exposed to AI hosts.
pub fn canonical_tools() -> Vec<ToolOperationManifest> {
    use ToolRole::*;
    let obj = |props: Value, required: Value| json!({"type": "object", "properties": props, "required": required, "additionalProperties": false});
    vec![
        tool(
            "status",
            "Check that the interaction runtime is reachable and get its current state (emergency stop, session, counters).",
            &[Receptor],
            RiskClass::ReadOnly,
            true,
            false,
            obj(json!({}), json!([])),
            json!({"type": "object", "description": "Runtime status document"}),
        ),
        tool(
            "capabilities",
            "Discover the current receptors, actuators, tool operations, constraints and session policy. Always call before planning; never assume a device exists.",
            &[Receptor],
            RiskClass::ReadOnly,
            true,
            false,
            obj(
                json!({"includeUnavailable": {"type": "boolean", "default": false}}),
                json!([]),
            ),
            schema_of::<interaction_core::CapabilitySnapshot>(),
        ),
        tool(
            "observe",
            "Query recent observations (optionally one receptor, freshness-bounded). Facts and inferences are separate; treat low-confidence inferences as guesses.",
            &[Receptor],
            RiskClass::ReadOnly,
            true,
            false,
            obj(
                json!({
                    "receptorId": {"type": "string"},
                    "maxAgeMs": {"type": "integer", "minimum": 0},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500},
                    "fresh": {"type": "boolean", "description": "Read live from the receptor instead of the store", "default": false}
                }),
                json!([]),
            ),
            json!({"type": "array", "items": schema_of::<interaction_core::Observation>()}),
        ),
        tool(
            "plan",
            "Create an interaction plan from a semantic intent. The deterministic policy governor bounds every parameter later; magnitudes are suggestions only. The plan may legitimately choose no action.",
            &[Actuator],
            RiskClass::Low,
            true,
            false,
            obj(
                json!({
                    "intent": {"type": "string", "description": "Semantic intent, e.g. celebrate-progress, warning"},
                    "message": {"type": "string"},
                    "magnitude": {"type": "number", "minimum": 0, "maximum": 1},
                    "durationMs": {"type": "integer", "minimum": 0},
                    "preferredChannels": {"type": "array", "items": {"type": "string"}},
                    "candidates": {"type": "array", "items": {"type": "string"}},
                    "minChannels": {"type": "integer", "minimum": 0},
                    "maxChannels": {"type": "integer", "minimum": 1},
                    "allowNoAction": {"type": "boolean", "default": true}
                }),
                json!(["intent"]),
            ),
            schema_of::<interaction_core::Plan>(),
        ),
        tool(
            "simulate",
            "Dry-run a plan through the policy governor: returns per-step decisions and effective bounded parameters without any side effect.",
            &[Receptor],
            RiskClass::ReadOnly,
            true,
            false,
            obj(json!({"planId": {"type": "string"}}), json!(["planId"])),
            json!({"type": "object", "description": "Simulation result with per-step policy decisions"}),
        ),
        tool(
            "execute",
            "Execute an authorized plan. Returns action receipts. IMPORTANT: an accepted/queued receipt is NOT completion — poll action_status or verify.",
            &[Actuator],
            RiskClass::BoundedSideEffect,
            false,
            false,
            obj(
                json!({"planId": {"type": "string"}, "dryRun": {"type": "boolean", "default": false}}),
                json!(["planId"]),
            ),
            json!({"type": "array", "items": schema_of::<interaction_core::ActionReceipt>()}),
        ),
        tool(
            "action_status",
            "Get the full receipt (state machine history) of one action.",
            &[Receptor],
            RiskClass::ReadOnly,
            true,
            false,
            obj(json!({"actionId": {"type": "string"}}), json!(["actionId"])),
            schema_of::<interaction_core::ActionReceipt>(),
        ),
        tool(
            "verify",
            "Re-verify an action against fresh observations and update its receipt (observed / uncertain).",
            &[Receptor, Actuator],
            RiskClass::Low,
            true,
            false,
            obj(json!({"actionId": {"type": "string"}}), json!(["actionId"])),
            schema_of::<interaction_core::ActionReceipt>(),
        ),
        tool(
            "cancel",
            "Cancel a queued or running action.",
            &[Actuator],
            RiskClass::Low,
            false,
            false,
            obj(json!({"actionId": {"type": "string"}}), json!(["actionId"])),
            schema_of::<interaction_core::ActionReceipt>(),
        ),
        tool(
            "stop",
            "EMERGENCY STOP: cancel every open action and halt all actuators immediately. Never requires approval; does not auto-resume.",
            &[Actuator],
            RiskClass::Low,
            false,
            false,
            obj(json!({"reason": {"type": "string"}}), json!([])),
            json!({"type": "object", "description": "Emergency stop acknowledgement"}),
        ),
        tool(
            "recipe_run",
            "Manually run one recipe (its trigger conditions are bypassed; policy still applies).",
            &[Actuator],
            RiskClass::BoundedSideEffect,
            false,
            false,
            obj(json!({"recipeId": {"type": "string"}}), json!(["recipeId"])),
            json!({"type": "object", "description": "Plan + receipts produced by the recipe run"}),
        ),
        tool(
            "policy",
            "Read the effective policy (limits, quiet hours, allowlists, initiative).",
            &[Receptor],
            RiskClass::ReadOnly,
            true,
            false,
            obj(json!({}), json!([])),
            schema_of::<interaction_core::PolicyConfig>(),
        ),
        // ---- 知識系統（spec §12）：AI 讀受限、寫只能 Candidate ----
        tool(
            "knowledge_search",
            "Search the knowledge graph (FTS + lexical-vector CANDIDATE retrieval; results are candidates, not truth). Read-only.",
            &[Receptor],
            RiskClass::ReadOnly,
            true,
            false,
            obj(
                json!({"query": {"type": "string"}, "k": {"type": "integer", "minimum": 1, "maximum": 50}}),
                json!(["query"]),
            ),
            json!({"type": "object", "description": "candidate matches with retrieval scores and usable flags"}),
        ),
        tool(
            "knowledge_get",
            "Read one knowledge node (status shows candidate/active/stale/disputed/superseded honestly).",
            &[Receptor],
            RiskClass::ReadOnly,
            true,
            false,
            obj(json!({"nodeId": {"type": "string"}}), json!(["nodeId"])),
            schema_of::<interaction_core::KnowledgeNode>(),
        ),
        tool(
            "knowledge_get_source",
            "Read a source asset's metadata and (for text) a capped content preview. Raw assets are immutable.",
            &[Receptor],
            RiskClass::ReadOnly,
            true,
            false,
            obj(json!({"hash": {"type": "string"}}), json!(["hash"])),
            json!({"type": "object"}),
        ),
        tool(
            "knowledge_expand_graph",
            "Expand one node's neighborhood (edges + neighbor summaries).",
            &[Receptor],
            RiskClass::ReadOnly,
            true,
            false,
            obj(json!({"root": {"type": "string"}}), json!(["root"])),
            json!({"type": "object"}),
        ),
        tool(
            "knowledge_propose_entity",
            "Propose an ENTITY node. AI proposals always land as CANDIDATE — a human review activates them.",
            &[Actuator],
            RiskClass::Low,
            true,
            false,
            obj(
                json!({"title": {"type": "string"}, "content": {"type": "string"}, "domains": {"type": "array", "items": {"type": "string"}}}),
                json!(["title", "content"]),
            ),
            schema_of::<interaction_core::KnowledgeNode>(),
        ),
        tool(
            "knowledge_propose_claim",
            "Propose a CLAIM with evidence (asset hash/URL + segment REQUIRED). Always a CANDIDATE; never activates itself.",
            &[Actuator],
            RiskClass::Low,
            true,
            false,
            obj(
                json!({
                    "title": {"type": "string"},
                    "content": {"type": "string"},
                    "evidence": {"type": "array", "items": {"type": "object"}},
                    "confidence": {"type": "number"},
                    "domains": {"type": "array", "items": {"type": "string"}},
                    "counterexamples": {"type": "array", "items": {"type": "string"}}
                }),
                json!(["title", "content", "evidence"]),
            ),
            schema_of::<interaction_core::KnowledgeNode>(),
        ),
        tool(
            "knowledge_propose_relation",
            "Propose a typed relation. Analogy/conjecture origins can NEVER claim causality; AI proposals are candidates.",
            &[Actuator],
            RiskClass::Low,
            true,
            false,
            obj(
                json!({
                    "from": {"type": "string"},
                    "to": {"type": "string"},
                    "relation": {"type": "string"},
                    "origin": {"type": "string"},
                    "confidence": {"type": "number"},
                    "rationale": {"type": "string"}
                }),
                json!(["from", "to", "relation", "origin"]),
            ),
            schema_of::<interaction_core::KnowledgeEdge>(),
        ),
        tool(
            "knowledge_propose_supersede",
            "Propose a new version that would supersede an existing node. The old node is only superseded when a HUMAN approves.",
            &[Actuator],
            RiskClass::Low,
            true,
            false,
            obj(
                json!({
                    "supersedes": {"type": "string"},
                    "title": {"type": "string"},
                    "content": {"type": "string"},
                    "evidence": {"type": "array", "items": {"type": "object"}},
                    "domains": {"type": "array", "items": {"type": "string"}},
                    "counterexamples": {"type": "array", "items": {"type": "string"}},
                    "applicability": {"type": "string"},
                    "confidence": {"type": "number"}
                }),
                json!(["supersedes", "title", "content", "evidence", "domains"]),
            ),
            schema_of::<interaction_core::KnowledgeNode>(),
        ),
        tool(
            "knowledge_submit_review",
            "Add a review COMMENT to a candidate. An AI can comment but can never approve/reject — that stays human.",
            &[Actuator],
            RiskClass::Low,
            true,
            false,
            obj(
                json!({"nodeId": {"type": "string"}, "note": {"type": "string"}}),
                json!(["nodeId", "note"]),
            ),
            schema_of::<interaction_core::KnowledgeNode>(),
        ),
    ]
    .into_iter()
    .map(attach_human_meta)
    .collect()
}

/// Formal effect declarations for the canonical tools (spec: builtin tool
/// operations must carry real human meta). Values stay honest: `execute` and
/// `recipe_run` keep `Unknown` physical/interruption facts because the real
/// impact depends on which actuator the governor selects.
fn attach_human_meta(mut m: ToolOperationManifest) -> ToolOperationManifest {
    use interaction_core::{
        ConfirmationLevel, EffectSemantics, HumanMeta, Interruptiveness, TriState,
    };
    let local_effect = |confirmation: ConfirmationLevel| EffectSemantics {
        affects: vec!["runtime-state".into()],
        external_side_effect: TriState::No,
        physical_effect: TriState::No,
        interruptiveness: Interruptiveness::None,
        reversible: TriState::Yes,
        confirmation_level: confirmation,
    };
    let effect = match m.operation.as_str() {
        // Pure local reads/plans complete deterministically in the runtime.
        "status" | "capabilities" | "observe" | "simulate" | "action_status" | "policy"
        | "plan" | "verify" => Some(local_effect(ConfirmationLevel::Completed)),
        // Cancellation is local and deterministic but cannot be un-done.
        "cancel" => Some(EffectSemantics {
            reversible: TriState::No,
            ..local_effect(ConfirmationLevel::Completed)
        }),
        // Emergency stop is local, deterministic, and deliberately sticky.
        "stop" => Some(EffectSemantics {
            reversible: TriState::No,
            ..local_effect(ConfirmationLevel::Completed)
        }),
        // Downstream impact depends on the selected actuators: physical and
        // interruption facts must stay Unknown, receipts only confirm ack.
        "execute" | "recipe_run" => Some(EffectSemantics {
            affects: vec!["selected-actuators".into()],
            external_side_effect: TriState::Unknown,
            physical_effect: TriState::Unknown,
            interruptiveness: Interruptiveness::Unknown,
            reversible: TriState::Unknown,
            confirmation_level: ConfirmationLevel::Acknowledged,
        }),
        _ => None,
    };
    if let Some(effect) = effect {
        m.human = Some(HumanMeta {
            effect: Some(effect),
            ..Default::default()
        });
    }
    m
}

// ---------------------------------------------------------------------------
// Export formats
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    OpenAi,
    Anthropic,
    Gemini,
    OpenApi,
    JsonSchema,
}

impl ExportFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "openai" => Some(Self::OpenAi),
            "anthropic" => Some(Self::Anthropic),
            "gemini" => Some(Self::Gemini),
            "openapi" => Some(Self::OpenApi),
            "json-schema" | "jsonschema" => Some(Self::JsonSchema),
            _ => None,
        }
    }
}

/// Platform-safe name: dots become underscores; stability is load-bearing.
pub fn platform_name(canonical: &str) -> String {
    canonical.replace('.', "_")
}

/// Companion policy document carrying metadata platforms cannot express.
pub fn companion_policy(manifests: &[ToolOperationManifest]) -> Value {
    let entries: Vec<Value> = manifests
        .iter()
        .map(|m| {
            json!({
                "name": platform_name(&m.name),
                "canonicalName": m.name,
                "roles": m.roles,
                "risk": m.risk,
                "reversible": m.reversible,
                "externalSideEffect": m.external_side_effect,
                "requiresApproval": m.requires_approval,
            })
        })
        .collect();
    json!({
        "schemaVersion": SCHEMA_VERSION,
        "note": "Metadata not expressible in the host tool format; hosts should enforce approval flags before invoking.",
        "tools": entries,
    })
}

/// Validation warnings for a manifest set (length limits, collisions).
pub fn validate_manifests(manifests: &[ToolOperationManifest]) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for m in manifests {
        let name = platform_name(&m.name);
        if !seen.insert(name.clone()) {
            warnings.push(format!(
                "duplicate platform name after normalization: {name}"
            ));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            warnings.push(format!("name {name} contains characters some hosts reject"));
        }
        if name.len() > 64 {
            warnings.push(format!("name {name} exceeds 64 chars (OpenAI limit)"));
        }
        if m.description.len() > 1024 {
            warnings.push(format!("{name}: description exceeds 1024 chars"));
        }
        if m.description.trim().is_empty() {
            warnings.push(format!("{name}: description is empty"));
        }
    }
    warnings
}

pub fn export(manifests: &[ToolOperationManifest], format: ExportFormat) -> Value {
    match format {
        ExportFormat::OpenAi => to_openai(manifests),
        ExportFormat::Anthropic => to_anthropic(manifests),
        ExportFormat::Gemini => to_gemini(manifests),
        ExportFormat::OpenApi => to_openapi(manifests),
        ExportFormat::JsonSchema => to_json_schema(manifests),
    }
}

pub fn to_openai(manifests: &[ToolOperationManifest]) -> Value {
    let tools: Vec<Value> = manifests
        .iter()
        .map(|m| {
            json!({
                "type": "function",
                "function": {
                    "name": platform_name(&m.name),
                    "description": m.description,
                    "parameters": m.input_schema,
                }
            })
        })
        .collect();
    json!({"tools": tools, "companionPolicy": companion_policy(manifests)})
}

pub fn to_anthropic(manifests: &[ToolOperationManifest]) -> Value {
    let tools: Vec<Value> = manifests
        .iter()
        .map(|m| {
            json!({
                "name": platform_name(&m.name),
                "description": m.description,
                "input_schema": m.input_schema,
            })
        })
        .collect();
    json!({"tools": tools, "companionPolicy": companion_policy(manifests)})
}

/// Gemini's schema subset: strip keys its function declarations reject.
fn sanitize_for_gemini(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                if matches!(
                    k.as_str(),
                    "$schema" | "$defs" | "$ref" | "additionalProperties" | "default"
                ) {
                    continue;
                }
                out.insert(k.clone(), sanitize_for_gemini(v));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(sanitize_for_gemini).collect()),
        other => other.clone(),
    }
}

pub fn to_gemini(manifests: &[ToolOperationManifest]) -> Value {
    let decls: Vec<Value> = manifests
        .iter()
        .map(|m| {
            json!({
                "name": platform_name(&m.name),
                "description": m.description,
                "parameters": sanitize_for_gemini(&m.input_schema),
            })
        })
        .collect();
    json!({
        "tools": [{"functionDeclarations": decls}],
        "companionPolicy": companion_policy(manifests)
    })
}

pub fn to_json_schema(manifests: &[ToolOperationManifest]) -> Value {
    let mut defs = Map::new();
    for m in manifests {
        defs.insert(
            m.name.clone(),
            json!({
                "description": m.description,
                "input": m.input_schema,
                "output": m.output_schema,
                "risk": m.risk,
                "roles": m.roles,
                "externalSideEffect": m.external_side_effect,
                "requiresApproval": m.requires_approval,
            }),
        );
    }
    json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "title": "interaction tools", "$defs": defs})
}

/// OpenAPI 3.1 document for the HTTP tool-call surface.
pub fn to_openapi(manifests: &[ToolOperationManifest]) -> Value {
    let mut paths = Map::new();
    paths.insert(
        "/v1/health".into(),
        json!({"get": {"summary": "Liveness probe", "responses": {"200": {"description": "OK"}}}}),
    );
    paths.insert(
        "/v1/capabilities".into(),
        json!({"get": {"summary": "Capability snapshot", "responses": {"200": {"description": "Snapshot"}}}}),
    );
    for m in manifests {
        paths.insert(
            format!("/v1/tools/{}/call", m.name),
            json!({
                "post": {
                    "summary": m.description,
                    "operationId": platform_name(&m.name),
                    "x-risk": m.risk,
                    "x-external-side-effect": m.external_side_effect,
                    "requestBody": {
                        "required": true,
                        "content": {"application/json": {"schema": m.input_schema}}
                    },
                    "responses": {
                        "200": {
                            "description": "Tool result",
                            "content": {"application/json": {"schema": m.output_schema}}
                        },
                        "4XX": {"description": "Validation / policy error"}
                    }
                }
            }),
        );
    }
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Adaptive Interaction Runtime",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Local-first adaptive interaction platform (no MCP)."
        },
        "servers": [{"url": "http://127.0.0.1:8787"}],
        "paths": paths,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_set_is_complete() {
        let names: Vec<String> = canonical_tools().iter().map(|t| t.name.clone()).collect();
        for expected in [
            "interaction.status",
            "interaction.capabilities",
            "interaction.observe",
            "interaction.plan",
            "interaction.simulate",
            "interaction.execute",
            "interaction.action_status",
            "interaction.verify",
            "interaction.cancel",
            "interaction.stop",
            "interaction.recipe_run",
            "interaction.policy",
        ] {
            assert!(names.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn generators_are_deterministic() {
        let tools = canonical_tools();
        for format in [
            ExportFormat::OpenAi,
            ExportFormat::Anthropic,
            ExportFormat::Gemini,
            ExportFormat::OpenApi,
            ExportFormat::JsonSchema,
        ] {
            let a = serde_json::to_string(&export(&tools, format)).unwrap();
            let b = serde_json::to_string(&export(&canonical_tools(), format)).unwrap();
            assert_eq!(a, b);
        }
    }

    #[test]
    fn no_validation_warnings_on_canonical_set() {
        let warnings = validate_manifests(&canonical_tools());
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn supersede_schema_requires_provenance_and_domain_scope() {
        let tool = canonical_tools()
            .into_iter()
            .find(|tool| tool.name == "interaction.knowledge_propose_supersede")
            .unwrap();
        let required = tool
            .input_schema
            .get("required")
            .and_then(|value| value.as_array())
            .unwrap();
        for field in ["supersedes", "evidence", "domains"] {
            assert!(required.iter().any(|value| value == field));
        }
    }

    #[test]
    fn names_are_consistent_across_formats() {
        let tools = canonical_tools();
        let openai = to_openai(&tools);
        let anthropic = to_anthropic(&tools);
        let gemini = to_gemini(&tools);
        let openai_names: Vec<&str> = openai["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        let anthropic_names: Vec<&str> = anthropic["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        let gemini_names: Vec<&str> = gemini["tools"][0]["functionDeclarations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(openai_names, anthropic_names);
        assert_eq!(openai_names, gemini_names);
        assert!(openai_names.contains(&"interaction_execute"));
    }

    #[test]
    fn risk_metadata_survives_in_companion_policy() {
        let tools = canonical_tools();
        for format in [
            ExportFormat::OpenAi,
            ExportFormat::Anthropic,
            ExportFormat::Gemini,
        ] {
            let out = export(&tools, format);
            let policy = &out["companionPolicy"]["tools"];
            let exec = policy
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["canonicalName"] == "interaction.execute")
                .expect("execute in companion policy");
            assert_eq!(exec["risk"], "bounded-side-effect");
        }
    }

    #[test]
    fn gemini_schema_is_sanitized() {
        let tools = canonical_tools();
        let out = to_gemini(&tools);
        let s = serde_json::to_string(&out["tools"]).unwrap();
        assert!(!s.contains("$defs"));
        assert!(!s.contains("additionalProperties"));
    }

    #[test]
    fn openapi_covers_every_tool() {
        let tools = canonical_tools();
        let doc = to_openapi(&tools);
        let paths = doc["paths"].as_object().unwrap();
        for t in &tools {
            assert!(paths.contains_key(&format!("/v1/tools/{}/call", t.name)));
        }
    }
}
