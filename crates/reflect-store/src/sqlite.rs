use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use reflect_core::error::{ReflectError, Result};
use reflect_core::storage::Storage;
use reflect_core::types::{
    ErrorPattern, EvalError, EvalSignal, Outcome, OutcomeCounts, Reflection, ReflectionStats,
    ScoredReflection, TagCount, Trend,
};

pub struct SqliteStorage {
    conn: Mutex<Connection>,
}

impl SqliteStorage {
    pub fn open(path: &str) -> Result<Self> {
        let conn =
            Connection::open(path).map_err(|e| ReflectError::Storage(e.to_string()))?;
        let storage = Self {
            conn: Mutex::new(conn),
        };
        storage.init_schema()?;
        Ok(storage)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn =
            Connection::open_in_memory().map_err(|e| ReflectError::Storage(e.to_string()))?;
        let storage = Self {
            conn: Mutex::new(conn),
        };
        storage.init_schema()?;
        Ok(storage)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| ReflectError::Storage(e.to_string()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| ReflectError::Storage(e.to_string()))?;
        conn.execute_batch(include_str!("schema.sql"))
            .map_err(|e| ReflectError::Storage(e.to_string()))?;
        Ok(())
    }

    fn map_err(e: rusqlite::Error) -> ReflectError {
        ReflectError::Storage(e.to_string())
    }
}

fn parse_datetime(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| ReflectError::Storage(format!("invalid datetime: {e}")))
}

fn row_to_reflection(
    row: &rusqlite::Row,
    eval_signals: Vec<EvalSignal>,
) -> std::result::Result<Reflection, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let id = Uuid::parse_str(&id_str)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?;
    let task_description: String = row.get(1)?;
    let draft: String = row.get(2)?;
    let critique: String = row.get(3)?;
    let lesson: String = row.get(4)?;
    let outcome_str: String = row.get(5)?;
    let outcome: Outcome = outcome_str
        .parse()
        .map_err(|e: String| rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e))))?;
    let pattern_id: Option<String> = row.get(6)?;
    let tags_json: String = row.get(7)?;
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
    let confidence: f64 = row.get(8)?;
    let validation_count: i32 = row.get(9)?;
    let contradiction_count: i32 = row.get(10)?;
    let created_at_str: String = row.get(11)?;
    let last_recalled_str: Option<String> = row.get(12)?;

    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(11, rusqlite::types::Type::Text, Box::new(e)))?;
    let last_recalled = last_recalled_str
        .map(|s| {
            DateTime::parse_from_rfc3339(&s)
                .map(|dt| dt.with_timezone(&Utc))
        })
        .transpose()
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(12, rusqlite::types::Type::Text, Box::new(e)))?;

    Ok(Reflection {
        id,
        task_description,
        draft,
        error_signals: eval_signals,
        critique,
        lesson,
        outcome,
        pattern_id,
        tags,
        confidence: confidence as f32,
        validation_count: validation_count as u32,
        contradiction_count: contradiction_count as u32,
        created_at,
        last_recalled,
    })
}

