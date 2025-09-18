ALTER TABLE tasks ADD COLUMN state INTEGER NOT NULL DEFAULT 0;

ALTER TABLE tasks DROP COLUMN is_done;

-- set all current task state to completed
UPDATE tasks SET state = 2;

ALTER TABLE tasks RENAME COLUMN channel_id TO card_id;

ALTER TABLE tasks RENAME COLUMN channel_title TO card_title;

CREATE TABLE IF NOT EXISTS scheduled_tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content TEXT NOT NULL,
    schedule TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS notes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS assistant_messages (
    card_id TEXT NOT NULL PRIMARY KEY,
    messages TEXT NOT NULL DEFAULT "[]"
);