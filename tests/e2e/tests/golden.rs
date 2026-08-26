//! Golden tests: every export format is generated from the single canonical
//! manifest and compared byte-for-byte against the committed files under
//! `schemas/`. Regenerate with `GOLDEN_UPDATE=1 cargo test -p interaction-e2e`.

use interaction_tool_schema::{canonical_tools, export, ExportFormat};
use std::path::PathBuf;

fn schemas_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas")
}

fn check_golden(name: &str, content: &str) {
    let path = schemas_dir().join(name);
    if std::env::var("GOLDEN_UPDATE").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!("missing golden {path:?}; run GOLDEN_UPDATE=1 cargo test -p interaction-e2e")
    });
    assert_eq!(
        expected, content,
        "golden {name} drifted; if intentional, regenerate with GOLDEN_UPDATE=1"
    );
}

fn pretty(v: &serde_json::Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap();
    s.push('\n');
    s
}

#[test]
fn golden_tool_exports() {
    let tools = canonical_tools();
    for (format, name) in [
        (ExportFormat::OpenAi, "tools.openai.json"),
        (ExportFormat::Anthropic, "tools.anthropic.json"),
        (ExportFormat::Gemini, "tools.gemini.json"),
        (ExportFormat::OpenApi, "openapi.json"),
        (ExportFormat::JsonSchema, "tools.schema.json"),
    ] {
        let out = export(&tools, format);
        check_golden(name, &pretty(&out));
    }
}

#[test]
fn golden_recipe_schema() {
    let schema = interaction_recipe::recipe_json_schema();
    check_golden("recipe.schema.json", &pretty(&schema));
}

#[test]
fn scenario_j_cross_platform_consistency() {
    let tools = canonical_tools();
    let openai = export(&tools, ExportFormat::OpenAi);
    let anthropic = export(&tools, ExportFormat::Anthropic);
    let gemini = export(&tools, ExportFormat::Gemini);

    let names = |v: &serde_json::Value, path: &[&str]| -> Vec<String> {
        let mut cursor = v;
        for p in path {
            cursor = &cursor[*p];
        }
        cursor
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                t.get("name")
                    .or_else(|| t.get("function").map(|f| &f["name"]))
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect()
    };
    let openai_names = names(&openai, &["tools"]);
    let anthropic_names = names(&anthropic, &["tools"]);
    assert_eq!(openai_names, anthropic_names);
    let gemini_names: Vec<String> = gemini["tools"][0]["functionDeclarations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(openai_names, gemini_names);

    // Input schemas agree (OpenAI parameters == Anthropic input_schema).
    for (o, a) in openai["tools"]
        .as_array()
        .unwrap()
        .iter()
        .zip(anthropic["tools"].as_array().unwrap())
    {
        assert_eq!(o["function"]["parameters"], a["input_schema"]);
    }

    // Risk metadata is preserved for every tool in every companion policy.
    for doc in [&openai, &anthropic, &gemini] {
        let policy_tools = doc["companionPolicy"]["tools"].as_array().unwrap();
        assert_eq!(policy_tools.len(), tools.len());
        assert!(policy_tools.iter().all(|t| t.get("risk").is_some()));
    }
}

/// Scenario K: the workspace has no MCP anywhere — not as a dependency, not as
/// an optional feature. Checked against the lockfile.
#[test]
fn scenario_k_no_mcp_dependency() {
    let lock =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"))
            .expect("Cargo.lock");
    for needle in ["name = \"mcp", "name = \"rmcp", "modelcontextprotocol"] {
        assert!(
            !lock.to_lowercase().contains(needle),
            "MCP-related dependency found in Cargo.lock: {needle}"
        );
    }
}

/// Version discipline: every version-bearing file agrees with the workspace.
#[test]
fn versions_are_in_sync() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workspace = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let version = workspace
        .lines()
        .find_map(|l| l.strip_prefix("version = \""))
        .and_then(|l| l.split('"').next())
        .expect("workspace version");

    let tauri_toml =
        std::fs::read_to_string(root.join("apps/interaction-desktop/src-tauri/Cargo.toml"))
            .unwrap();
    assert!(
        tauri_toml.contains(&format!("version = \"{version}\"")),
        "src-tauri Cargo.toml version != {version}"
    );
    let tauri_conf: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("apps/interaction-desktop/src-tauri/tauri.conf.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(tauri_conf["version"], version, "tauri.conf.json version");
    let pkg: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("apps/interaction-desktop/package.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(pkg["version"], version, "package.json version");

    let changelog = std::fs::read_to_string(root.join("CHANGELOG.md")).unwrap();
    assert!(
        changelog.contains(&format!("## [{version}]")),
        "CHANGELOG.md missing section for {version}"
    );
}
