use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ReflectConfig {
    pub storage: StorageConfig,
    pub eval: HashMap<String, EvalConfig>,
    pub recall: RecallConfig,
    #[serde(default)]
    pub patterns: Vec<PatternConfig>,
}

impl Default for ReflectConfig {
    fn default() -> Self {
        let mut eval = HashMap::new();
        eval.insert(
            "cargo_test".into(),
            EvalConfig {
                command: "cargo test".into(),
                timeout_secs: 60,
            },
        );
        eval.insert(
            "pytest".into(),
            EvalConfig {
                command: "pytest --tb=short -q".into(),
                timeout_secs: 120,
            },
        );
        eval.insert(
            "eslint".into(),
            EvalConfig {
                command: "npx eslint . --format stylish".into(),
                timeout_secs: 60,
            },
        );
        eval.insert(
            "tsc".into(),
            EvalConfig {
                command: "npx tsc --noEmit".into(),
                timeout_secs: 60,
            },
        );
        Self {
            storage: StorageConfig::default(),
            eval,
            recall: RecallConfig::default(),
            patterns: vec![],
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub path: String,
    pub backend: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            path: ".reflect/reflect.db".into(),
            backend: "sqlite".into(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct EvalConfig {
    pub command: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 {
    60
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct RecallConfig {
    pub default_limit: usize,
    pub dedup_threshold: f64,
}

impl Default for RecallConfig {
    fn default() -> Self {
        Self {
            default_limit: 5,
            dedup_threshold: 0.75,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct PatternConfig {
    pub evaluator: String,
    pub regex: String,
    pub id: String,
    pub category: String,
}

pub fn load_config() -> ReflectConfig {
    let paths = if let Ok(p) = std::env::var("REFLECT_CONFIG") {
        vec![PathBuf::from(p)]
    } else {
        vec![
            PathBuf::from("reflect.toml"),
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
                .join(".config/reflect/reflect.toml"),
        ]
    };
    for path in paths {
        if path.exists() {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                if let Ok(config) = toml::from_str(&contents) {
                    return config;
                }
            }
        }
    }
    ReflectConfig::default()
}

pub fn resolve_db_path(config: &ReflectConfig) -> String {
    std::env::var("REFLECT_DB").unwrap_or_else(|_| config.storage.path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let config: ReflectConfig = toml::from_str("").unwrap();
        assert_eq!(config.storage.path, ".reflect/reflect.db");
    }

    #[test]
    fn parse_full_config() {
        let toml_str = r#"
[storage]
path = "/tmp/test.db"
[eval.cargo_test]
command = "cargo test -- --nocapture"
timeout_secs = 120
[recall]
default_limit = 10
"#;
        let config: ReflectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.storage.path, "/tmp/test.db");
        assert_eq!(
            config.eval.get("cargo_test").unwrap().command,
            "cargo test -- --nocapture"
        );
        assert_eq!(config.recall.default_limit, 10);
    }

    #[test]
    fn default_has_all_evaluators() {
        let config = ReflectConfig::default();
        assert!(config.eval.contains_key("cargo_test"));
        assert!(config.eval.contains_key("pytest"));
        assert!(config.eval.contains_key("eslint"));
        assert!(config.eval.contains_key("tsc"));
        assert_eq!(config.eval.get("pytest").unwrap().command, "pytest --tb=short -q");
    }

    #[test]
    fn parse_custom_patterns() {
        let toml_str = r#"
[[patterns]]
evaluator = "cargo_test"
regex = "connection refused"
id = "db-connection-refused"
category = "infrastructure"
"#;
        let config: ReflectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.patterns.len(), 1);
        assert_eq!(config.patterns[0].id, "db-connection-refused");
    }
}
