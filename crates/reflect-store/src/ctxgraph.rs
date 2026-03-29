use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use ctxgraph_core::graph::Graph;
use ctxgraph_core::types::{Edge, Entity, Episode};
use uuid::Uuid;

use reflect_core::error::{ReflectError, Result};
use reflect_core::storage::Storage;
use reflect_core::types::{
    ErrorPattern, Outcome, OutcomeCounts, Reflection, ReflectionStats, ScoredReflection, TagCount,
    Trend,
};

pub struct CtxgraphStorage {
    graph: Mutex<Graph>,
}

impl CtxgraphStorage {
    pub fn open(path: &str) -> Result<Self> {
        let db_path = Path::new(path);
        let graph = Graph::open_or_create(db_path)
            .map_err(|e| ReflectError::Storage(format!("ctxgraph: {e}")))?;
        Ok(Self {
            graph: Mutex::new(graph),
        })
    }

    fn map_err(e: impl std::fmt::Display) -> ReflectError {
        ReflectError::Storage(format!("ctxgraph: {e}"))
    }

    fn reflection_to_episode(reflection: &Reflection) -> Result<Episode> {
        let content =
            serde_json::to_string(reflection).map_err(|e| ReflectError::Storage(e.to_string()))?;
        Ok(Episode {
            id: reflection.id.to_string(),
            content,
            source: Some("reflect".into()),
            recorded_at: reflection.created_at,
            metadata: Some(serde_json::json!({
                "outcome": reflection.outcome.as_str(),
                "tags": reflection.tags,
                "confidence": reflection.confidence,
                "pattern_id": reflection.pattern_id,
            })),
        })
    }

    fn episode_to_reflection(episode: &Episode) -> Result<Reflection> {
        serde_json::from_str(&episode.content)
            .map_err(|e| ReflectError::Storage(format!("deserialize reflection: {e}")))
    }
}

#[async_trait]
impl Storage for CtxgraphStorage {
    async fn store_reflection(&self, reflection: &Reflection) -> Result<()> {
        let episode = Self::reflection_to_episode(reflection)?;
        let graph = self.graph.lock().map_err(Self::map_err)?;
        let result = graph.add_episode(episode).map_err(Self::map_err)?;

        // If reflection has a pattern_id, create an edge linking episode to pattern entity
        if let Some(ref pattern_id) = reflection.pattern_id {
            let edge = Edge::new(&result.episode_id, pattern_id, "matched_pattern");
            // Best-effort: ignore edge creation failure (pattern entity may not exist yet)
            let _ = graph.add_edge(edge);
        }

        Ok(())
    }

    async fn get_reflection(&self, id: &Uuid) -> Result<Option<Reflection>> {
        let graph = self.graph.lock().map_err(Self::map_err)?;
        let episode = graph.get_episode(&id.to_string()).map_err(Self::map_err)?;
        match episode {
            Some(ep) => Ok(Some(Self::episode_to_reflection(&ep)?)),
            None => Ok(None),
        }
    }

    async fn delete_reflection(&self, _id: &Uuid) -> Result<bool> {
        Err(ReflectError::Storage(
            "delete not supported by ctxgraph backend".into(),
        ))
    }

    async fn search_reflections(
        &self,
        query: &str,
        tags: &[String],
        limit: usize,
    ) -> Result<Vec<ScoredReflection>> {
        let graph = self.graph.lock().map_err(Self::map_err)?;

        // Use a larger pool when filtering by tags, since we filter post-search
        let search_limit = if tags.is_empty() { limit } else { limit * 5 };

        let results = graph.search(query, search_limit).map_err(Self::map_err)?;

        let mut scored = Vec::new();
        for (episode, score) in results {
            let reflection = match Self::episode_to_reflection(&episode) {
                Ok(r) => r,
                Err(_) => continue, // skip non-reflection episodes
            };

            // Filter by tags if specified
            if !tags.is_empty() && !tags.iter().any(|t| reflection.tags.contains(t)) {
                continue;
            }

            scored.push(ScoredReflection {
                reflection,
                relevance_score: score,
            });

            if scored.len() >= limit {
                break;
            }
        }

        Ok(scored)
    }

    async fn upsert_pattern(&self, pattern: &ErrorPattern) -> Result<()> {
        let graph = self.graph.lock().map_err(Self::map_err)?;

        let entity = Entity {
            id: pattern.id.clone(),
            name: pattern.id.clone(),
            entity_type: "error_pattern".into(),
            summary: Some(pattern.description.clone()),
            created_at: pattern.first_seen,
            metadata: Some(serde_json::json!({
                "category": pattern.category,
                "occurrences": pattern.occurrences,
                "first_seen": pattern.first_seen,
                "last_seen": pattern.last_seen,
                "reflection_ids": pattern.reflection_ids,
                "trend": pattern.trend,
            })),
        };

        graph.add_entity(entity).map_err(Self::map_err)?;
        Ok(())
    }

