use chrono::Utc;
use reflect_core::dedup::is_duplicate_lesson;
use reflect_core::pattern::{PatternEngine, PatternMatch};
use reflect_core::storage::Storage;
use reflect_core::types::*;
use reflect_eval::{run_evaluator, RunnerConfig};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::ServerCapabilities;
use rmcp::{schemars, tool, tool_handler, tool_router, ServerHandler};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::config::ReflectConfig;

// --- Tool input types ---

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EvaluateOutputInput {
    #[schemars(description = "The code being evaluated (stored for reflection context)")]
    pub code: Option<String>,
    #[schemars(description = "Language hint for parser selection")]
    pub language: String,
    #[schemars(description = "List of evaluators to run")]
    pub evaluators: Vec<String>,
    #[schemars(description = "Project root where evaluators run")]
    pub working_dir: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReflectOnOutputInput {
    pub task: String,
    pub draft: String,
    pub signals: Option<Vec<EvalSignalInput>>,
    pub critique: String,
    pub lesson: String,
    pub outcome: String,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct EvalSignalInput {
    pub evaluator: String,
    pub passed: bool,
    pub summary: String,
    pub errors: Option<Vec<EvalErrorInput>>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct EvalErrorInput {
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub code: Option<String>,
    pub message: String,
    pub severity: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StoreReflectionInput {
    pub task: String,
    pub critique: String,
    pub lesson: String,
    pub outcome: String,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RecallReflectionsInput {
    pub task: String,
    pub tags: Option<Vec<String>>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetErrorPatternsInput {
    pub min_occurrences: Option<u32>,
    pub category: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ForgetReflectionInput {
    pub reflection_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetReflectionStatsInput {}

// --- Server ---

#[derive(Clone)]
pub struct ReflectServer {
    storage: Arc<Mutex<Box<dyn Storage>>>,
    pattern_engine: Arc<PatternEngine>,
    config: Arc<ReflectConfig>,
    tool_router: ToolRouter<Self>,
}

// --- Helper conversions ---

fn parse_outcome(s: &str) -> Outcome {
    s.parse().unwrap_or(Outcome::Failure)
}

fn convert_signal_input(input: &EvalSignalInput) -> EvalSignal {
    EvalSignal {
        evaluator: input.evaluator.clone(),
        passed: input.passed,
        summary: input.summary.clone(),
        errors: input
            .errors
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|e| EvalError {
                file: e.file.clone(),
                line: e.line,
                column: e.column,
                code: e.code.clone(),
                message: e.message.clone(),
                severity: match e.severity.as_deref() {
                    Some("warning") => Severity::Warning,
                    _ => Severity::Error,
                },
            })
            .collect(),
    }
}

fn json_ok(value: &impl Serialize) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|e| {
        serde_json::to_string_pretty(&serde_json::json!({
            "error": format!("serialization failed: {e}")
        }))
        .unwrap()
    })
}

fn json_err(msg: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({ "error": msg })).unwrap()
}

// --- Tool implementations ---

#[tool_router]
impl ReflectServer {
    pub fn new(
        storage: Box<dyn Storage>,
        pattern_engine: PatternEngine,
        config: ReflectConfig,
    ) -> Self {
        Self {
            storage: Arc::new(Mutex::new(storage)),
            pattern_engine: Arc::new(pattern_engine),
            config: Arc::new(config),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Run evaluators against code in a project directory. Returns structured pass/fail signals.")]
    async fn evaluate_output(
        &self,
        Parameters(input): Parameters<EvaluateOutputInput>,
    ) -> String {
        let mut signals = Vec::new();

        for evaluator_name in &input.evaluators {
            let eval_config = self.config.eval.get(evaluator_name);
            let (command, timeout_secs) = match eval_config {
                Some(cfg) => (cfg.command.clone(), cfg.timeout_secs),
                None => (evaluator_name.clone(), 60),
            };

            let runner_config = RunnerConfig {
                name: evaluator_name.clone(),
                command,
                args: vec![],
                timeout: Duration::from_secs(timeout_secs),
                working_dir: input.working_dir.clone(),
            };

            match run_evaluator(&runner_config).await {
                Ok(signal) => signals.push(signal),
                Err(e) => {
                    signals.push(EvalSignal {
                        evaluator: evaluator_name.clone(),
                        passed: false,
                        summary: format!("evaluator error: {e}"),
                        errors: vec![],
                    });
                }
            }
        }

        json_ok(&serde_json::json!({ "signals": signals }))
    }

    #[tool(description = "Store a structured reflection with pattern extraction from evaluation signals.")]
    async fn reflect_on_output(
        &self,
        Parameters(input): Parameters<ReflectOnOutputInput>,
    ) -> String {
        let core_signals: Vec<EvalSignal> = input
            .signals
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(convert_signal_input)
            .collect();

        // Extract patterns from signals
        let pattern_matches: Vec<PatternMatch> = self.pattern_engine.extract(&core_signals);

        let storage = self.storage.lock().await;

        // Check for duplicates and handle pattern
        let mut pattern_id: Option<String> = None;
        let mut pattern_occurrences: u32 = 0;
        let mut is_duplicate = false;

        if let Some(first_match) = pattern_matches.first() {
            pattern_id = Some(first_match.id.clone());

            // Check for existing pattern
            let existing_pattern = storage.get_pattern(&first_match.id).await;

            match existing_pattern {
                Ok(Some(mut existing)) => {
                    // Check for duplicate lesson among reflections with this pattern
                    let existing_reflections = storage
                        .search_reflections(&input.task, &[], 10)
                        .await
                        .unwrap_or_default();

                    for sr in &existing_reflections {
                        if sr.reflection.pattern_id.as_deref() == Some(&first_match.id)
                            && is_duplicate_lesson(&sr.reflection.lesson, &input.lesson, self.config.recall.dedup_threshold)
                        {
                            is_duplicate = true;
                            break;
                        }
                    }

                    // Update pattern
                    existing.occurrences += 1;
                    existing.last_seen = Utc::now();
                    pattern_occurrences = existing.occurrences;
                    if let Err(e) = storage.upsert_pattern(&existing).await {
                        return json_err(&format!("failed to update pattern: {e}"));
                    }
                }
                Ok(None) => {
                    // Create new pattern
                    let now = Utc::now();
                    let new_pattern = ErrorPattern {
                        id: first_match.id.clone(),
                        category: first_match.category.clone(),
                        description: first_match.description.clone(),
                        occurrences: 1,
                        first_seen: now,
                        last_seen: now,
                        reflection_ids: vec![],
                        trend: Trend::Stable,
                    };
                    pattern_occurrences = 1;
                    if let Err(e) = storage.upsert_pattern(&new_pattern).await {
                        return json_err(&format!("failed to create pattern: {e}"));
                    }
                }
                Err(e) => {
                    return json_err(&format!("failed to get pattern: {e}"));
                }
            }
        }

        // Store reflection
        let reflection_id = Uuid::now_v7();
        let outcome = parse_outcome(&input.outcome);
        let confidence = confidence_score(0, 0);

        let reflection = Reflection {
            id: reflection_id,
            task_description: input.task,
            draft: input.draft,
            error_signals: core_signals,
            critique: input.critique,
            lesson: input.lesson,
            outcome,
            pattern_id: pattern_id.clone(),
            tags: input.tags.unwrap_or_default(),
            confidence,
            validation_count: 0,
            contradiction_count: 0,
            created_at: Utc::now(),
            last_recalled: None,
        };

        if let Err(e) = storage.store_reflection(&reflection).await {
            return json_err(&format!("failed to store reflection: {e}"));
        }

        json_ok(&serde_json::json!({
            "reflection_id": reflection_id.to_string(),
            "pattern_id": pattern_id,
            "pattern_occurrences": pattern_occurrences,
            "is_duplicate": is_duplicate,
            "confidence": confidence,
        }))
    }

    #[tool(description = "Store a standalone lesson learned without evaluation signals.")]
    async fn store_reflection(
        &self,
        Parameters(input): Parameters<StoreReflectionInput>,
    ) -> String {
        let reflection_id = Uuid::now_v7();
        let outcome = parse_outcome(&input.outcome);

        let reflection = Reflection {
            id: reflection_id,
            task_description: input.task,
            draft: String::new(),
            error_signals: vec![],
            critique: input.critique,
            lesson: input.lesson,
            outcome,
            pattern_id: None,
            tags: input.tags.unwrap_or_default(),
            confidence: confidence_score(0, 0),
            validation_count: 0,
            contradiction_count: 0,
            created_at: Utc::now(),
            last_recalled: None,
        };

        let storage = self.storage.lock().await;
        if let Err(e) = storage.store_reflection(&reflection).await {
            return json_err(&format!("failed to store reflection: {e}"));
        }

        json_ok(&serde_json::json!({
            "reflection_id": reflection_id.to_string(),
            "pattern_id": null,
            "is_duplicate": false,
        }))
    }

    #[tool(description = "Search for relevant past lessons before starting a task.")]
    async fn recall_reflections(
        &self,
        Parameters(input): Parameters<RecallReflectionsInput>,
    ) -> String {
        let limit = input
            .limit
            .unwrap_or(self.config.recall.default_limit);
        let tags = input.tags.unwrap_or_default();

        let storage = self.storage.lock().await;

        let reflections = match storage.search_reflections(&input.task, &tags, limit).await {
            Ok(r) => r,
            Err(e) => return json_err(&format!("search failed: {e}")),
        };

        // Get patterns with 2+ occurrences as "patterns to watch"
        let patterns_to_watch = match storage.list_patterns(2, 10).await {
            Ok(p) => p,
            Err(_) => vec![],
        };

        json_ok(&serde_json::json!({
            "reflections": reflections,
            "patterns_to_watch": patterns_to_watch,
        }))
    }

    #[tool(description = "List recurring error patterns with frequency and trend data.")]
    async fn get_error_patterns(
        &self,
        Parameters(input): Parameters<GetErrorPatternsInput>,
    ) -> String {
        let min_occurrences = input.min_occurrences.unwrap_or(1);
        let limit = input.limit.unwrap_or(20);

        let storage = self.storage.lock().await;

        let patterns = match storage.list_patterns(min_occurrences, limit).await {
            Ok(p) => p,
            Err(e) => return json_err(&format!("failed to list patterns: {e}")),
        };

        // Apply category filter if provided
        let filtered: Vec<&ErrorPattern> = match &input.category {
            Some(cat) => patterns.iter().filter(|p| &p.category == cat).collect(),
            None => patterns.iter().collect(),
        };

        json_ok(&serde_json::json!({ "patterns": filtered }))
    }

    #[tool(description = "Get aggregated reflection statistics.")]
    async fn get_reflection_stats(
        &self,
        Parameters(_input): Parameters<GetReflectionStatsInput>,
    ) -> String {
        let storage = self.storage.lock().await;

        match storage.get_stats().await {
            Ok(stats) => json_ok(&stats),
            Err(e) => json_err(&format!("failed to get stats: {e}")),
        }
    }

    #[tool(description = "Delete a specific reflection by ID.")]
    async fn forget_reflection(
        &self,
        Parameters(input): Parameters<ForgetReflectionInput>,
    ) -> String {
        let id = match Uuid::parse_str(&input.reflection_id) {
            Ok(id) => id,
            Err(e) => return json_err(&format!("invalid UUID: {e}")),
        };

        let storage = self.storage.lock().await;

        match storage.delete_reflection(&id).await {
            Ok(deleted) => json_ok(&serde_json::json!({ "deleted": deleted })),
            Err(e) => json_err(&format!("failed to delete reflection: {e}")),
        }
    }
}

#[tool_handler]
impl ServerHandler for ReflectServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        rmcp::model::ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "reflect: self-correction engine for AI agents. \
                 Use recall_reflections before tasks, \
                 evaluate_output + reflect_on_output after failures.",
            )
    }
}
