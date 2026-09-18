-- Expand the collection without moving credential contents into assertions or FTS.
CREATE TABLE collection_item_new (
  local_id INTEGER PRIMARY KEY,
  stable_id TEXT NOT NULL UNIQUE,
  title TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('note','document','credential')),
  source_id TEXT NOT NULL REFERENCES source(id),
  property_key TEXT REFERENCES property_definition(key),
  current_assertion_id TEXT REFERENCES assertion(id),
  extension TEXT NOT NULL DEFAULT '',
  pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0,1)),
  revision INTEGER NOT NULL DEFAULT 1,
  deleted_at TEXT
) STRICT;
INSERT INTO collection_item_new SELECT * FROM collection_item;
DROP TABLE collection_item;
ALTER TABLE collection_item_new RENAME TO collection_item;

-- Preserve the complete original, including attachments and unknown future fields.
-- These BLOBs live inside SQLCipher, and never enter the document/agent pipeline.
CREATE TABLE credential_import (
  id TEXT PRIMARY KEY,
  fingerprint TEXT NOT NULL UNIQUE,
  archive BLOB NOT NULL,
  imported_at TEXT NOT NULL
) STRICT;
CREATE TABLE credential_record (
  item_id INTEGER PRIMARY KEY REFERENCES collection_item(local_id),
  import_id TEXT NOT NULL REFERENCES credential_import(id),
  account_uuid TEXT NOT NULL,
  vault_uuid TEXT NOT NULL,
  item_uuid TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  vault_name TEXT NOT NULL,
  category TEXT NOT NULL,
  archived INTEGER NOT NULL CHECK(archived IN (0,1)),
  raw_json TEXT NOT NULL CHECK(json_valid(raw_json)),
  attachment_paths_json TEXT NOT NULL CHECK(json_valid(attachment_paths_json)),
  UNIQUE(account_uuid,vault_uuid,item_uuid,fingerprint)
) STRICT;
PRAGMA user_version = 6;
