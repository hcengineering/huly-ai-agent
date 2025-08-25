DROP TABLE IF EXISTS vec_mem_entity;

DROP TABLE IF EXISTS mem_observation;

DROP TABLE IF EXISTS mem_relation;

DROP TABLE IF EXISTS mem_entity;

CREATE TABLE IF NOT EXISTS mem_entity (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    importance REAL NOT NULL DEFAULT 0.0,
    access_count INTEGER NOT NULL DEFAULT 0,
    observations TEXT NOT NULL DEFAULT '[]',
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE VIRTUAL TABLE vec_mem_entity USING vec0(embedding float[256]);

CREATE TABLE IF NOT EXISTS mem_relation (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    from_id INTEGER NOT NULL,
    to_id INTEGER NOT NULL,
    FOREIGN KEY (from_id) REFERENCES mem_entity (id),
    FOREIGN KEY (to_id) REFERENCES mem_entity (id)
);
