-- Personal knowledge and authority records. All content remains inside SQLCipher.
CREATE TABLE entity_identifier (
 id TEXT PRIMARY KEY, entity_id TEXT NOT NULL REFERENCES entity(id),
 namespace TEXT NOT NULL, value TEXT NOT NULL,
 UNIQUE(entity_id,namespace,value)
) STRICT;
-- Merges never rewrite original assertions or evidence. Reversal is an event.
CREATE INDEX entity_identifier_lookup ON entity_identifier(namespace,value);
CREATE TABLE entity_merge (
 id TEXT PRIMARY KEY, from_id TEXT NOT NULL REFERENCES entity(id),
 into_id TEXT NOT NULL REFERENCES entity(id), actor_id TEXT NOT NULL,
 recorded_at TEXT NOT NULL, reversed_at TEXT,
 CHECK(from_id<>into_id)
) STRICT;
CREATE UNIQUE INDEX entity_active_merge ON entity_merge(from_id) WHERE reversed_at IS NULL;
CREATE INDEX entity_merge_target ON entity_merge(into_id) WHERE reversed_at IS NULL;
CREATE INDEX assertion_property_time ON assertion(property_key,subject_id,valid_from,valid_to);
CREATE TABLE observation (
 id TEXT PRIMARY KEY, source_id TEXT NOT NULL REFERENCES source(id),
 run_id TEXT REFERENCES extraction_run(id), candidate_id TEXT NOT NULL,
 property_hint TEXT NOT NULL, value_json TEXT NOT NULL CHECK(json_valid(value_json)),
 subject_quote TEXT NOT NULL, context_quote TEXT NOT NULL,
 locator_json TEXT NOT NULL CHECK(json_valid(locator_json)),
 verification TEXT NOT NULL CHECK(verification IN ('supported','unverified','manual')),
 recorded_at TEXT NOT NULL, UNIQUE(source_id,run_id,candidate_id)
) STRICT;
CREATE TABLE observation_resolution (
 id TEXT PRIMARY KEY, observation_id TEXT NOT NULL REFERENCES observation(id),
 assertion_id TEXT NOT NULL REFERENCES assertion(id), actor_id TEXT NOT NULL,
 recorded_at TEXT NOT NULL, UNIQUE(observation_id,assertion_id)
) STRICT;
CREATE TABLE assertion_derivation (
 assertion_id TEXT NOT NULL REFERENCES assertion(id),
 input_id TEXT NOT NULL REFERENCES assertion(id), rule_version TEXT NOT NULL,
 PRIMARY KEY(assertion_id,input_id), CHECK(assertion_id<>input_id)
) STRICT;
CREATE TABLE credential_identity (
 id TEXT PRIMARY KEY REFERENCES collection_item(stable_id),
 entity_id TEXT REFERENCES entity(id), service_id TEXT REFERENCES entity(id)
) STRICT;
-- One immutable protected payload version includes unknown import fields and secrets.
-- Existing credential_record is the editable compatibility projection.
CREATE TABLE secret_version (
 id TEXT PRIMARY KEY, credential_id TEXT NOT NULL REFERENCES credential_identity(id),
 parent_id TEXT REFERENCES secret_version(id), format TEXT NOT NULL,
 payload_json TEXT NOT NULL CHECK(json_valid(payload_json)), recorded_at TEXT NOT NULL
) STRICT;
CREATE TABLE secret_head (
 credential_id TEXT PRIMARY KEY REFERENCES credential_identity(id),
 version_id TEXT NOT NULL REFERENCES secret_version(id)
) STRICT;
CREATE TABLE principal (
 id TEXT PRIMARY KEY, label TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('user','agent','service')),
 disabled_at TEXT
) STRICT;
CREATE TABLE capability_grant (
 id TEXT PRIMARY KEY, principal_id TEXT NOT NULL REFERENCES principal(id),
 task_id TEXT NOT NULL REFERENCES action_intent(id), resource_kind TEXT NOT NULL, resource_id TEXT NOT NULL,
 operation TEXT NOT NULL CHECK(operation IN ('read','credential_use','secret_reveal','draft','execute')),
 purpose TEXT NOT NULL, issued_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
 revoked_at INTEGER, device_id TEXT NOT NULL, CHECK(expires_at>issued_at)
) STRICT;
CREATE TABLE action_intent (
 id TEXT PRIMARY KEY, title TEXT NOT NULL, entity_id TEXT REFERENCES entity(id),
 source_id TEXT REFERENCES source(id), due_date TEXT, deadline_assertion_id TEXT REFERENCES assertion(id),
 created_at TEXT NOT NULL, deleted_at TEXT
) STRICT;
CREATE TABLE action_revision (
 id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES action_intent(id),
 parent_id TEXT REFERENCES action_revision(id), operation TEXT NOT NULL,
 recipient TEXT NOT NULL, body TEXT NOT NULL,
 attachments_json TEXT NOT NULL CHECK(json_valid(attachments_json)),
 recorded_at TEXT NOT NULL
) STRICT;
CREATE TABLE action_head (
 task_id TEXT PRIMARY KEY REFERENCES action_intent(id), revision_id TEXT NOT NULL REFERENCES action_revision(id)
) STRICT;
CREATE TABLE action_approval (
 id TEXT PRIMARY KEY, revision_id TEXT NOT NULL REFERENCES action_revision(id),
 actor_id TEXT NOT NULL, recorded_at TEXT NOT NULL, revoked_at TEXT
) STRICT;
CREATE TABLE action_attempt (
 id TEXT PRIMARY KEY, revision_id TEXT NOT NULL REFERENCES action_revision(id),
 approval_id TEXT NOT NULL REFERENCES action_approval(id), grant_id TEXT NOT NULL REFERENCES capability_grant(id),
 device_id TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('started','succeeded','failed','indeterminate')),
 started_at TEXT NOT NULL, finished_at TEXT
) STRICT;
CREATE UNIQUE INDEX action_single_attempt ON action_attempt(revision_id);
CREATE TABLE access_audit (
 id TEXT PRIMARY KEY, principal_id TEXT NOT NULL, task_id TEXT NOT NULL,
 resource_kind TEXT NOT NULL, resource_id TEXT NOT NULL, operation TEXT NOT NULL,
 grant_id TEXT, outcome TEXT NOT NULL CHECK(outcome IN ('allowed','denied','revoked')),
 recorded_at TEXT NOT NULL, device_id TEXT NOT NULL
) STRICT;
CREATE TABLE domain_revision (
 sequence INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE,
 record_kind TEXT NOT NULL, record_id TEXT NOT NULL,
 parents_json TEXT NOT NULL CHECK(json_valid(parents_json) AND json_type(parents_json)='array'),
 operation TEXT NOT NULL CHECK(operation IN ('put','delete')),
 payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
 format_version INTEGER NOT NULL CHECK(format_version=1),
 key_scope TEXT NOT NULL CHECK(key_scope IN ('personal','credential','authority')),
 actor_id TEXT NOT NULL, device_id TEXT NOT NULL, recorded_at TEXT NOT NULL
) STRICT;
CREATE TABLE domain_head (
 record_kind TEXT NOT NULL, record_id TEXT NOT NULL,
 revision_id TEXT NOT NULL REFERENCES domain_revision(id), PRIMARY KEY(record_kind,record_id)
) STRICT;
-- Reuse identical ciphertext when a transport retries an immutable revision.
CREATE TABLE replication_envelope (
 revision_id TEXT PRIMARY KEY REFERENCES domain_revision(id), ciphertext BLOB NOT NULL
) STRICT;
CREATE TABLE replication_asset_envelope (
 kind TEXT NOT NULL, asset_id TEXT NOT NULL, ciphertext BLOB NOT NULL,
 PRIMARY KEY(kind,asset_id)
) STRICT;
CREATE INDEX domain_record_history ON domain_revision(record_kind,record_id,sequence);
CREATE TRIGGER domain_revision_immutable BEFORE UPDATE ON domain_revision
BEGIN SELECT RAISE(ABORT,'domain revisions are immutable'); END;
CREATE TRIGGER secret_version_immutable BEFORE UPDATE ON secret_version
BEGIN SELECT RAISE(ABORT,'secret versions are immutable'); END;
CREATE TRIGGER observation_immutable BEFORE UPDATE ON observation
BEGIN SELECT RAISE(ABORT,'observations are immutable'); END;
CREATE TRIGGER action_revision_immutable BEFORE UPDATE ON action_revision
BEGIN SELECT RAISE(ABORT,'action revisions are immutable'); END;
CREATE TRIGGER access_audit_immutable BEFORE UPDATE ON access_audit
BEGIN SELECT RAISE(ABORT,'audit events are immutable'); END;
PRAGMA user_version=14;
