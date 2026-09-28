-- Per-file lookups of every Imports refresh: the import a source arrived with and
-- the paid requests of a source. Without these each refresh scans both tables once
-- per file while holding the vault lock.
CREATE INDEX IF NOT EXISTS envelope_source_source ON envelope_source(source_id);
CREATE INDEX IF NOT EXISTS import_request_source ON import_request(source_id);
PRAGMA user_version = 17;
