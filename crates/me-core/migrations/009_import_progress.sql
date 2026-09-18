CREATE TABLE import_progress (
 source_id TEXT PRIMARY KEY REFERENCES source(id),
 run_id TEXT NOT NULL,
 stage TEXT NOT NULL,
 current INTEGER NOT NULL DEFAULT 0,
 total INTEGER NOT NULL DEFAULT 0
) STRICT;
PRAGMA user_version = 9;
