//! File=Truth configuration.
//!
//! Layout under the interaction home directory (default
//! `~/.adaptive-interaction`, overridable via `INTERACT_AI_HOME` or `--config`):
//!
//! ```text
//! config/
//! ├── interaction.yaml     # runtime config
//! ├── policies/policy.yaml # policy config
//! └── recipes/*.yaml       # recipes
//! state/
//! ├── interaction.db       # sqlite
//! ├── runtime.lock         # instance lock
//! ├── api-token            # human/control-center capability token (0600)
//! └── api-agent-token      # restricted AI/tool capability token (0600)
//! ```
//!
//! Writes are atomic (tmp + rename). Invalid files never crash the runtime:
//! the last known good value stays active and the error is surfaced.

use interaction_core::{DomainError, DomainResult, PolicyConfig};
use interaction_recipe::Recipe;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct RuntimeConfig {
    /// API bind host. Loopback by default; never expose to LAN by default.
    pub api_host: String,
    pub api_port: u16,
    /// Webhook target allowlist (prefix match) for the webhook actuator.
    pub webhook_allowlist: Vec<String>,
    /// Observation retention in hours (privacy).
    pub observation_retention_hours: u32,
    /// Lifecycle mode: `foreground`, `background`, `desktop-managed`, `external-daemon`.
    pub lifecycle: String,
    /// Sweep interval for TTL/watchdog checks, ms.
    pub watchdog_interval_ms: u64,
    /// Default session TTL in minutes (0 = no expiry).
    pub session_ttl_minutes: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            api_host: "127.0.0.1".into(),
            api_port: 8787,
            webhook_allowlist: Vec::new(),
            observation_retention_hours: 72,
            lifecycle: "foreground".into(),
            watchdog_interval_ms: 1_000,
            session_ttl_minutes: 240,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
}

impl Paths {
    pub fn resolve(explicit: Option<&Path>) -> Self {
        let home = explicit
            .map(|p| p.to_path_buf())
            .or_else(|| std::env::var_os("INTERACT_AI_HOME").map(PathBuf::from))
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".adaptive-interaction")
            });
        Self { home }
    }

    pub fn config_dir(&self) -> PathBuf {
        self.home.join("config")
    }
    pub fn runtime_config(&self) -> PathBuf {
        self.config_dir().join("interaction.yaml")
    }
    pub fn policy_file(&self) -> PathBuf {
        self.config_dir().join("policies").join("policy.yaml")
    }
    pub fn recipes_dir(&self) -> PathBuf {
        self.config_dir().join("recipes")
    }
    pub fn state_dir(&self) -> PathBuf {
        self.home.join("state")
    }
    pub fn db_file(&self) -> PathBuf {
        self.state_dir().join("interaction.db")
    }
    pub fn lock_file(&self) -> PathBuf {
        self.state_dir().join("runtime.lock")
    }
    pub fn token_file(&self) -> PathBuf {
        self.state_dir().join("api-token")
    }
    pub fn agent_token_file(&self) -> PathBuf {
        self.state_dir().join("api-agent-token")
    }
    pub fn estop_file(&self) -> PathBuf {
        self.state_dir().join("emergency-stop.requested")
    }
}

/// Atomic write: write to a temp file in the same directory, then rename.
pub fn atomic_write(path: &Path, contents: &str) -> DomainResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| DomainError::Storage(format!("no parent dir for {path:?}")))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| DomainError::Storage(format!("create {parent:?}: {e}")))?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    std::fs::write(&tmp, contents)
        .map_err(|e| DomainError::Storage(format!("write {tmp:?}: {e}")))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| DomainError::Storage(format!("rename {tmp:?} -> {path:?}: {e}")))?;
    Ok(())
}

pub struct ConfigService {
    pub paths: Paths,
}

