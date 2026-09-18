CREATE TABLE data_recent (
    assertion_id TEXT PRIMARY KEY REFERENCES assertion(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL
) STRICT;
PRAGMA user_version=8;
