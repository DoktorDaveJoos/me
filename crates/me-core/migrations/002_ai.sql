-- Typed MVP identity fields; existing free notes are not silently reclassified.
INSERT INTO property_definition VALUES
 ('person.tax_id','Steuer-ID','identifier','one','confirm','{}',1),
 ('person.social_insurance_number','Sozialversicherungsnummer','identifier','one','confirm','{}',1),
 ('person.birth_date','Geburtsdatum','date','one','confirm','{}',1);
-- Model outputs stay proposals until a user makes an explicit decision.
CREATE TABLE ai_proposal (
 id TEXT PRIMARY KEY,
 source_id TEXT NOT NULL REFERENCES source(id),
 property_key TEXT NOT NULL REFERENCES property_definition(key),
 value TEXT NOT NULL,
 quote TEXT NOT NULL,
 segment_id TEXT NOT NULL,
 subject_quote TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('proposed','accepted','rejected')),
 run_id TEXT NOT NULL REFERENCES extraction_run(id),
 UNIQUE(source_id,property_key,value,segment_id)
) STRICT;
PRAGMA user_version = 2;
