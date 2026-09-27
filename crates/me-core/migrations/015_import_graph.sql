-- Tiered import pipeline and personal graph derivation. All content stays inside SQLCipher.
-- A batch groups one dump or one delivery run and carries its own OpenAI allowance.
CREATE TABLE import_batch (
 id TEXT PRIMARY KEY,
 label TEXT NOT NULL,
 created_at TEXT NOT NULL,
 openai_allowance INTEGER NOT NULL CHECK(openai_allowance >= 0),
 openai_calls INTEGER NOT NULL DEFAULT 0,
 typesafe_requests INTEGER NOT NULL DEFAULT 0,
 typesafe_input_tokens INTEGER NOT NULL DEFAULT 0
) STRICT;
-- One intake event. Sources produce envelopes; they know nothing about stages.
CREATE TABLE intake_envelope (
 id TEXT PRIMARY KEY,
 source_kind TEXT NOT NULL CHECK(source_kind IN ('drop','folder','scanner','mail')),
 origin_json TEXT NOT NULL CHECK(json_valid(origin_json)),
 batch_id TEXT REFERENCES import_batch(id),
 received_at TEXT NOT NULL
) STRICT;
CREATE TABLE envelope_source (
 envelope_id TEXT NOT NULL REFERENCES intake_envelope(id),
 source_id TEXT NOT NULL REFERENCES source(id),
 ordinal INTEGER NOT NULL,
 -- The bytes were already in the vault; the envelope links the existing source.
 duplicate INTEGER NOT NULL DEFAULT 0 CHECK(duplicate IN (0,1)),
 PRIMARY KEY(envelope_id, source_id)
) STRICT;
-- Device-local stage cache. Identical content never runs a stage version twice.
CREATE TABLE stage_run (
 content_hash TEXT NOT NULL,
 stage TEXT NOT NULL,
 version TEXT NOT NULL,
 output_json TEXT NOT NULL CHECK(json_valid(output_json)),
 model TEXT,
 input_tokens INTEGER,
 recorded_at TEXT NOT NULL,
 PRIMARY KEY(content_hash, stage, version)
) STRICT;
-- Document-level classification: family, specific type and subject anchor.
CREATE TABLE document_profile (
 source_id TEXT PRIMARY KEY REFERENCES source(id),
 family TEXT NOT NULL,
 doc_type TEXT,
 family_confidence REAL NOT NULL CHECK(family_confidence BETWEEN 0 AND 1),
 type_confidence REAL CHECK(type_confidence BETWEEN 0 AND 1),
 tier TEXT NOT NULL CHECK(tier IN ('eager','lazy')),
 subject_entity_id TEXT REFERENCES entity(id),
 subject_name TEXT,
 graph_state TEXT NOT NULL CHECK(graph_state IN ('classified','resolved','skipped','failed')),
 model TEXT,
 classified_at TEXT NOT NULL
) STRICT;
CREATE INDEX document_profile_family ON document_profile(family, doc_type);
-- Confidence and review routing of automatically accepted assertions. Review state is
-- derived: a user decision means reviewed; a policy decision with check_reason means
-- check suggested; a policy decision alone means unreviewed.
CREATE TABLE assertion_review (
 assertion_id TEXT PRIMARY KEY REFERENCES assertion(id),
 confidence REAL NOT NULL CHECK(confidence BETWEEN 0 AND 1),
 confidence_source TEXT NOT NULL CHECK(confidence_source IN ('checksum','typesafe','openai_grounded','user')),
 -- The value itself passed a checksum (MRZ, IBAN, tax ID). Meaning is judged separately.
 value_checked INTEGER NOT NULL DEFAULT 0 CHECK(value_checked IN (0,1)),
 check_reason TEXT CHECK(check_reason IN ('low_confidence','conflict','verification')),
 model TEXT,
 recorded_at TEXT NOT NULL
) STRICT;
-- Outcomes of reviewed automatic assertions tune the check threshold.
CREATE TABLE review_outcome (
 assertion_id TEXT PRIMARY KEY REFERENCES assertion(id),
 confidence REAL NOT NULL,
 corrected INTEGER NOT NULL CHECK(corrected IN (0,1)),
 recorded_at TEXT NOT NULL
) STRICT;
-- Declared identity anchors beyond the self profile.
CREATE TABLE household_member (
 entity_id TEXT PRIMARY KEY REFERENCES entity(id),
 role TEXT NOT NULL CHECK(role IN ('partner','child','parent','other')),
 added_at TEXT NOT NULL
) STRICT;
CREATE TABLE identity_setup (
 singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
 completed INTEGER NOT NULL DEFAULT 0 CHECK(completed IN (0,1))
) STRICT;
INSERT INTO identity_setup(singleton, completed) VALUES(1, 0);
-- Non-anchor people named as a document's subject. Three sources propose a member.
CREATE TABLE person_mention (
 normalized_name TEXT NOT NULL,
 name TEXT NOT NULL,
 source_id TEXT NOT NULL REFERENCES source(id),
 PRIMARY KEY(normalized_name, source_id)
) STRICT;
CREATE TABLE household_proposal (
 normalized_name TEXT PRIMARY KEY,
 name TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('open','accepted','dismissed'))
) STRICT;
-- Name-only entity matches are proposals, never automatic merges.
CREATE TABLE merge_proposal (
 id TEXT PRIMARY KEY,
 entity_a TEXT NOT NULL REFERENCES entity(id),
 entity_b TEXT NOT NULL REFERENCES entity(id),
 score REAL NOT NULL CHECK(score BETWEEN 0 AND 1),
 state TEXT NOT NULL CHECK(state IN ('open','accepted','dismissed')),
 UNIQUE(entity_a, entity_b),
 CHECK(entity_a < entity_b)
) STRICT;
-- Interactive drops outrank a bulk dump; lazy backfill runs last.
ALTER TABLE document_evaluation ADD COLUMN priority INTEGER NOT NULL DEFAULT 1;
PRAGMA user_version = 15;
