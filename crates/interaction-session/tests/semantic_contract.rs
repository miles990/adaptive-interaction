//! Shared consumer corpus plus a separately pinned, never-regenerated release corpus.
use interaction_aip::{canonical_hash, canonical_json};
use interaction_session::{
    apply_patch, state_hash, validate_semantic_state, CharacterSession, SessionConfig, Snapshot,
};
use serde_json::{json, Value};
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../interaction-aip/tests/fixtures")
}
fn read(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir().join(name)).unwrap()).unwrap()
}
#[test]
fn shared_consumer_states_and_authoritative_restore_policies() {
    let corpus = read("semantic-state-cases.json");
    let states = corpus["states"].as_array().unwrap();
    assert!(states.len() >= 26);
    for case in states {
        let state: Value = serde_json::from_str(case["wire"].as_str().unwrap()).unwrap();
        assert_eq!(
            validate_semantic_state(&state).is_some(),
            case["accept"].as_bool().unwrap(),
            "{}",
            case["id"]
        );
        assert_eq!(
            canonical_json(&state),
            case["canonical"].as_str().unwrap(),
            "{}",
            case["id"]
        );
        assert_eq!(
            state_hash(&state),
            case["hash"].as_str().unwrap(),
            "{}",
            case["id"]
        );
        if let Some(expected) = case["authoritativeRestore"].as_bool() {
            let config = SessionConfig {
                character_id: "ref-shape".into(),
                ..SessionConfig::default()
            };
            let now = "2026-09-06T00:00:00Z".parse().unwrap();
            let mut snapshot = CharacterSession::new(config.clone(), 1, now).snapshot();
            snapshot.state = state;
            snapshot.hash = state_hash(&snapshot.state);
            assert_eq!(
                CharacterSession::restore(config, &snapshot, now).is_ok(),
                expected,
                "restore {}",
                case["id"]
            );
        }
    }
}
#[test]
fn shared_merge_results_validate_before_atomic_adoption() {
    let corpus = read("semantic-state-cases.json");
    for case in corpus["patches"].as_array().unwrap() {
        let base: Value = serde_json::from_str(case["baseWire"].as_str().unwrap()).unwrap();
        let patch: Value = serde_json::from_str(case["patchWire"].as_str().unwrap()).unwrap();
        let merged = apply_patch(&base, &patch);
        assert_eq!(
            validate_semantic_state(&merged).is_some(),
            case["accept"].as_bool().unwrap(),
            "{}",
            case["id"]
        );
        assert_eq!(canonical_json(&merged), case["canonical"].as_str().unwrap());
        assert_eq!(state_hash(&merged), case["hash"].as_str().unwrap());
        assert!(
            validate_semantic_state(&base).is_some(),
            "failure never mutates base"
        );
    }
}
#[test]
fn published_v070_corpus_is_immutable_and_still_compatible() {
    let index = read("releases/v0.7.0/manifest.json");
    assert_eq!(
        index["sourceSha"],
        "630b4291f6a59444cfb1d8185f757f9dcda9ecc4"
    );
    let mut contents = serde_json::Map::new();
    for file in index["files"].as_array().unwrap() {
        let name = file["file"].as_str().unwrap();
        let text = std::fs::read_to_string(dir().join("releases/v0.7.0").join(name)).unwrap();
        contents.insert(name.into(), json!(text));
        let document: Value = serde_json::from_str(&text).unwrap();
        if name.contains("format0") || name.contains("pre-unsupported") {
            let snapshot: Snapshot = serde_json::from_value(document).unwrap();
            let config = SessionConfig {
                session_id: snapshot.session_id.clone(),
                ..SessionConfig::default()
            };
            assert!(
                CharacterSession::restore(
                    config,
                    &snapshot,
                    "2026-09-06T00:00:00Z".parse().unwrap()
                )
                .is_ok(),
                "{name}"
            );
        } else if name == "state-snapshot.json" {
            assert!(validate_semantic_state(&document["payload"]["state"]).is_some());
            assert_eq!(
                state_hash(&document["payload"]["state"]),
                document["payload"]["hash"]
            );
        }
    }
    // This constant is deliberately outside all GOLDEN_UPDATE/AIP_UPDATE_FIXTURES generators.
    assert_eq!(
        canonical_hash(&Value::Object(contents)),
        "f043a8c9fbbd4eb5441a4069d009cc2c8e31a1b22f0a4c3102b46f356d72e1b0"
    );
}
#[test]
fn schema_never_turns_an_absent_optional_into_a_present_null() {
    let schema = interaction_session::semantic_state_schema();
    let mut state = serde_json::to_value(interaction_session::SemanticState::new(
        "literal null inside a string",
    ))
    .unwrap();
    assert!(validate_semantic_state(&state).is_some());
    state["lastInteraction"] = Value::Null;
    assert!(validate_semantic_state(&state).is_none());
    assert!(schema["properties"]["lastInteraction"]
        .get("anyOf")
        .is_none());
}

#[test]
fn authoritative_restore_rejects_explicit_null_before_it_can_be_discarded() {
    let config = SessionConfig::default();
    let now = "2026-09-06T00:00:00Z".parse().unwrap();
    let mut snapshot = CharacterSession::new(config.clone(), 1, now).snapshot();
    snapshot.state["lastInteraction"] = Value::Null;
    snapshot.hash = state_hash(&snapshot.state);
    assert!(
        CharacterSession::restore(config, &snapshot, now).is_err(),
        "a self-consistent hash does not authorize deleting a state null on restore"
    );
}