fn load_eval_signals(
    conn: &Connection,
    reflection_id: &str,
) -> std::result::Result<Vec<EvalSignal>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT evaluator, passed, summary, errors_json FROM eval_signals WHERE reflection_id = ?1",
    )?;
    let signals = stmt
        .query_map(params![reflection_id], |row| {
            let evaluator: String = row.get(0)?;
            let passed: bool = row.get(1)?;
            let summary: String = row.get(2)?;
            let errors_json: String = row.get(3)?;
            let errors: Vec<EvalError> = serde_json::from_str(&errors_json).unwrap_or_default();
            Ok(EvalSignal {
                evaluator,
                passed,
                summary,
                errors,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(signals)
}

#[async_trait]
impl Storage for SqliteStorage {
    async fn store_reflection(&self, reflection: &Reflection) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| ReflectError::Storage(e.to_string()))?;
        let id_str = reflection.id.to_string();
        let tags_json =
            serde_json::to_string(&reflection.tags).map_err(|e| ReflectError::Storage(e.to_string()))?;
        let created_at_str = reflection.created_at.to_rfc3339();
        let last_recalled_str = reflection.last_recalled.map(|dt| dt.to_rfc3339());

        conn.execute(
            "INSERT INTO reflections (id, task_description, draft, critique, lesson, outcome, pattern_id, tags, confidence, validation_count, contradiction_count, created_at, last_recalled)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                id_str,
                reflection.task_description,
                reflection.draft,
                reflection.critique,
                reflection.lesson,
                reflection.outcome.as_str(),
                reflection.pattern_id,
                tags_json,
                reflection.confidence as f64,
                reflection.validation_count as i32,
                reflection.contradiction_count as i32,
                created_at_str,
                last_recalled_str,
            ],
        )
        .map_err(SqliteStorage::map_err)?;

        for signal in &reflection.error_signals {
            let errors_json = serde_json::to_string(&signal.errors)
                .map_err(|e| ReflectError::Storage(e.to_string()))?;
            conn.execute(
                "INSERT INTO eval_signals (reflection_id, evaluator, passed, summary, errors_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id_str, signal.evaluator, signal.passed, signal.summary, errors_json],
            )
            .map_err(SqliteStorage::map_err)?;
        }

        Ok(())
    }

    async fn get_reflection(&self, id: &Uuid) -> Result<Option<Reflection>> {
        let conn = self.conn.lock().map_err(|e| ReflectError::Storage(e.to_string()))?;
        let id_str = id.to_string();

        let signals = load_eval_signals(&conn, &id_str).map_err(SqliteStorage::map_err)?;

        let result = conn
            .query_row(
                "SELECT id, task_description, draft, critique, lesson, outcome, pattern_id, tags, confidence, validation_count, contradiction_count, created_at, last_recalled
                 FROM reflections WHERE id = ?1",
                params![id_str],
                |row| row_to_reflection(row, signals.clone()),
            )
            .optional()
            .map_err(SqliteStorage::map_err)?;

        Ok(result)
    }

    async fn delete_reflection(&self, id: &Uuid) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| ReflectError::Storage(e.to_string()))?;
        let id_str = id.to_string();

        // Delete eval_signals first (or rely on CASCADE)
        conn.execute(
            "DELETE FROM eval_signals WHERE reflection_id = ?1",
            params![id_str],
        )
        .map_err(SqliteStorage::map_err)?;

        // We need to manually update FTS before deleting the row, since the trigger
        // reads old.* values, but we also need to handle the content sync.
        // Actually the trigger handles it — just delete.
        let rows = conn
            .execute("DELETE FROM reflections WHERE id = ?1", params![id_str])
            .map_err(SqliteStorage::map_err)?;

        Ok(rows > 0)
    }

    async fn search_reflections(
        &self,
        query: &str,
        tags: &[String],
        limit: usize,
    ) -> Result<Vec<ScoredReflection>> {
        let conn = self.conn.lock().map_err(|e| ReflectError::Storage(e.to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT r.id, r.task_description, r.draft, r.critique, r.lesson, r.outcome,
                        r.pattern_id, r.tags, r.confidence, r.validation_count,
                        r.contradiction_count, r.created_at, r.last_recalled, -fts.rank as score
                 FROM reflections_fts fts
                 JOIN reflections r ON r.rowid = fts.rowid
                 WHERE reflections_fts MATCH ?1
                 ORDER BY rank
                 LIMIT ?2",
            )
            .map_err(SqliteStorage::map_err)?;

        let results = stmt
            .query_map(params![query, limit as i64], |row| {
                let score: f64 = row.get(13)?;
                let reflection = row_to_reflection(row, vec![])?;
                Ok(ScoredReflection {
                    reflection,
                    relevance_score: score,
                })
            })
            .map_err(SqliteStorage::map_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(SqliteStorage::map_err)?;

        // Load eval signals for each result and apply tag filter
        let mut filtered = Vec::new();
        for mut sr in results {
            // Load eval signals
            let signals =
                load_eval_signals(&conn, &sr.reflection.id.to_string()).map_err(SqliteStorage::map_err)?;
            sr.reflection.error_signals = signals;

            // Tag filter: if tags provided, reflection must contain all of them
            if !tags.is_empty() {
                let has_all_tags = tags.iter().all(|tag| sr.reflection.tags.contains(tag));
                if !has_all_tags {
                    continue;
                }
            }
            filtered.push(sr);
        }

        Ok(filtered)
    }

    async fn upsert_pattern(&self, pattern: &ErrorPattern) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| ReflectError::Storage(e.to_string()))?;

        conn.execute(
            "INSERT INTO error_patterns (id, category, description, occurrences, first_seen, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                 category = ?2,
                 description = ?3,
                 occurrences = ?4,
                 first_seen = ?5,
                 last_seen = ?6",
            params![
                pattern.id,
                pattern.category,
                pattern.description,
                pattern.occurrences as i32,
                pattern.first_seen.to_rfc3339(),
                pattern.last_seen.to_rfc3339(),
            ],
        )
        .map_err(SqliteStorage::map_err)?;

        Ok(())
    }

    async fn get_pattern(&self, id: &str) -> Result<Option<ErrorPattern>> {
        let conn = self.conn.lock().map_err(|e| ReflectError::Storage(e.to_string()))?;

        let result = conn
            .query_row(
                "SELECT id, category, description, occurrences, first_seen, last_seen
                 FROM error_patterns WHERE id = ?1",
                params![id],
                |row| {
                    let id: String = row.get(0)?;
                    let category: String = row.get(1)?;
                    let description: String = row.get(2)?;
                    let occurrences: i32 = row.get(3)?;
                    let first_seen_str: String = row.get(4)?;
                    let last_seen_str: String = row.get(5)?;
                    Ok((id, category, description, occurrences, first_seen_str, last_seen_str))
                },
            )
            .optional()
            .map_err(SqliteStorage::map_err)?;

        match result {
            None => Ok(None),
            Some((id, category, description, occurrences, first_seen_str, last_seen_str)) => {
                let first_seen = parse_datetime(&first_seen_str)?;
                let last_seen = parse_datetime(&last_seen_str)?;
                Ok(Some(ErrorPattern {
                    id,
                    category,
                    description,
                    occurrences: occurrences as u32,
                    first_seen,
                    last_seen,
                    reflection_ids: vec![],
                    trend: Trend::Stable,
                }))
            }
        }
    }

    async fn list_patterns(&self, min_occurrences: u32, limit: usize) -> Result<Vec<ErrorPattern>> {
        let conn = self.conn.lock().map_err(|e| ReflectError::Storage(e.to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, category, description, occurrences, first_seen, last_seen
                 FROM error_patterns
                 WHERE occurrences >= ?1
                 ORDER BY occurrences DESC
                 LIMIT ?2",
            )
            .map_err(SqliteStorage::map_err)?;

        let rows = stmt
            .query_map(params![min_occurrences as i32, limit as i64], |row| {
                let id: String = row.get(0)?;
                let category: String = row.get(1)?;
                let description: String = row.get(2)?;
                let occurrences: i32 = row.get(3)?;
                let first_seen_str: String = row.get(4)?;
                let last_seen_str: String = row.get(5)?;
                Ok((id, category, description, occurrences, first_seen_str, last_seen_str))
            })
            .map_err(SqliteStorage::map_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(SqliteStorage::map_err)?;

        let mut patterns = Vec::new();
        for (id, category, description, occurrences, first_seen_str, last_seen_str) in rows {
            let first_seen = parse_datetime(&first_seen_str)?;
            let last_seen = parse_datetime(&last_seen_str)?;
            patterns.push(ErrorPattern {
                id,
                category,
                description,
                occurrences: occurrences as u32,
                first_seen,
                last_seen,
                reflection_ids: vec![],
                trend: Trend::Stable,
            });
        }

        Ok(patterns)
    }

    async fn get_stats(&self) -> Result<ReflectionStats> {
        let conn = self.conn.lock().map_err(|e| ReflectError::Storage(e.to_string()))?;

        let total_reflections: u64 = conn
            .query_row("SELECT COUNT(*) FROM reflections", [], |row| row.get(0))
            .map_err(SqliteStorage::map_err)?;

        let success: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM reflections WHERE outcome = 'success'",
                [],
                |row| row.get(0),
            )
            .map_err(SqliteStorage::map_err)?;

        let failure: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM reflections WHERE outcome = 'failure'",
                [],
                |row| row.get(0),
            )
            .map_err(SqliteStorage::map_err)?;

        let partial: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM reflections WHERE outcome = 'partial'",
                [],
                |row| row.get(0),
            )
            .map_err(SqliteStorage::map_err)?;

        let avg_confidence: f64 = conn
            .query_row(
                "SELECT COALESCE(AVG(confidence), 0.0) FROM reflections",
                [],
                |row| row.get(0),
            )
            .map_err(SqliteStorage::map_err)?;

        let reflections_this_week: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM reflections WHERE created_at >= datetime('now', '-7 days')",
                [],
                |row| row.get(0),
            )
            .map_err(SqliteStorage::map_err)?;

        // Top tags using json_each
        let mut tag_stmt = conn
            .prepare(
                "SELECT value, COUNT(*) as cnt
                 FROM reflections, json_each(reflections.tags)
                 GROUP BY value
                 ORDER BY cnt DESC
                 LIMIT 10",
            )
            .map_err(SqliteStorage::map_err)?;

        let top_tags = tag_stmt
            .query_map([], |row| {
                let tag: String = row.get(0)?;
                let count: u64 = row.get(1)?;
                Ok(TagCount { tag, count })
            })
            .map_err(SqliteStorage::map_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(SqliteStorage::map_err)?;

        // Top patterns
        let mut pat_stmt = conn
            .prepare(
                "SELECT id, category, description, occurrences, first_seen, last_seen
                 FROM error_patterns
                 ORDER BY occurrences DESC
                 LIMIT 5",
            )
            .map_err(SqliteStorage::map_err)?;

        let pattern_rows = pat_stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let category: String = row.get(1)?;
                let description: String = row.get(2)?;
                let occurrences: i32 = row.get(3)?;
                let first_seen_str: String = row.get(4)?;
                let last_seen_str: String = row.get(5)?;
                Ok((id, category, description, occurrences, first_seen_str, last_seen_str))
            })
            .map_err(SqliteStorage::map_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(SqliteStorage::map_err)?;

        let mut top_patterns = Vec::new();
        for (id, category, description, occurrences, first_seen_str, last_seen_str) in pattern_rows
        {
            let first_seen = parse_datetime(&first_seen_str)?;
            let last_seen = parse_datetime(&last_seen_str)?;
            top_patterns.push(ErrorPattern {
                id,
                category,
                description,
                occurrences: occurrences as u32,
                first_seen,
                last_seen,
                reflection_ids: vec![],
                trend: Trend::Stable,
            });
        }

        Ok(ReflectionStats {
            total_reflections,
            by_outcome: OutcomeCounts {
                success,
                failure,
                partial,
            },
            top_patterns,
            top_tags,
            avg_confidence,
            reflections_this_week,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use reflect_core::types::{EvalError, Outcome, Severity};
    use uuid::Uuid;

    fn test_reflection(task: &str, lesson: &str, tags: Vec<String>) -> Reflection {
        Reflection {
            id: Uuid::now_v7(),
            task_description: task.into(),
            draft: "let x = input.parse().unwrap();".into(),
            error_signals: vec![],
            critique: "used unwrap on user input".into(),
            lesson: lesson.into(),
            outcome: Outcome::Failure,
            pattern_id: None,
            tags,
            confidence: 0.5,
            validation_count: 0,
            contradiction_count: 0,
            created_at: Utc::now(),
            last_recalled: None,
        }
    }

    fn storage() -> SqliteStorage {
        SqliteStorage::open_in_memory().expect("failed to open in-memory db")
    }

    #[tokio::test]
    async fn store_and_get_reflection() {
        let store = storage();
        let r = test_reflection("parse user date", "use Result instead of unwrap", vec!["rust".into()]);
        let id = r.id;

        store.store_reflection(&r).await.unwrap();
        let fetched = store.get_reflection(&id).await.unwrap().expect("should exist");

        assert_eq!(fetched.id, id);
        assert_eq!(fetched.task_description, "parse user date");
        assert_eq!(fetched.lesson, "use Result instead of unwrap");
        assert_eq!(fetched.tags, vec!["rust".to_string()]);
        assert_eq!(fetched.outcome, Outcome::Failure);
    }

    #[tokio::test]
    async fn delete_reflection() {
        let store = storage();
        let r = test_reflection("delete test", "lesson", vec![]);
        let id = r.id;

        store.store_reflection(&r).await.unwrap();
        let deleted = store.delete_reflection(&id).await.unwrap();
        assert!(deleted);

        let fetched = store.get_reflection(&id).await.unwrap();
        assert!(fetched.is_none());

        let deleted_again = store.delete_reflection(&id).await.unwrap();
        assert!(!deleted_again);
    }

    #[tokio::test]
    async fn search_fts() {
        let store = storage();
        let r1 = test_reflection("parse user date input", "use chrono for dates", vec!["rust".into()]);
        let r2 = test_reflection("handle network timeout", "add retry logic", vec!["networking".into()]);

        store.store_reflection(&r1).await.unwrap();
        store.store_reflection(&r2).await.unwrap();

        let results = store.search_reflections("date", &[], 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].reflection.task_description, "parse user date input");
        assert!(results[0].relevance_score > 0.0);
    }

    #[tokio::test]
    async fn search_by_tags() {
        let store = storage();
        let r1 = test_reflection("task one", "lesson one", vec!["rust".into(), "error-handling".into()]);
        let r2 = test_reflection("task two", "lesson two", vec!["python".into()]);

        store.store_reflection(&r1).await.unwrap();
        store.store_reflection(&r2).await.unwrap();

        // Search with tag filter — FTS query matches both (via "lesson" in critique/lesson)
        let results = store
            .search_reflections("unwrap", &["rust".into()], 10)
            .await
            .unwrap();

        assert!(results.len() >= 1);
        assert!(results.iter().all(|sr| sr.reflection.tags.contains(&"rust".to_string())));
    }

    #[tokio::test]
    async fn upsert_and_list_patterns() {
        let store = storage();
        let now = Utc::now();

        let pattern = ErrorPattern {
            id: "unwrap-on-input".into(),
            category: "error-handling".into(),
            description: "Using unwrap on user input".into(),
            occurrences: 3,
            first_seen: now,
            last_seen: now,
            reflection_ids: vec![],
            trend: Trend::Stable,
        };

        store.upsert_pattern(&pattern).await.unwrap();

        let fetched = store.get_pattern("unwrap-on-input").await.unwrap().expect("should exist");
        assert_eq!(fetched.occurrences, 3);
        assert_eq!(fetched.category, "error-handling");

        // Upsert with increased occurrences
        let updated = ErrorPattern {
            occurrences: 5,
            ..pattern.clone()
        };
        store.upsert_pattern(&updated).await.unwrap();
        let fetched2 = store.get_pattern("unwrap-on-input").await.unwrap().unwrap();
        assert_eq!(fetched2.occurrences, 5);

        // List with min_occurrences filter
        let all = store.list_patterns(1, 10).await.unwrap();
        assert_eq!(all.len(), 1);

        let filtered = store.list_patterns(10, 10).await.unwrap();
        assert!(filtered.is_empty());
    }

    #[tokio::test]
    async fn store_with_eval_signals() {
        let store = storage();
        let mut r = test_reflection("eval test", "check signals", vec![]);
        r.error_signals = vec![EvalSignal {
            evaluator: "cargo_test".into(),
            passed: false,
            summary: "1 test failed".into(),
            errors: vec![EvalError {
                file: Some("src/main.rs".into()),
                line: Some(42),
                column: None,
                code: None,
                message: "panicked at unwrap".into(),
                severity: Severity::Error,
            }],
        }];
        let id = r.id;

        store.store_reflection(&r).await.unwrap();
        let fetched = store.get_reflection(&id).await.unwrap().unwrap();

        assert_eq!(fetched.error_signals.len(), 1);
        assert_eq!(fetched.error_signals[0].evaluator, "cargo_test");
        assert!(!fetched.error_signals[0].passed);
        assert_eq!(fetched.error_signals[0].errors.len(), 1);
        assert_eq!(fetched.error_signals[0].errors[0].line, Some(42));
        assert_eq!(fetched.error_signals[0].errors[0].message, "panicked at unwrap");
    }

    #[tokio::test]
    async fn get_stats() {
        let store = storage();

        let r1 = test_reflection("task 1", "lesson 1", vec!["rust".into(), "error-handling".into()]);
        let mut r2 = test_reflection("task 2", "lesson 2", vec!["rust".into()]);
        r2.outcome = Outcome::Success;
        r2.confidence = 0.8;

        store.store_reflection(&r1).await.unwrap();
        store.store_reflection(&r2).await.unwrap();

        let stats = store.get_stats().await.unwrap();
        assert_eq!(stats.total_reflections, 2);
        assert_eq!(stats.by_outcome.failure, 1);
        assert_eq!(stats.by_outcome.success, 1);
        assert_eq!(stats.by_outcome.partial, 0);
        assert!((stats.avg_confidence - 0.65).abs() < 0.01);
        assert!(stats.top_tags.iter().any(|t| t.tag == "rust" && t.count == 2));
    }
}
