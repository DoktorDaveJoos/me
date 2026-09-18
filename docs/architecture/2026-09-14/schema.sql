-- Me. logical schema proposal, 2026-09-14. NOT a production vault migration.
-- One database per vault. In production open with SQLCipher and provide key
-- before schema access. This file contains no encryption or authorization code.
-- Plain SQLite is used only for synthetic schema verification.
PRAGMA foreign_keys = ON;
PRAGMA temp_store = MEMORY;

CREATE TABLE entity (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  label TEXT NOT NULL,
  created_at TEXT NOT NULL,
  deleted_at TEXT
) STRICT;
CREATE TABLE entity_alias (
  entity_id TEXT NOT NULL REFERENCES entity(id) ON DELETE CASCADE,
  alias TEXT NOT NULL,
  normalized_alias TEXT NOT NULL,
  PRIMARY KEY(entity_id, normalized_alias)
) STRICT;
CREATE INDEX entity_alias_lookup ON entity_alias(normalized_alias);

CREATE TABLE property_definition (
  key TEXT PRIMARY KEY,
  label TEXT NOT NULL,
  value_type TEXT NOT NULL CHECK(value_type IN
    ('text','identifier','date','integer','decimal','money','quantity','boolean','entity','json')),
  cardinality TEXT NOT NULL CHECK(cardinality IN ('one','many')),
  change_policy TEXT NOT NULL CHECK(change_policy IN
    ('confirm','append_period','auto_if_unambiguous','explicit_user_intent')),
  rules_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(rules_json)),
  schema_version INTEGER NOT NULL CHECK(schema_version > 0),
  UNIQUE(key, value_type)
) STRICT;

CREATE TABLE inbox_item (
  id TEXT PRIMARY KEY,
  channel TEXT NOT NULL CHECK(channel IN ('drop','manual','watch_folder','forwarded_email')),
  connector_id TEXT NOT NULL,
  delivery_key TEXT NOT NULL,
  received_at TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN
    ('received','staged','processing','waiting_provider','needs_review','done','failed','discarded')),
  UNIQUE(connector_id, delivery_key)
) STRICT;

CREATE TABLE source (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN ('file','email','note','form','scan_part')),
  title TEXT NOT NULL,
  document_date TEXT,
  received_at TEXT NOT NULL,
  content_fingerprint TEXT NOT NULL,
  sensitivity TEXT NOT NULL CHECK(sensitivity IN ('personal','restricted','credential')),
  retention TEXT NOT NULL CHECK(retention IN ('pending','keep','ephemeral','purge_requested','purged')),
  purge_after TEXT,
  object_id TEXT,
  extraction_revision INTEGER NOT NULL DEFAULT 0,
  CHECK(retention <> 'purged' OR object_id IS NULL)
) STRICT;
-- Equal bytes do not imply equal delivery context or a repeated user statement.
CREATE INDEX source_content_lookup ON source(content_fingerprint);
-- object_id resolves to an encrypted object; never an arbitrary agent-supplied path.
CREATE TABLE inbox_source (
  inbox_id TEXT NOT NULL REFERENCES inbox_item(id) ON DELETE CASCADE,
  source_id TEXT NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  PRIMARY KEY(inbox_id, source_id)
) STRICT;
CREATE TABLE source_link (
  parent_id TEXT NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  child_id TEXT NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK(role IN ('attachment','split_from','revision_of')),
  locator_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(locator_json)),
  PRIMARY KEY(parent_id, child_id, role),
  CHECK(parent_id <> child_id)
) STRICT;
CREATE TABLE source_entity (
  source_id TEXT NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  entity_id TEXT NOT NULL REFERENCES entity(id) ON DELETE CASCADE,
  role TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('proposed','accepted','rejected')),
  PRIMARY KEY(source_id, entity_id, role)
) STRICT;

CREATE TABLE source_segment (
  rowid INTEGER PRIMARY KEY,
  id TEXT NOT NULL UNIQUE,
  source_id TEXT NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  extraction_revision INTEGER NOT NULL,
  ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
  locator_json TEXT NOT NULL CHECK(json_valid(locator_json)),
  text TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  UNIQUE(source_id, extraction_revision, ordinal),
  UNIQUE(id, source_id)
) STRICT;
CREATE TRIGGER segment_source_guard BEFORE INSERT ON source_segment
WHEN EXISTS (SELECT 1 FROM source WHERE id = NEW.source_id AND
  (sensitivity = 'credential' OR retention IN ('purge_requested','purged')))
