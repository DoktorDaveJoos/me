-- Every stage and paid request survives process restarts inside SQLCipher.
ALTER TABLE import_progress ADD COLUMN error_code TEXT;
ALTER TABLE import_progress ADD COLUMN error_provider TEXT;
CREATE TABLE import_step_progress (
 source_id TEXT NOT NULL REFERENCES source(id),
 stage TEXT NOT NULL,
 current INTEGER NOT NULL DEFAULT 0,
 total INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(source_id,stage)
) STRICT;
CREATE TABLE import_step_cache (
 source_id TEXT NOT NULL REFERENCES source(id),
 pipeline TEXT NOT NULL,
 batch_key TEXT NOT NULL,
 step TEXT NOT NULL,
 output_json TEXT NOT NULL,
 PRIMARY KEY(source_id,pipeline,batch_key,step)
) STRICT;
CREATE TABLE import_budget (
 source_id TEXT PRIMARY KEY REFERENCES source(id),
 openai_limit INTEGER NOT NULL DEFAULT 12,
 typesafe_limit INTEGER NOT NULL DEFAULT 24,
 token_limit INTEGER NOT NULL DEFAULT 180000
) STRICT;
CREATE TABLE import_request (
 id TEXT PRIMARY KEY,
 source_id TEXT NOT NULL REFERENCES source(id),
 provider TEXT NOT NULL CHECK(provider IN ('openai','typesafe')),
 input_tokens INTEGER,
 output_tokens INTEGER,
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;
CREATE TABLE import_control (
 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
 pause_reason TEXT
) STRICT;
INSERT INTO import_control VALUES(1,NULL);
PRAGMA user_version = 10;