impl ConfigService {
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }

    /// Load runtime config; missing file = defaults; invalid file = error with
    /// the previous file left untouched.
    pub fn load_runtime_config(&self) -> DomainResult<RuntimeConfig> {
        let path = self.paths.runtime_config();
        if !path.exists() {
            return Ok(RuntimeConfig::default());
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| DomainError::Storage(format!("read {path:?}: {e}")))?;
        serde_yaml::from_str(&raw)
            .map_err(|e| DomainError::Validation(format!("interaction.yaml: {e}")))
    }

    pub fn save_runtime_config(&self, config: &RuntimeConfig) -> DomainResult<()> {
        let yaml = serde_yaml::to_string(config)
            .map_err(|e| DomainError::Internal(format!("serialize runtime config: {e}")))?;
        atomic_write(&self.paths.runtime_config(), &yaml)
    }

    pub fn load_policy(&self) -> DomainResult<PolicyConfig> {
        let path = self.paths.policy_file();
        if !path.exists() {
            return Ok(PolicyConfig::default());
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| DomainError::Storage(format!("read {path:?}: {e}")))?;
        serde_yaml::from_str(&raw).map_err(|e| DomainError::Validation(format!("policy.yaml: {e}")))
    }

    pub fn save_policy(&self, policy: &PolicyConfig) -> DomainResult<()> {
        let yaml = serde_yaml::to_string(policy)
            .map_err(|e| DomainError::Internal(format!("serialize policy: {e}")))?;
        atomic_write(&self.paths.policy_file(), &yaml)
    }

    /// Load all recipes; invalid files are reported, not fatal.
    pub fn load_recipes(&self) -> (Vec<Recipe>, Vec<(PathBuf, String)>) {
        let dir = self.paths.recipes_dir();
        let mut recipes = Vec::new();
        let mut errors = Vec::new();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return (recipes, errors);
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                matches!(
                    p.extension().and_then(|e| e.to_str()),
                    Some("yaml") | Some("yml") | Some("json")
                )
            })
            .collect();
        paths.sort();
        for path in paths {
            match std::fs::read_to_string(&path) {
                Ok(raw) => match interaction_recipe::parse_and_validate(&raw) {
                    Ok(recipe) => recipes.push(recipe),
                    Err(e) => errors.push((path, e.to_string())),
                },
                Err(e) => errors.push((path, e.to_string())),
            }
        }
        (recipes, errors)
    }

    /// Persist a recipe as YAML under its id (path traversal safe).
    pub fn save_recipe(&self, recipe: &Recipe) -> DomainResult<PathBuf> {
        let id = recipe.id.as_str();
        if id.contains('/') || id.contains('\\') || id.contains("..") || id.starts_with('.') {
            return Err(DomainError::Validation(format!(
                "recipe id {id:?} contains path-unsafe characters"
            )));
        }
        let path = self.paths.recipes_dir().join(format!("{id}.yaml"));
        let yaml = serde_yaml::to_string(recipe)
            .map_err(|e| DomainError::Internal(format!("serialize recipe: {e}")))?;
        atomic_write(&path, &yaml)?;
        // Recipes may have been loaded from *.yml / *.json or a file whose
        // basename differs from the id; leaving those behind would fork a
        // duplicate that resurrects the old version on restart.
        for stale in self.recipe_files_with_id(id) {
            if stale != path {
                let _ = std::fs::remove_file(&stale);
            }
        }
        Ok(path)
    }

    /// Every recipe file in the recipes dir whose PARSED id matches — the id
    /// lives in the content, not the filename.
    fn recipe_files_with_id(&self, id: &str) -> Vec<PathBuf> {
        let mut hits = Vec::new();
        let Ok(entries) = std::fs::read_dir(self.paths.recipes_dir()) else {
            return hits;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !matches!(ext, "yaml" | "yml" | "json") {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(recipe) = interaction_recipe::parse_and_validate(&text) {
                    if recipe.id.as_str() == id {
                        hits.push(path);
                    }
                }
            }
        }
        hits
    }

    /// Delete every file backing this recipe id. Idempotent: a recipe that
    /// only ever lived in memory deletes cleanly.
    pub fn delete_recipe(&self, id: &str) -> DomainResult<()> {
        if id.contains('/') || id.contains('\\') || id.contains("..") {
            return Err(DomainError::Validation("path-unsafe recipe id".into()));
        }
        for path in self.recipe_files_with_id(id) {
            std::fs::remove_file(&path)
                .map_err(|e| DomainError::Storage(format!("remove {path:?}: {e}")))?;
        }
        Ok(())
    }

    /// Load or create the local API capability token (0600).
    pub fn load_or_create_token(&self) -> DomainResult<String> {
        self.load_or_create_capability_token(&self.paths.token_file(), "iat-human-")
    }

    /// Restricted token for AI hosts and cross-agent skills. API middleware
    /// rejects human-only control operations even when this token is valid.
    pub fn load_or_create_agent_token(&self) -> DomainResult<String> {
        self.load_or_create_capability_token(&self.paths.agent_token_file(), "iat-agent-")
    }

    fn load_or_create_capability_token(&self, path: &Path, prefix: &str) -> DomainResult<String> {
        if let Ok(existing) = std::fs::read_to_string(path) {
            let token = existing.trim().to_string();
            if !token.is_empty() {
                return Ok(token);
            }
        }
        let token = format!(
            "{prefix}{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        atomic_write(path, &token)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> (tempfile::TempDir, ConfigService) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: dir.path().to_path_buf(),
        };
        (dir, ConfigService::new(paths))
    }

    #[test]
    fn defaults_when_missing() {
        let (_g, svc) = service();
        let cfg = svc.load_runtime_config().unwrap();
        assert_eq!(cfg.api_host, "127.0.0.1");
        let policy = svc.load_policy().unwrap();
        assert!(policy.enabled);
    }

    #[test]
    fn roundtrip_and_atomicity() {
        let (_g, svc) = service();
        let cfg = RuntimeConfig {
            api_port: 9999,
            ..RuntimeConfig::default()
        };
        svc.save_runtime_config(&cfg).unwrap();
        assert_eq!(svc.load_runtime_config().unwrap().api_port, 9999);
        // No stray tmp files.
        let entries: Vec<_> = std::fs::read_dir(svc.paths.config_dir()).unwrap().collect();
        assert!(entries.iter().all(|e| {
            !e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp-")
        }));
    }

    #[test]
    fn invalid_config_is_error_not_panic() {
        let (_g, svc) = service();
        atomic_write(&svc.paths.runtime_config(), "apiPort: [not a number]").unwrap();
        assert!(svc.load_runtime_config().is_err());
    }

    #[test]
    fn invalid_recipe_reported_not_fatal() {
        let (_g, svc) = service();
        atomic_write(&svc.paths.recipes_dir().join("bad.yaml"), "id: [").unwrap();
        let (recipes, errors) = svc.load_recipes();
        assert!(recipes.is_empty());
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn recipe_id_path_traversal_rejected() {
        let (_g, svc) = service();
        let mut recipe: Recipe = serde_yaml::from_str(
            r#"
id: ok
name: n
trigger: { mode: any, steps: [{ receptor: a }] }
decision: { objective: o }
actuation: { candidates: [conversation] }
"#,
        )
        .unwrap();
        recipe.id = interaction_core::RecipeId::new("../evil");
        assert!(svc.save_recipe(&recipe).is_err());
        assert!(svc.delete_recipe("../../etc/passwd").is_err());
    }

    #[test]
    fn token_created_and_stable() {
        let (_g, svc) = service();
        let t1 = svc.load_or_create_token().unwrap();
        let t2 = svc.load_or_create_token().unwrap();
        assert_eq!(t1, t2);
        assert!(t1.starts_with("iat-human-"));
        assert!(t1.len() > 40);

        let a1 = svc.load_or_create_agent_token().unwrap();
        let a2 = svc.load_or_create_agent_token().unwrap();
        assert_eq!(a1, a2);
        assert_ne!(a1, t1);
        assert!(a1.starts_with("iat-agent-"));
    }
}