    async fn get_pattern(&self, id: &str) -> Result<Option<ErrorPattern>> {
        let graph = self.graph.lock().map_err(Self::map_err)?;
        let entity = graph.get_entity(id).map_err(Self::map_err)?;

        match entity {
            Some(e) if e.entity_type == "error_pattern" => {
                let meta = e.metadata.unwrap_or(serde_json::json!({}));
                let pattern = ErrorPattern {
                    id: e.id,
                    category: meta
                        .get("category")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    description: e.summary.unwrap_or_default(),
                    occurrences: meta
                        .get("occurrences")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32,
                    first_seen: parse_datetime_or_default(meta.get("first_seen"), e.created_at),
                    last_seen: parse_datetime_or_default(meta.get("last_seen"), e.created_at),
                    reflection_ids: meta
                        .get("reflection_ids")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default(),
                    trend: meta
                        .get("trend")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or(Trend::Stable),
                };
                Ok(Some(pattern))
            }
            _ => Ok(None),
        }
    }

    async fn list_patterns(
        &self,
        min_occurrences: u32,
        limit: usize,
    ) -> Result<Vec<ErrorPattern>> {
        let graph = self.graph.lock().map_err(Self::map_err)?;

        // Fetch a generous pool of error_pattern entities
        let entities = graph
            .list_entities(Some("error_pattern"), limit * 3)
            .map_err(Self::map_err)?;

        let mut patterns = Vec::new();
        for e in entities {
            let meta = e.metadata.unwrap_or(serde_json::json!({}));
            let occurrences = meta
                .get("occurrences")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;

            if occurrences < min_occurrences {
                continue;
            }

            let pattern = ErrorPattern {
                id: e.id,
                category: meta
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                description: e.summary.unwrap_or_default(),
                occurrences,
                first_seen: parse_datetime_or_default(meta.get("first_seen"), e.created_at),
                last_seen: parse_datetime_or_default(meta.get("last_seen"), e.created_at),
                reflection_ids: meta
                    .get("reflection_ids")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
                trend: meta
                    .get("trend")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or(Trend::Stable),
            };

            patterns.push(pattern);
            if patterns.len() >= limit {
                break;
            }
        }

        Ok(patterns)
    }

    async fn get_stats(&self) -> Result<ReflectionStats> {
        let graph = self.graph.lock().map_err(Self::map_err)?;
        let graph_stats = graph.stats().map_err(Self::map_err)?;

        // Fetch all reflect episodes to compute outcome counts and tag stats
        let all_episodes = graph.search("", graph_stats.episode_count.max(1000)).map_err(Self::map_err)?;

        let mut success: u64 = 0;
        let mut failure: u64 = 0;
        let mut partial: u64 = 0;
        let mut total: u64 = 0;
        let mut confidence_sum: f64 = 0.0;
        let mut tag_counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        let mut this_week: u64 = 0;
        let week_ago = Utc::now() - chrono::Duration::days(7);

        for (episode, _) in &all_episodes {
            let reflection = match Self::episode_to_reflection(episode) {
                Ok(r) => r,
                Err(_) => continue,
            };

            total += 1;
            confidence_sum += reflection.confidence as f64;

            match reflection.outcome {
                Outcome::Success => success += 1,
                Outcome::Failure => failure += 1,
                Outcome::Partial => partial += 1,
            }

            for tag in &reflection.tags {
                *tag_counts.entry(tag.clone()).or_insert(0) += 1;
            }

            if reflection.created_at > week_ago {
                this_week += 1;
            }
        }

        let avg_confidence = if total > 0 {
            confidence_sum / total as f64
        } else {
            0.0
        };

        let mut top_tags: Vec<TagCount> = tag_counts
            .into_iter()
            .map(|(tag, count)| TagCount { tag, count })
            .collect();
        top_tags.sort_by(|a, b| b.count.cmp(&a.count));
        top_tags.truncate(10);

        // Fetch top patterns
        let pattern_entities = graph
            .list_entities(Some("error_pattern"), 5)
            .map_err(Self::map_err)?;

        let top_patterns: Vec<ErrorPattern> = pattern_entities
            .into_iter()
            .filter_map(|e| {
                let meta = e.metadata.unwrap_or(serde_json::json!({}));
                Some(ErrorPattern {
                    id: e.id,
                    category: meta
                        .get("category")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    description: e.summary.unwrap_or_default(),
                    occurrences: meta.get("occurrences").and_then(|v| v.as_u64()).unwrap_or(0)
                        as u32,
                    first_seen: parse_datetime_or_default(meta.get("first_seen"), e.created_at),
                    last_seen: parse_datetime_or_default(meta.get("last_seen"), e.created_at),
                    reflection_ids: meta
                        .get("reflection_ids")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default(),
                    trend: meta
                        .get("trend")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or(Trend::Stable),
                })
            })
            .collect();

        Ok(ReflectionStats {
            total_reflections: total,
            by_outcome: OutcomeCounts {
                success,
                failure,
                partial,
            },
            top_patterns,
            top_tags,
            avg_confidence,
            reflections_this_week: this_week,
        })
    }
}

fn parse_datetime_or_default(
    value: Option<&serde_json::Value>,
    default: chrono::DateTime<Utc>,
) -> chrono::DateTime<Utc> {
    value
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}
