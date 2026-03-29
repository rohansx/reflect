CREATE TABLE IF NOT EXISTS reflections (
    id                  TEXT PRIMARY KEY,
    task_description    TEXT NOT NULL,
    draft               TEXT NOT NULL,
    critique            TEXT NOT NULL,
    lesson              TEXT NOT NULL,
    outcome             TEXT NOT NULL CHECK(outcome IN ('success','failure','partial')),
    pattern_id          TEXT,
    tags                TEXT NOT NULL DEFAULT '[]',
    confidence          REAL NOT NULL DEFAULT 0.5,
    validation_count    INTEGER NOT NULL DEFAULT 0,
    contradiction_count INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT NOT NULL,
    last_recalled       TEXT
);

CREATE TABLE IF NOT EXISTS eval_signals (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    reflection_id   TEXT NOT NULL REFERENCES reflections(id) ON DELETE CASCADE,
    evaluator       TEXT NOT NULL,
    passed          INTEGER NOT NULL,
    summary         TEXT NOT NULL,
    errors_json     TEXT NOT NULL DEFAULT '[]'
);

CREATE TABLE IF NOT EXISTS error_patterns (
    id          TEXT PRIMARY KEY,
    category    TEXT NOT NULL,
    description TEXT NOT NULL,
    occurrences INTEGER NOT NULL DEFAULT 1,
    first_seen  TEXT NOT NULL,
    last_seen   TEXT NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS reflections_fts USING fts5(
    task_description, critique, lesson, tags,
    content='reflections', content_rowid='rowid'
);

CREATE TRIGGER IF NOT EXISTS reflections_ai AFTER INSERT ON reflections BEGIN
    INSERT INTO reflections_fts(rowid, task_description, critique, lesson, tags)
    VALUES (new.rowid, new.task_description, new.critique, new.lesson, new.tags);
END;

CREATE TRIGGER IF NOT EXISTS reflections_ad AFTER DELETE ON reflections BEGIN
    INSERT INTO reflections_fts(reflections_fts, rowid, task_description, critique, lesson, tags)
    VALUES ('delete', old.rowid, old.task_description, old.critique, old.lesson, old.tags);
END;

CREATE TRIGGER IF NOT EXISTS reflections_au AFTER UPDATE ON reflections BEGIN
    INSERT INTO reflections_fts(reflections_fts, rowid, task_description, critique, lesson, tags)
    VALUES ('delete', old.rowid, old.task_description, old.critique, old.lesson, old.tags);
    INSERT INTO reflections_fts(rowid, task_description, critique, lesson, tags)
    VALUES (new.rowid, new.task_description, new.critique, new.lesson, new.tags);
END;
