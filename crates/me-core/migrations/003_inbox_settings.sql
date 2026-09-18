CREATE TABLE app_settings (
 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
 automatic_evaluation INTEGER NOT NULL DEFAULT 1 CHECK(automatic_evaluation IN (0,1))
) STRICT;
INSERT INTO app_settings VALUES(1,1);

-- Separate orchestration state: indexing and fact extraction keep their own jobs.
CREATE TABLE document_evaluation (
 source_id TEXT PRIMARY KEY REFERENCES source(id),
 state TEXT NOT NULL CHECK(state IN ('queued','running','manual','done','failed')),
 error_message TEXT
) STRICT;
INSERT INTO document_evaluation(source_id,state)
 SELECT i.source_id, CASE
 WHEN EXISTS(SELECT 1 FROM job j WHERE j.source_id=i.source_id AND j.kind='extract_facts' AND j.state IN ('done','needs_review')) THEN 'done'
 WHEN s.sensitivity='credential' THEN 'manual' ELSE 'queued' END
 FROM collection_item i JOIN source s ON s.id=i.source_id
 WHERE i.kind='document' AND i.deleted_at IS NULL;
PRAGMA user_version = 3;