BEGIN SELECT RAISE(ABORT, 'source is not indexable'); END;
CREATE TRIGGER segment_immutable BEFORE UPDATE ON source_segment
BEGIN SELECT RAISE(ABORT, 'segments are immutable; rebuild with new ids'); END;
-- Derived FTS index lives in the same encrypted database in production.
CREATE VIRTUAL TABLE segment_fts USING fts5(
  text, content='source_segment', content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER segment_ai AFTER INSERT ON source_segment BEGIN
  INSERT INTO segment_fts(rowid,text) VALUES(new.rowid,new.text);
END;
CREATE TRIGGER segment_ad AFTER DELETE ON source_segment BEGIN
  INSERT INTO segment_fts(segment_fts,rowid,text) VALUES('delete',old.rowid,old.text);
END;

CREATE TABLE extraction_run (
  id TEXT PRIMARY KEY,
  source_id TEXT REFERENCES source(id) ON DELETE SET NULL,
  provider TEXT NOT NULL,
  model TEXT NOT NULL,
  pipeline_version TEXT NOT NULL,
  schema_version INTEGER NOT NULL,
  started_at TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('running','succeeded','failed'))
) STRICT;

CREATE TABLE assertion (
  id TEXT PRIMARY KEY,
  subject_id TEXT NOT NULL REFERENCES entity(id),
  property_key TEXT NOT NULL,
  value_type TEXT NOT NULL,
  value_json TEXT NOT NULL CHECK(json_valid(value_json)),
  object_entity_id TEXT REFERENCES entity(id),
  -- Computed by trusted normalization code, never accepted from model as-is.
  canonical_value TEXT NOT NULL,
  semantic_key TEXT NOT NULL UNIQUE,
  time_kind TEXT NOT NULL CHECK(time_kind IN ('timeless','interval','point','unknown')),
  valid_from TEXT,
  valid_to TEXT,
  recorded_at TEXT NOT NULL,
  origin TEXT NOT NULL CHECK(origin IN ('user','extraction','calculation')),
  extraction_run_id TEXT REFERENCES extraction_run(id),
  supersedes_id TEXT REFERENCES assertion(id),
  FOREIGN KEY(property_key,value_type) REFERENCES property_definition(key,value_type),
  CHECK((value_type='entity' AND object_entity_id IS NOT NULL AND
    json_type(value_json)='text' AND json_extract(value_json,'$')=object_entity_id)
    OR (value_type<>'entity' AND object_entity_id IS NULL)),
  CHECK(CASE value_type
    WHEN 'text' THEN json_type(value_json)='text'
    WHEN 'identifier' THEN json_type(value_json)='text'
    WHEN 'date' THEN json_type(value_json)='text'
    WHEN 'integer' THEN json_type(value_json)='integer'
    WHEN 'decimal' THEN json_type(value_json)='text'
    WHEN 'money' THEN json_type(value_json)='object'
    WHEN 'quantity' THEN json_type(value_json)='object'
    WHEN 'boolean' THEN json_type(value_json) IN ('true','false')
    WHEN 'entity' THEN json_type(value_json)='text'
    ELSE 1 END),
  CHECK((time_kind IN ('timeless','unknown') AND valid_from IS NULL AND valid_to IS NULL)
    OR (time_kind='point' AND valid_from IS NOT NULL AND valid_to IS NULL)
    OR (time_kind='interval' AND valid_from IS NOT NULL AND
      (valid_to IS NULL OR valid_to > valid_from)))
) STRICT;
CREATE INDEX assertion_exact ON assertion(subject_id,property_key,canonical_value);
CREATE INDEX assertion_reverse_relation ON assertion(object_entity_id,property_key);
CREATE TRIGGER assertion_immutable BEFORE UPDATE ON assertion
BEGIN SELECT RAISE(ABORT, 'assertions are immutable; append a revision'); END;

CREATE TABLE assertion_evidence (
  assertion_id TEXT NOT NULL REFERENCES assertion(id) ON DELETE CASCADE,
  source_id TEXT NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  evidence_key TEXT NOT NULL,
  -- Source-relative immutable locator, not an FK to a rebuildable search chunk.
  locator_json TEXT NOT NULL CHECK(json_valid(locator_json)),
  PRIMARY KEY(assertion_id, source_id, evidence_key)
) STRICT;

