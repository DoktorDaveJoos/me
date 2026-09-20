-- A local presentation of the existing knowledge graph, inside SQLCipher.
-- Node keys refer to canonical items/assertions; no copied personal values.
CREATE TABLE knowledge_position (
    node_id TEXT PRIMARY KEY,
    cluster_id TEXT NOT NULL,
    q INTEGER NOT NULL CHECK(q BETWEEN -1000000 AND 1000000),
    r INTEGER NOT NULL CHECK(r BETWEEN -1000000 AND 1000000),
    layout_version INTEGER NOT NULL DEFAULT 1 CHECK(layout_version = 1),
    UNIQUE(q, r)
) STRICT;
CREATE TABLE knowledge_view (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    center_q REAL NOT NULL,
    center_r REAL NOT NULL,
    zoom REAL NOT NULL CHECK(zoom BETWEEN 0.4 AND 2.0),
    selected_node TEXT
) STRICT;
-- Empty until the user changes the viewport. Development wipe clears both tables.
PRAGMA user_version = 11;
