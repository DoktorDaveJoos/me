-- Deep-first reading. Document facts are derived from encrypted originals and are
-- replaced on every re-read of their source; profile values stay assertions.
CREATE TABLE document_fact (
 id TEXT PRIMARY KEY,
 source_id TEXT NOT NULL REFERENCES source(id),
 run_id TEXT NOT NULL,
 ordinal INTEGER NOT NULL,
 label TEXT NOT NULL,
 value TEXT NOT NULL,
 context_quote TEXT NOT NULL,
 locator_json TEXT NOT NULL CHECK(json_valid(locator_json)),
 owner TEXT NOT NULL CHECK(owner IN ('self','household','party','organization','unclear','unknown')),
 owner_entity_id TEXT REFERENCES entity(id),
 owner_name TEXT,
 period_kind TEXT CHECK(period_kind IS NULL OR period_kind IN ('document','cumulative','other','none')),
 slot TEXT,
 assertion_id TEXT REFERENCES assertion(id),
 state TEXT NOT NULL CHECK(state IN ('verified','uncertain','unverified','uninterpreted')),
 confidence REAL CHECK(confidence IS NULL OR confidence BETWEEN 0 AND 1),
 recorded_at TEXT NOT NULL
) STRICT;
CREATE INDEX document_fact_source ON document_fact(source_id, state, ordinal);
-- Counts and rejection codes only; the Imports line and the self-check read this.
CREATE TABLE read_summary (
 source_id TEXT PRIMARY KEY REFERENCES source(id),
 run_id TEXT NOT NULL,
 policy TEXT NOT NULL,
 values_read INTEGER NOT NULL,
 in_profile INTEGER NOT NULL,
 checks INTEGER NOT NULL,
 uninterpreted INTEGER NOT NULL,
 rejected_json TEXT NOT NULL CHECK(json_valid(rejected_json)),
 recorded_at TEXT NOT NULL
) STRICT;
-- Documents finished by the limited tiered reader are read again behind new imports.
UPDATE document_evaluation
 SET state='queued', priority=0, warning_message=NULL, error_message=NULL
 WHERE state='done'
 AND source_id IN (SELECT source_id FROM document_profile)
 AND NOT EXISTS(SELECT 1 FROM job j WHERE j.source_id=document_evaluation.source_id AND j.kind='extract_facts' AND j.state IN ('done','needs_review'));
PRAGMA user_version = 16;
