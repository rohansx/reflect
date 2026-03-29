mod config;
mod tools;

use config::{load_config, resolve_db_path};
use reflect_core::pattern::{PatternEngine, PatternRule};
use reflect_core::storage::Storage;
use reflect_store::SqliteStorage;
use rmcp::transport::io::stdio;
use rmcp::ServiceExt;
use tools::ReflectServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let config = load_config();
    let db_path = resolve_db_path(&config);

    if let Some(parent) = std::path::Path::new(&db_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let storage: Box<dyn Storage> = match config.storage.backend.as_str() {
        #[cfg(feature = "ctxgraph")]
        "ctxgraph" => {
            tracing::info!("using ctxgraph storage backend");
            Box::new(reflect_store::CtxgraphStorage::open(&db_path)?)
        }
        _ => {
            tracing::info!("using sqlite storage backend");
            Box::new(SqliteStorage::open(&db_path)?)
        }
    };

    let mut engine = PatternEngine::default();
    for rule in &config.patterns {
        if let Ok(re) = regex::Regex::new(&rule.regex) {
            engine.add_rule(PatternRule {
                evaluator: rule.evaluator.clone(),
                regex: re,
                id: rule.id.clone(),
                category: rule.category.clone(),
                description: format!("Custom: {}", rule.id),
            });
        }
    }

    let server = ReflectServer::new(storage, engine, config);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