CREATE TABLE decision (
  local_seq INTEGER PRIMARY KEY,
  id TEXT NOT NULL UNIQUE,
  assertion_id TEXT NOT NULL REFERENCES assertion(id) ON DELETE CASCADE,
  action TEXT NOT NULL CHECK(action IN ('accept','reject','retract')),
  actor TEXT NOT NULL CHECK(actor IN ('user','policy')),
  policy_version TEXT,
  reason_code TEXT NOT NULL,
  recorded_at TEXT NOT NULL,
  CHECK(actor <> 'policy' OR policy_version IS NOT NULL)
) STRICT;
CREATE INDEX decision_by_assertion ON decision(assertion_id,local_seq DESC);
CREATE TRIGGER decision_immutable BEFORE UPDATE ON decision
BEGIN SELECT RAISE(ABORT, 'decisions are immutable'); END;
-- Local single-device decision order. NOT a multi-device conflict resolution rule.
CREATE VIEW assertion_state AS
SELECT a.*, coalesce(d.action,'proposed') AS state
FROM assertion a LEFT JOIN decision d ON d.local_seq =
 (SELECT max(d2.local_seq) FROM decision d2 WHERE d2.assertion_id=a.id);
CREATE VIEW accepted_edges AS
SELECT id AS assertion_id, subject_id, property_key, object_entity_id,
       time_kind,valid_from,valid_to
FROM assertion_state WHERE state='accept' AND value_type='entity';

CREATE TABLE review_case (
  id TEXT PRIMARY KEY,
  subject_id TEXT REFERENCES entity(id),
  reason_code TEXT NOT NULL,
  severity TEXT NOT NULL CHECK(severity IN ('info','review','critical')),
  status TEXT NOT NULL CHECK(status IN ('open','resolved','dismissed')),
  created_at TEXT NOT NULL,
  resolved_at TEXT
) STRICT;
CREATE TABLE review_member (
  review_id TEXT NOT NULL REFERENCES review_case(id) ON DELETE CASCADE,
  assertion_id TEXT NOT NULL REFERENCES assertion(id) ON DELETE CASCADE,
  PRIMARY KEY(review_id,assertion_id)
) STRICT;
CREATE TABLE task (
  id TEXT PRIMARY KEY,
  inbox_id TEXT REFERENCES inbox_item(id) ON DELETE SET NULL,
  source_id TEXT REFERENCES source(id) ON DELETE SET NULL,
  kind TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('queued','working','needs_input','ready','approved','done','failed')),
  draft_object_id TEXT,
  due_at TEXT,
  revision INTEGER NOT NULL DEFAULT 1,
  approved_revision INTEGER,
  CHECK(approved_revision IS NULL OR approved_revision <= revision)
) STRICT;
CREATE TABLE job (
  id TEXT PRIMARY KEY,
  source_id TEXT REFERENCES source(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  dedup_key TEXT NOT NULL UNIQUE,
  state TEXT NOT NULL CHECK(state IN ('queued','running','waiting_provider','done','failed')),
  attempts INTEGER NOT NULL DEFAULT 0,
  lease_until TEXT,
  next_attempt_at TEXT,
  last_error_code TEXT
) STRICT;

CREATE TABLE embedding_model (
  id TEXT PRIMARY KEY,
  model_name TEXT NOT NULL,
  model_revision TEXT NOT NULL,
  dimension INTEGER NOT NULL CHECK(dimension > 0),
  normalization TEXT NOT NULL,
  preprocessing_version TEXT NOT NULL,
  distance TEXT NOT NULL CHECK(distance IN ('cosine','dot','l2')),
  UNIQUE(id,dimension)
) STRICT;
-- Baseline representation: normal SQL table, no vector extension required.
-- Exact similarity is computed over authorized candidates in a Rust worker.
CREATE TABLE chunk_embedding (
  segment_id TEXT NOT NULL REFERENCES source_segment(id) ON DELETE CASCADE,
  model_id TEXT NOT NULL,
  dimension INTEGER NOT NULL,
  vector_f32 BLOB NOT NULL,
  PRIMARY KEY(segment_id,model_id),
  FOREIGN KEY(model_id,dimension) REFERENCES embedding_model(id,dimension),
  CHECK(length(vector_f32)=4*dimension)
) STRICT;

CREATE TRIGGER source_purge_guard BEFORE UPDATE OF retention ON source
WHEN NEW.retention='purged' AND (
  EXISTS(SELECT 1 FROM source_segment WHERE source_id=OLD.id) OR
  EXISTS(SELECT 1 FROM assertion_evidence WHERE source_id=OLD.id)
)
BEGIN SELECT RAISE(ABORT, 'purge derived data and resolve evidence first'); END;

-- Deliberately not implemented here: key envelopes/object encryption, grant
-- enforcement, complete value validation, temporal resolver, conflict detector,
-- embedding inference, vector scoring, IPC, external actions, or sync protocol.
