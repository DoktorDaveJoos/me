-- Native records share credential-only protection and metadata queries with imports.
ALTER TABLE credential_record RENAME TO credential_record_v12;
CREATE TABLE credential_record (
 item_id INTEGER PRIMARY KEY REFERENCES collection_item(local_id),
 import_id TEXT REFERENCES credential_import(id),
 account_uuid TEXT NOT NULL, vault_uuid TEXT NOT NULL, item_uuid TEXT NOT NULL,
 fingerprint TEXT NOT NULL, vault_name TEXT NOT NULL, category TEXT NOT NULL,
 archived INTEGER NOT NULL CHECK(archived IN (0,1)),
 raw_json TEXT NOT NULL CHECK(json_valid(raw_json)),
 attachment_paths_json TEXT NOT NULL CHECK(json_valid(attachment_paths_json)),
 format TEXT NOT NULL DEFAULT '1pux' CHECK(format IN ('1pux','me-v1')),
 CHECK((format='1pux' AND import_id IS NOT NULL) OR (format='me-v1' AND import_id IS NULL)),
 UNIQUE(account_uuid,vault_uuid,item_uuid,fingerprint)
) STRICT;
INSERT INTO credential_record SELECT item_id,import_id,account_uuid,vault_uuid,item_uuid,fingerprint,vault_name,category,archived,raw_json,attachment_paths_json, '1pux' FROM credential_record_v12;
DROP TABLE credential_record_v12;
PRAGMA user_version = 13;
