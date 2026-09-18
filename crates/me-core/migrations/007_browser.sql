CREATE TABLE document_folder (
    item_id INTEGER PRIMARY KEY REFERENCES collection_item(local_id),
    path TEXT NOT NULL
);
UPDATE property_definition SET label='Tax ID' WHERE key='person.tax_id';
UPDATE property_definition SET label='Social insurance number' WHERE key='person.social_insurance_number';
UPDATE property_definition SET label='Date of birth' WHERE key='person.birth_date';
UPDATE document_evaluation SET warning_message='Previous analysis left details unverified. Analyze again to review them.' WHERE warning_message LIKE 'Die frühere Auswertung%';
PRAGMA user_version=7;
