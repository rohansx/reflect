use regex::Regex;

use crate::types::EvalSignal;

#[derive(Debug, Clone)]
pub struct PatternMatch {
    pub id: String,
    pub category: String,
    pub description: String,
}

pub struct PatternRule {
    pub evaluator: String,
    pub regex: Regex,
    pub id: String,
    pub category: String,
    pub description: String,
}

pub struct PatternEngine {
    rules: Vec<PatternRule>,
}

impl PatternEngine {
    pub fn new(rules: Vec<PatternRule>) -> Self {
        Self { rules }
    }

    pub fn add_rule(&mut self, rule: PatternRule) {
        self.rules.push(rule);
    }

    /// Extract pattern matches from evaluation signals.
    /// Skips passed signals. Deduplicates by pattern ID.
    pub fn extract(&self, signals: &[EvalSignal]) -> Vec<PatternMatch> {
        let mut matches = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for signal in signals {
            if signal.passed {
                continue;
            }
            for error in &signal.errors {
                for rule in &self.rules {
                    if rule.evaluator == signal.evaluator
                        && rule.regex.is_match(&error.message)
                        && seen.insert(rule.id.clone())
                    {
                        matches.push(PatternMatch {
                            id: rule.id.clone(),
                            category: rule.category.clone(),
                            description: rule.description.clone(),
                        });
                    }
                }
            }
        }
        matches
    }
}

impl Default for PatternEngine {
    fn default() -> Self {
        Self::new(vec![
            PatternRule {
                evaluator: "cargo_test".into(),
                regex: Regex::new(
                    r"unwrap\(\).*Err|called `Result::unwrap\(\)` on an `Err`",
                )
                .unwrap(),
                id: "rust-unwrap-on-parse".into(),
                category: "error_handling".into(),
                description: "Calling unwrap() on a Result that contains Err".into(),
            },
            PatternRule {
                evaluator: "cargo_test".into(),
                regex: Regex::new(r"index out of bounds").unwrap(),
                id: "rust-index-oob".into(),
                category: "bounds_check".into(),
                description: "Array/slice index out of bounds".into(),
            },
            PatternRule {
                evaluator: "cargo_test".into(),
                regex: Regex::new(r"expected .* found").unwrap(),
                id: "rust-type-mismatch".into(),
                category: "type_error".into(),
                description: "Type mismatch: expected one type, found another".into(),
            },
            PatternRule {
                evaluator: "cargo_test".into(),
                regex: Regex::new(r"borrow.*moved").unwrap(),
                id: "rust-use-after-move".into(),
                category: "ownership".into(),
                description: "Use of moved value (ownership violation)".into(),
            },
            // pytest rules
            PatternRule {
                evaluator: "pytest".into(),
                regex: Regex::new(r"Assertion(?:Error)?").unwrap(),
                id: "python-assertion-error".into(),
                category: "assertion".into(),
                description: "Assertion failed in test".into(),
            },
            PatternRule {
                evaluator: "pytest".into(),
                regex: Regex::new(r"TypeError:.*argument").unwrap(),
                id: "python-type-error-args".into(),
                category: "type_error".into(),
                description: "Wrong argument type passed to function".into(),
            },
            PatternRule {
                evaluator: "pytest".into(),
                regex: Regex::new(r"ImportError|ModuleNotFoundError").unwrap(),
                id: "python-import-error".into(),
                category: "import".into(),
                description: "Module import failed".into(),
            },
            // eslint rules
            PatternRule {
                evaluator: "eslint".into(),
                regex: Regex::new(r"no-unused-vars").unwrap(),
                id: "js-unused-vars".into(),
                category: "lint".into(),
                description: "Variable declared but never used".into(),
            },
            PatternRule {
                evaluator: "eslint".into(),
                regex: Regex::new(r"no-explicit-any").unwrap(),
                id: "ts-no-explicit-any".into(),
                category: "type_safety".into(),
                description: "Using 'any' type instead of specific type".into(),
            },
            // tsc rules
            PatternRule {
                evaluator: "tsc".into(),
                regex: Regex::new(r"TS2322.*not assignable").unwrap(),
                id: "ts-type-not-assignable".into(),
                category: "type_error".into(),
                description: "Type is not assignable to target type".into(),
            },
            PatternRule {
                evaluator: "tsc".into(),
                regex: Regex::new(r"TS2304.*Cannot find name").unwrap(),
                id: "ts-cannot-find-name".into(),
                category: "reference".into(),
                description: "Reference to undefined name".into(),
            },
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    fn signal(evaluator: &str, msg: &str) -> EvalSignal {
        EvalSignal {
            evaluator: evaluator.into(),
            passed: false,
            summary: "1 failed".into(),
            errors: vec![EvalError {
                file: None,
                line: None,
                column: None,
                code: None,
                message: msg.into(),
                severity: Severity::Error,
            }],
        }
    }

    #[test]
    fn matches_unwrap_on_parse() {
        let engine = PatternEngine::default();
        let s = signal(
            "cargo_test",
            "panicked at 'called `Result::unwrap()` on an `Err` value'",
        );
        let matches = engine.extract(&[s]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "rust-unwrap-on-parse");
    }

    #[test]
    fn matches_index_oob() {
        let engine = PatternEngine::default();
        let s = signal(
            "cargo_test",
            "index out of bounds: len is 3 but index is 5",
        );
        let matches = engine.extract(&[s]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "rust-index-oob");
    }

    #[test]
    fn matches_type_mismatch() {
        let engine = PatternEngine::default();
        let s = signal("cargo_test", "expected `String` found `&str`");
        let matches = engine.extract(&[s]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "rust-type-mismatch");
    }

    #[test]
    fn matches_use_after_move() {
        let engine = PatternEngine::default();
        let s = signal("cargo_test", "borrow of moved value: `x`");
        let matches = engine.extract(&[s]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "rust-use-after-move");
    }

    #[test]
    fn no_match_on_passing() {
        let engine = PatternEngine::default();
        let s = EvalSignal {
            evaluator: "cargo_test".into(),
            passed: true,
            summary: "3 passed".into(),
            errors: vec![],
        };
        assert!(engine.extract(&[s]).is_empty());
    }

    #[test]
    fn matches_python_assertion() {
        let engine = PatternEngine::default();
        let s = signal("pytest", "AssertionError: assert False is True");
        let matches = engine.extract(&[s]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "python-assertion-error");
    }

    #[test]
    fn matches_eslint_unused_vars() {
        let engine = PatternEngine::default();
        let s = signal("eslint", "'useState' is defined but never used  no-unused-vars");
        let matches = engine.extract(&[s]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "js-unused-vars");
    }

    #[test]
    fn matches_tsc_type_not_assignable() {
        let engine = PatternEngine::default();
        let s = signal("tsc", "TS2322: Type 'string' is not assignable to type 'number'");
        let matches = engine.extract(&[s]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "ts-type-not-assignable");
    }

    #[test]
    fn custom_rule() {
        let mut engine = PatternEngine::default();
        engine.add_rule(PatternRule {
            evaluator: "cargo_test".into(),
            regex: Regex::new("connection refused").unwrap(),
            id: "db-connection-refused".into(),
            category: "infrastructure".into(),
            description: "Database connection refused".into(),
        });
        let s = signal("cargo_test", "connection refused (os error 111)");
        assert!(engine
            .extract(&[s])
            .iter()
            .any(|m| m.id == "db-connection-refused"));
    }
}
