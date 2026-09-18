"""Synthetic design checks, not a test of encryption, AI quality or production code."""
import json
import sqlite3
import struct
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent
NOW = "2026-09-14T12:00:00Z"


class SchemaChecks(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(":memory:")
        self.db.executescript((ROOT / "schema.sql").read_text())
        self.db.executemany("INSERT INTO entity VALUES(?,?,?,?,NULL)", [
            ("p1", "person", "SYNTHETIC Person A", NOW),
            ("p2", "person", "SYNTHETIC Person B", NOW),
            ("c1", "certificate", "SYNTHETIC First Aid", NOW),
            ("employment", "employment", "SYNTHETIC Employment", NOW),
            ("payroll-sep", "payroll", "SYNTHETIC September", NOW),
        ])
        props = [
            ("person.social_insurance_number", "identifier", "one", "confirm"),
            ("person.height", "quantity", "one", "confirm"),
            ("person.holds_certificate", "entity", "many", "confirm"),
            ("employment.base_salary", "money", "one", "confirm"),
            ("payroll.gross_total", "money", "one", "append_period"),
        ]
        self.db.executemany("INSERT INTO property_definition VALUES(?,?,?,?,?,'{}',1)",
                            [(key,key,typ,card,policy) for key,typ,card,policy in props])
        self.add_source("s1", "SYNTHETIC Certificate document")
        self.db.execute("INSERT INTO source_segment VALUES(1,'seg1','s1',1,0,?,?,'hash-seg1')",
                        ('{"page":1}', "SYNTHETIC Erste Hilfe Kurs erfolgreich absolviert"))
        self.db.execute("INSERT INTO source_entity VALUES('s1','c1','documents','accepted')")
        self.add_source("letter", "SYNTHETIC Request letter")
        self.db.execute("INSERT INTO source_segment VALUES(2,'seg-letter','letter',1,0,?,?,'hash-letter')",
                        ('{"page":1}', "SYNTHETIC Bitte senden Sie den angeforderten Zertifikatsnachweis."))
        self.db.execute("INSERT INTO source_entity VALUES('letter','p1','recipient','accepted')")
        self.db.execute("INSERT INTO source_entity VALUES('letter','c1','requests','accepted')")
        self.add_assertion("a-id", "p1", "person.social_insurance_number", "identifier", "001234-TEST")
        self.add_assertion("a-height", "p1", "person.height", "quantity", {"value":"182", "unit":"cm"})
        self.add_assertion("a-cert", "p1", "person.holds_certificate", "entity", "c1", obj="c1")
        self.accept("a-id"); self.accept("a-height"); self.accept("a-cert")

    def tearDown(self):
        self.db.close()

    def add_source(self, sid, title, sensitivity="personal"):
        self.db.execute("""INSERT INTO source
          (id,kind,title,received_at,content_fingerprint,sensitivity,retention,object_id)
          VALUES(?, 'file', ?, ?, ?, ?, 'keep', ?)""",
          (sid,title,NOW,"fingerprint-"+sid,sensitivity,"encrypted-object-"+sid))

    def add_assertion(self, aid, subject, key, typ, value, obj=None,
                      time_kind="timeless", start=None, end=None):
        self.db.execute("""INSERT INTO assertion
          (id,subject_id,property_key,value_type,value_json,object_entity_id,
           canonical_value,semantic_key,time_kind,valid_from,valid_to,recorded_at,origin)
          VALUES(?,?,?,?,?,?,?,?,?,?,?,?, 'user')""",
          (aid,subject,key,typ,json.dumps(value),obj,json.dumps(value,sort_keys=True),
           "semantic-"+aid,time_kind,start,end,NOW))

    def accept(self, aid):
        self.db.execute("""INSERT INTO decision
          (id,assertion_id,action,actor,reason_code,recorded_at)
          VALUES(?,?,'accept','user','explicit_confirmation',?)""", ("d-"+aid,aid,NOW))

    def test_leading_zero_identifier_is_preserved(self):
        value = self.db.execute("SELECT json_extract(value_json,'$') FROM assertion_state WHERE id='a-id'").fetchone()[0]
        self.assertEqual(value,"001234-TEST")

    def test_mismatching_property_type_rejected(self):
        with self.assertRaises(sqlite3.IntegrityError):
            self.add_assertion("bad", "p1", "person.height", "identifier", "182")

    def test_numeric_identifier_rejected(self):
        with self.assertRaises(sqlite3.IntegrityError):
            self.add_assertion("bad", "p1", "person.social_insurance_number", "identifier", 1234)

    def test_relation_json_and_foreign_key_must_agree(self):
        with self.assertRaises(sqlite3.IntegrityError):
            self.add_assertion("bad", "p1", "person.holds_certificate", "entity", "c1", obj="p2")

    def test_invalid_interval_rejected(self):
        with self.assertRaises(sqlite3.IntegrityError):
            self.add_assertion("bad", "employment", "employment.base_salary", "money",
                {"amount":"6000.00", "currency":"EUR"}, time_kind="interval",
                start="2026-09-01", end="2026-08-01")

    def test_proposal_does_not_overwrite_accepted_identifier(self):
        self.add_assertion("new-id","p1","person.social_insurance_number","identifier","DIFFERENT-TEST")
        rows = self.db.execute("SELECT id,state FROM assertion_state WHERE property_key='person.social_insurance_number' ORDER BY id").fetchall()
        self.assertEqual(rows,[("a-id","accept"),("new-id","proposed")])

    def test_review_groups_old_and_new(self):
        self.add_assertion("new-id","p1","person.social_insurance_number","identifier","DIFFERENT-TEST")
        self.db.execute("INSERT INTO review_case VALUES('r1','p1','conflicting_identifier','critical','open',?,NULL)",(NOW,))
        self.db.executemany("INSERT INTO review_member VALUES('r1',?)",[("a-id",),("new-id",)])
        self.assertEqual(self.db.execute("SELECT count(*) FROM review_member WHERE review_id='r1'").fetchone()[0],2)

    def test_retraction_keeps_history_but_changes_projection(self):
        self.db.execute("INSERT INTO decision(id,assertion_id,action,actor,reason_code,recorded_at) VALUES('undo','a-height','retract','user','correction',?)",(NOW,))
        self.assertEqual(self.db.execute("SELECT state FROM assertion_state WHERE id='a-height'").fetchone()[0],"retract")
        self.assertEqual(self.db.execute("SELECT count(*) FROM decision WHERE assertion_id='a-height'").fetchone()[0],2)

    def test_payroll_does_not_change_contract_salary(self):
        self.add_assertion("salary","employment","employment.base_salary","money",{"amount":"6000.00","currency":"EUR"})
        self.add_assertion("gross","payroll-sep","payroll.gross_total","money",{"amount":"7000.00","currency":"EUR"})
        self.accept("salary"); self.accept("gross")
        self.assertEqual(self.db.execute("SELECT json_extract(value_json,'$.amount') FROM assertion_state WHERE subject_id='employment' AND state='accept'").fetchone()[0],"6000.00")

    def test_duplicate_delivery_rejected(self):
        row=("inbox1","drop","scanner1","delivery1",NOW,"received")
        self.db.execute("INSERT INTO inbox_item VALUES(?,?,?,?,?,?)",row)
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.execute("INSERT INTO inbox_item VALUES(?,?,?,?,?,?)",("inbox2",)+row[1:])

    def test_combined_fact_relation_document_lookup(self):
        facts=self.db.execute("SELECT property_key FROM assertion_state WHERE subject_id=? AND state='accept' AND value_type<>'entity'",("p1",)).fetchall()
        docs=self.db.execute("""SELECT se.source_id FROM accepted_edges e
          JOIN source_entity se ON se.entity_id=e.object_entity_id AND se.status='accepted'
          WHERE e.subject_id=? AND e.property_key='person.holds_certificate'""",("p1",)).fetchall()
        self.assertEqual(set(f[0] for f in facts),{"person.social_insurance_number","person.height"})
        self.assertEqual(set(docs),{("s1",),("letter",)})
        other=self.db.execute("SELECT * FROM accepted_edges WHERE subject_id='p2'").fetchall()
        self.assertEqual(other,[])

    def test_full_text_search_finds_passage(self):
        self.assertEqual(self.db.execute("SELECT rowid FROM segment_fts WHERE segment_fts MATCH ?",('"Erste Hilfe"',)).fetchall(),[(1,)])

    def test_credential_sources_cannot_be_indexed(self):
        self.add_source("secret","SYNTHETIC credential", "credential")
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.execute("INSERT INTO source_segment VALUES(3,'seg-secret','secret',1,0,'{}','NEVER INDEX','x')")

    def test_equal_content_can_have_distinct_delivery_context(self):
        self.add_source("s-repeat", "SYNTHETIC repeated statement")
        self.db.execute("UPDATE source SET content_fingerprint='fingerprint-s1' WHERE id='s-repeat'")
        self.assertEqual(self.db.execute("SELECT count(*) FROM source WHERE content_fingerprint='fingerprint-s1'").fetchone()[0],2)

    def test_full_text_candidate_filter_respects_supplied_source_scope(self):
        rows=self.db.execute("""SELECT s.id FROM segment_fts f
          JOIN source_segment s ON s.rowid=f.rowid
          WHERE segment_fts MATCH ? AND s.source_id IN (SELECT value FROM json_each(?))""",
          ('"Erste Hilfe"',json.dumps(["letter"]))).fetchall()
        self.assertEqual(rows,[])

    def test_embedding_model_dimensions_enforced(self):
        self.db.execute("INSERT INTO embedding_model VALUES('demo','synthetic','1',3,'unit','1','cosine')")
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.execute("INSERT INTO chunk_embedding VALUES('seg1','demo',3,?)",(b'bad',))
        self.db.execute("INSERT INTO chunk_embedding VALUES('seg1','demo',3,?)",(struct.pack('<fff',1,0,0),))

    def test_purge_removes_search_and_vectors(self):
        self.db.execute("INSERT INTO embedding_model VALUES('demo','synthetic','1',3,'unit','1','cosine')")
        self.db.execute("INSERT INTO chunk_embedding VALUES('seg1','demo',3,?)",(struct.pack('<fff',1,0,0),))
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.execute("UPDATE source SET retention='purged',object_id=NULL WHERE id='s1'")
        self.db.execute("DELETE FROM source_segment WHERE source_id='s1'")
        self.db.execute("UPDATE source SET retention='purged',object_id=NULL,title='[purged]' WHERE id='s1'")
        self.assertEqual(self.db.execute("SELECT count(*) FROM chunk_embedding").fetchone()[0],0)
        self.assertEqual(self.db.execute("SELECT rowid FROM segment_fts WHERE segment_fts MATCH 'Hilfe'").fetchall(),[])

    def test_assertions_cannot_be_silently_edited(self):
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.execute("UPDATE assertion SET value_json='\"other\"' WHERE id='a-id'")

    def test_integrity(self):
        self.assertEqual(self.db.execute("PRAGMA foreign_key_check").fetchall(),[])
        self.assertEqual(self.db.execute("PRAGMA integrity_check").fetchone()[0],"ok")


if __name__ == "__main__":
    print(f"Synthetic schema checks; SQLite {sqlite3.sqlite_version}; no encryption or model calls.", flush=True)
    unittest.main(verbosity=2)
