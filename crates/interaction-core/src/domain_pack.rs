//! Versioned built-in domain knowledge packages. A pack is reference data,
//! not executable code and not authority: it can inform a Context Bundle but
//! can never grant consent, widen a session, or become user memory.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DomainPack {
    pub id: String,
    pub display_name: String,
    pub version: String,
    #[serde(default)]
    pub supersedes: Vec<String>,
    pub concepts: Vec<String>,
    pub principles: Vec<String>,
    pub workflow: Vec<String>,
    pub heuristics: Vec<String>,
    pub failure_patterns: Vec<String>,
    pub counterexamples: Vec<String>,
    pub quality_rubric: Vec<String>,
    pub verification: Vec<String>,
    pub sources: Vec<String>,
    pub applicability: Vec<String>,
    pub limitations: Vec<String>,
}

impl DomainPack {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err("domain pack id 必須是小寫 kebab-case".into());
        }
        if self.display_name.trim().is_empty() || self.version.trim().is_empty() {
            return Err("domain pack displayName/version 不得為空".into());
        }
        for (name, values) in [
            ("concepts", &self.concepts),
            ("principles", &self.principles),
            ("workflow", &self.workflow),
            ("heuristics", &self.heuristics),
            ("failurePatterns", &self.failure_patterns),
            ("counterexamples", &self.counterexamples),
            ("qualityRubric", &self.quality_rubric),
            ("verification", &self.verification),
            ("sources", &self.sources),
            ("applicability", &self.applicability),
            ("limitations", &self.limitations),
        ] {
            if values.is_empty() || values.iter().any(|value| value.trim().is_empty()) {
                return Err(format!("domain pack {name} 必須至少有一個非空項目"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_requires_every_knowledge_and_know_how_section() {
        let mut pack = DomainPack {
            id: "test-domain".into(),
            display_name: "Test".into(),
            version: "1.0.0".into(),
            supersedes: vec![],
            concepts: vec!["concept".into()],
            principles: vec!["principle".into()],
            workflow: vec!["step".into()],
            heuristics: vec!["heuristic".into()],
            failure_patterns: vec!["failure".into()],
            counterexamples: vec!["counterexample".into()],
            quality_rubric: vec!["rubric".into()],
            verification: vec!["check".into()],
            sources: vec!["repo://docs/ARCHITECTURE.md".into()],
            applicability: vec!["test".into()],
            limitations: vec!["not authority".into()],
        };
        assert!(pack.validate().is_ok());
        pack.counterexamples.clear();
        assert!(pack.validate().is_err());
    }
}
