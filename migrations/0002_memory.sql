CREATE TABLE IF NOT EXISTS mem_entity (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    type TEXT NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE VIRTUAL TABLE vec_mem_entity USING vec0(embedding float[256]);

CREATE TABLE IF NOT EXISTS mem_observation (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_id INTEGER NOT NULL,
    observation TEXT NOT NULL,
    FOREIGN KEY (entity_id) REFERENCES mem_entity (id)
);

CREATE TABLE IF NOT EXISTS mem_relation (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    from_id INTEGER NOT NULL,
    to_id INTEGER NOT NULL,
    relation_type TEXT NOT NULL,
    FOREIGN KEY (from_id) REFERENCES mem_entity (id),
    FOREIGN KEY (to_id) REFERENCES mem_entity (id)
);
