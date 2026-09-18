-- Unverified model suggestions are questions, never source evidence or facts.
CREATE TABLE ai_question (
 id TEXT PRIMARY KEY,
 source_id TEXT NOT NULL REFERENCES source(id),
 candidate_key TEXT NOT NULL,
 property_key TEXT NOT NULL REFERENCES property_definition(key),
 value TEXT NOT NULL,
 claimed_quote TEXT NOT NULL,
 claimed_subject TEXT NOT NULL,
 segment_id TEXT,
 reason_code TEXT NOT NULL,
 run_id TEXT NOT NULL REFERENCES extraction_run(id),
 state TEXT NOT NULL CHECK(state IN ('pending','confirmed','dismissed','resolved')),
 answer_value TEXT,
 UNIQUE(source_id,candidate_key)
) STRICT;
UPDATE document_evaluation SET warning_message='Die frühere Auswertung hat unsichere Angaben nicht gespeichert. Bitte erneut auswerten, um sie einzeln zu prüfen.' WHERE warning_message LIKE '%ausgelassen%';
PRAGMA user_version = 5;
