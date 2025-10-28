CREATE TABLE IF NOT EXISTS scheduler (
    id TEXT NOT NULL,
    next_run_at DATETIME NOT NULL
);