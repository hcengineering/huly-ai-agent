ALTER TABLE mem_entity RENAME COLUMN entity_type TO category;

ALTER TABLE mem_entity
ADD COLUMN entity_type INTEGER NOT NULL DEFAULT 0;

CREATE VIRTUAL TABLE vec_mem_entity1 USING vec0(
	embedding float[256],
	entity_type INTEGER
);

INSERT INTO
    vec_mem_entity1 (rowid, embedding, entity_type)
SELECT v.rowid, v.embedding, m.entity_type
FROM vec_mem_entity as v, mem_entity as m
WHERE
    v.rowid = m.rowid;

DROP TABLE vec_mem_entity;