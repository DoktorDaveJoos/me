ALTER TABLE document_evaluation ADD COLUMN warning_message TEXT;
CREATE TABLE extraction_checkpoint (
 source_id TEXT NOT NULL REFERENCES source(id),
 pipeline TEXT NOT NULL,
 batch_key TEXT NOT NULL,
 output_json TEXT NOT NULL,
 PRIMARY KEY(source_id,pipeline,batch_key)
) STRICT;
PRAGMA user_version = 4;
