# ME. — Datenhaltung, Suche und Agentenanbindung

> Historische Recherche. Die damalige Annahme eines kostenlosen quelloffenen
> Produkts ist überholt. Maßgeblich sind der aktuelle [README](../../../README.md)
> und die [Lizenz](../../../LICENSE): Quelltexteinsicht ist erlaubt, Nutzung
> erfordert eine separate bezahlte Vereinbarung.

Entscheidungsentwurf · Recherche vom 14. September 2026

**KI-Festlegung vom 15. September:** ME. nutzt Codex App Server mit dem bestehenden Nutzerabo für die Inbox. Interaktive Fragen und ausführende Aufgaben laufen zunächst in der Codex-App, die ME. über MCP verwendet. Andere Agentenruntimes und ein eigener Computer-Use-Unterbau gehören nicht zum MVP. Das [KI-Konzept für Inbox, Fragen und ausführende Aufgaben](../2026-09-15/KI-KONZEPT.md) konkretisiert diese Richtung und ersetzt die folgenden Empfehlungen zu einem frühen parallelen Adaptervergleich.

**Ergänzung vom 15. September:** Die Datenbankentscheidung wird in [Desktop, iPhone und Chrome mit ME. Sync](../2026-09-15/SYNC-ENTSCHEIDUNG.md) unter den geräteübergreifenden Anforderungen erneut bewertet. PostgreSQL auf dem Desktop bleibt eine tragfähige Alternative; die folgende Erstempfehlung ist keine endgültige Festlegung.

**Empfehlung:** Ein lokaler SQLCipher-Tresor auf SQLite-Basis, verschlüsselte Originaldateien und eine Suche, die strukturierte Abfragen, Volltext, Vektoren und explizite Beziehungen verbindet. Der Agent benutzt begrenzte ME.-Werkzeuge. Er bekommt weder die Datenbank noch ihren Schlüssel. Pi ist ein geeigneter austauschbarer Agent; ME. verantwortet Wissen, Rechte und Zustandsänderungen.

Diese Ausarbeitung berücksichtigt die aktuelle Richtung aus dem Gespräch: ME. als kostenloser, quelloffener persönlicher Tresor mit Inbox und Agentenanbindung, perspektivisch bezahlter verschlüsselter Sync. Die vorhandenen Repository-Dokumente nennen teilweise noch Closed Source und OpenAI-exklusives Pro. Diese älteren Festlegungen wurden hier nicht übernommen. Repository-Code, Lizenz und bestehende Dokumente wurden nicht geändert.

Die Dateien `schema.sql` und `verify_schema.py` sind ein ausführbarer **Schemaentwurf mit synthetischen Daten**. Sie sind keine implementierte Tresorfunktion. Das Schema wurde unter SQLite 3.51.2 in einer In-Memory-Datenbank geprüft; SQLCipher, KI-Extraktion, echte Embeddings, Betriebssystemisolation und Sync wurden nicht ausgeführt.

## 1. Die Datenbankentscheidung

| Bedarf | Empfehlung | Begründung |
|---|---|---|
| Angaben, Quellen, Entscheidungen, Aufgaben | SQLite mit SQLCipher | Eine lokale Transaktionsgrenze, keine separate Datenbankinstallation |
| Original-PDFs, Bilder, E-Mails | Verschlüsselter Objektspeicher auf dem Gerät | Große Dateien unabhängig von Datenbankseiten sichern und übertragen |
| Wortlaut, Namen, Nummern, Dokumenttext | SQLite FTS5 plus normale SQL-Indizes | Exakte Suche und Volltext in demselben Tresor |
| Sinngemäß passende Textstellen | Lokale Embeddings; zunächst exakter Vektorvergleich | Semantische Suche ohne zusätzlichen Datenbankdienst |
| Zusammenhänge | Entitäten und bestätigte Beziehungen in SQL | Quellen und Gültigkeit bleiben Teil jeder Beziehung |
| Späterer Cloud-Sync | Server-Metadatenbank und verschlüsselte Objekte/Änderungen | Server muss den persönlichen Inhalt nicht entschlüsseln |

**Eine lokale Datenbank, mehrere Indizes.** Eine Vektordatenbank ist eine mögliche technische Umsetzung der Ähnlichkeitssuche, aber keine Voraussetzung für sie.

SQLCipher verschlüsselt SQLite-Seiten und die Datenseiten in Journal/WAL. Temporäre Dateien verlangen eine passende Konfiguration; der Hersteller verlangt, dateibasierte temporäre Speicher zu vermeiden. Der Rust-Zugriff kann über `rusqlite` erfolgen, das SQLCipher-Buildoptionen anbietet. Die Community-Ausgabe von SQLCipher kann unter Einhaltung ihrer Lizenzhinweise auch in Open-Source-Produkten eingesetzt werden. Das ist keine Garantie für die Sicherheit der gesamten Anwendung. [SQLCipher-Design](https://www.zetetic.net/sqlcipher/design/), [Community-Ausgabe](https://www.zetetic.net/sqlcipher/community/), [rusqlite](https://github.com/rusqlite/rusqlite)

Produktions-Buildentscheidung: `rusqlite` mit einer gepinnten SQLCipher-Variante; Kryptoanbieter und FTS5-Verfügbarkeit auf macOS und Linux testen. Nicht versehentlich zusätzlich eine unverschlüsselte SQLite-Implementierung durch eine zweite Abhängigkeit einbinden. Keine zur Laufzeit frei ladbaren Datenbankerweiterungen für Agenten.

### Alternativen

| Option | Was sie leisten kann | Entscheidung für ME. |
|---|---|---|
| PostgreSQL + pgvector | Relationale Daten und exakte/approximative Vektorsuche | Gute Serveroption; zusätzlicher lokaler Dienst für den Desktop unnötig |
| Qdrant | Vektorsuche, Filter und kombinierte Suchabfragen | Später prüfen, falls gemessene Größe/Last den lokalen Ansatz überfordert |
| Eigene Graphdatenbank | Spezialisierte Graphabfragen | Für Personen, Verträge, Nachweise und Briefe zunächst kein Bedarf |
| Reine Dokument-/JSON-Datenbank | Flexible Objekte | Beziehungen, Quellen und Validierungsregeln wären weiterhin selbst zu bauen |
| Ein einziger Vektorindex | Ähnlichkeit zwischen Texten | Kann bestätigte Werte, Widersprüche und Zeiträume nicht ersetzen |

pgvector dokumentiert beide Suchmodi; Qdrant dokumentiert mehrstufige hybride Abfragen. Das sind reale Fähigkeiten, aber keine Notwendigkeit, diese Produkte in ME. zu betreiben. [pgvector](https://github.com/pgvector/pgvector), [Qdrant Query API](https://qdrant.tech/documentation/search/hybrid-queries/)

## 2. Wissensmodell: Aussagen mit Belegen

Das Modell unterscheidet **eingehenden Inhalt**, **daraus gewonnene Aussagen**, **deren Bewertung** und **darauf beruhende Handlungen**. Der Suchindex ist abgeleitet und wiederaufbaubar.

### Tabellen und Beziehungen

| Tabelle | Verantwortung |
|---|---|
| `entity` | Person, Organisation, Beschäftigung, Versicherung, Ausweis, Zertifikat, Abrechnung, Vorgang |
| `entity_alias` | Schreibweisen wie „TK“ und „Techniker Krankenkasse“, jeweils an eine Identität gebunden |
| `property_definition` | Kanonische Feldbedeutung, Datentyp, Kardinalität und Änderungspolitik |
| `inbox_item` | Eingangskanal, Lieferkennung, Empfang und Verarbeitungsstatus |
| `source` | Originalquelle, Datum, Sensitivität, Aufbewahrung und verschlüsseltes Dateiobjekt |
| `inbox_source` / `source_link` | E-Mail mit Anhängen, Sammelscan mit Teilbriefen, mehrere Eingänge derselben Datei |
| `source_entity` | Zuordnung von Quellen zu Personen/Vorgängen/Nachweisen, mit Bestätigungsstatus |
| `source_segment` | Extrahierte Textpassagen mit Seite/Region und Verarbeitungsversion |
| `extraction_run` | Herkunft einer Extraktion: Provider, Modell und Pipeline-/Schemaversion |
| `assertion` | Aussage über eine Entität, einschließlich Wert und Zeitbezug |
| `assertion_evidence` | Eine oder mehrere Quellen und genaue Fundstellen zu einer Aussage |
| `decision` | Annahme, Ablehnung oder Rücknahme durch Nutzer beziehungsweise versionierte Regel |
| `review_case` / `review_member` | Zusammenhängende Klärung statt vieler einzelner Bestätigungen |
| `task` / `job` | Nutzeraufgabe und wiederaufnehmbare technische Verarbeitung |
| `embedding_model` / `chunk_embedding` | Versionierte Suchvektoren getrennt von fachlichem Wissen |

`accepted_edges` ist eine Sicht auf akzeptierte Aussagen mit einer anderen Entität als Wert. So verwenden Werte und Beziehungen dasselbe Quellen- und Entscheidungsmodell. Es gibt keinen zweiten, unabhängig gepflegten „KI-Graphen“.

Beispiel:

```text
Person A — hält Zertifikat → Zertifikat C
Zertifikat C — ausgestellt von → Organisation O
Zertifikat C — belegt durch → Quelldokument S
Brief B — betrifft → Vorgang V
Vorgang V — betrifft Person → Person A
```

Die ersten beiden Verbindungen sind fachliche Aussagen; die Dokumentzuordnung liegt in `source_entity`. Auch die Graphsuche muss Zeit und Status filtern. SQLite unterstützt dafür unter anderem rekursive Abfragen. Für ME. reichen anfangs begrenzte Abfragen über ein bis zwei Beziehungsschritte. [SQLite WITH/rekursive Abfragen](https://www.sqlite.org/lang_with.html)

### Werte und Zeit

- Kennnummern bleiben Text; führende Nullen und gültige Zeichen bleiben erhalten.
- Geld: Dezimalzeichenfolge plus ISO-Währung, beispielsweise `{"amount":"7000.00","currency":"EUR"}`. Geldberechnungen erfolgen im Core mit Dezimalarithmetik, nicht mit binären Fließkommazahlen.
- Körpergröße: Betrag plus Einheit, beispielsweise `{"value":"182","unit":"cm"}`; eine Messung kann einen Beobachtungstag haben. Daraus folgt kein unbegrenzt gültiger Wert.
- Ein Zertifikat ist eine eigene Entität mit Typ, Inhaber, Aussteller und gegebenenfalls Ausstellungs-/Ablaufdatum. Eine Kursteilnahme ist nicht automatisch eine bestandene Zertifizierung.
- Zeitarten: zeitlos, Intervall, Zeitpunkt oder unbekannt. Ein unbekannter Beginn ist keine stillschweigende Aussage „gilt schon immer“.
- Intervalle sind halboffen: `[valid_from, valid_to)`. Das Enddatum gehört nicht mehr zum Intervall.
- `recorded_at` beschreibt, wann ME. etwas erfahren hat. Es ersetzt nicht das Datum, ab dem die Angabe galt.
- Regeldefinitionen validieren die zulässige Kombination aus Entität, Feld, Wert, Einheit und Zeit. Freie Felder bleiben möglich und beginnen mit einer konservativen Regel.

Eine neue Abrechnung bekommt eine eigene Entität. Ihr Gesamtbrutto überschreibt nicht das Grundgehalt des Beschäftigungsverhältnisses. Ein neuer Reisepass bekommt eine eigene Entität; seine Nummer ersetzt nicht pauschal alle alten Passnummern.

### Entscheidungen und aktueller Wissensstand

Neue Aussagen beginnen als Vorschläge. Wiederholte äquivalente Aussagen können dieselbe normalisierte Aussage mit zusätzlichem Beleg stärken. Die dafür verwendete `semantic_key` berechnet der Core aus Subjekt, Bedeutung, normalisiertem Wert und Zeitbezug; die KI darf sie nicht bestimmen.

Es gibt keine globale Regel „neuestes Dokument gewinnt“. Die Auflösung arbeitet pro Feld und Zeitpunkt. Ein akzeptierter alter Wert bleibt erhalten; eine Korrektur wird als neue Aussage mit Bezug zur Vorgängeraussage angelegt. Eine echte zeitliche Änderung beendet das bisherige Intervall durch eine nachvollziehbare Revision und beginnt ein neues. Beide Schritte werden gemeinsam bestätigt.

Der Resolver liefert Zustände wie `resolved`, `missing`, `ambiguous`, `conflicting`, `stale` und `unverified`. Offene kritische Konflikte müssen auch dann sichtbar bleiben, wenn noch ein akzeptierter Bestandswert existiert. Kritische Vorschläge werden nicht aus Suchtreffern heimlich zu Formularwerten.

Das SQL-Demoschema speichert die Bausteine dafür. Der vollständige zeitliche Resolver und die fachliche Konflikterkennung sind **noch zu implementieren**. Die lokale Entscheidungsreihenfolge ist ausdrücklich kein Sync-Mergeverfahren.

## 3. Die Suche kombiniert vier Wege

### A. Strukturierte Abfrage

„Meine Sozialversicherungsnummer“ wird auf eine bekannte Feldbedeutung und die vom Nutzer bestimmte Person aufgelöst. Der Core liefert den bestätigten Wert, seinen Status und Belege. Keine semantische Näherung für Kennnummern.

Ebenso laufen Summen, Zeitfilter und Vollständigkeitsfragen über strukturierte Abfragen: „Welche zwölf Monate sind vorhanden?“ oder „Wie hoch ist die Summe dieser Abrechnungswerte?“ Eine Top-k-Dokumentsuche kann keine vollständige Mengenabfrage ersetzen.

### B. Volltext

FTS5 durchsucht Textpassagen nach Namen, Wortlaut und Begriffen. Der Index referenziert die kanonischen Passagen. FTS5 bietet BM25-Ranking; kleinere Werte sind dabei bessere Treffer. Der Standardtokenizer ersetzt keine deutsche Sprachverarbeitung. Aliasauflösung und kontrollierte Sucherweiterung ergänzen ihn. Indexpflege und Löschung erfolgen transaktional mit den Passagen. [SQLite FTS5](https://www.sqlite.org/fts5.html)

### C. Semantische Suche

Embeddings helfen, wenn die Anfrage „Nachweis über medizinische Erstversorgung“ lautet, das Zertifikat aber „Erste Hilfe“ enthält. Sie liefern Kandidaten, keinen Nachweis für eine fachliche Behauptung.

Dokumente werden nach Absätzen, Tabellen und Seitenstruktur segmentiert. Ausgangspunkt: etwa 250–400 Tokens, begrenzte Überlappung, Überschrift/Absender als Kontext; anschließend am echten Dokumentkorpus messen. Tabellenzeilen erhalten Spaltenüberschriften. Summaries sind Suchhilfen, zitierbar bleiben die Originalstellen. Anfragen können Nachbarpassagen oder die Originalseite nachladen.

**Embedding-Modell getrennt vom Gesprächsmodell halten.** Ein Wechsel von Codex zu Claude soll keine neue Indexierung erzwingen. Ein Abozugang garantiert zudem keinen geeigneten Embedding-Endpunkt.

Kandidaten für lokale Tests:

| Modell | Dokumentierte Eigenschaften | Rolle |
|---|---|---|
| `intfloat/multilingual-e5-small` | 384 Dimensionen, bis 512 Tokens, mehrsprachig; `query:`/`passage:`-Präfixe | Kompakte Referenz für Laptop/CPU |
| `BAAI/bge-m3` | 1024 Dimensionen, bis 8192 Tokens, mehrsprachig | Vergleichskandidat, wenn Qualität den höheren Ressourcenbedarf rechtfertigt |

Das ist keine Aussage, welches Modell für deutsche Briefe am besten ist. Modell, Tokenizer, Gewichtsrevision, Vorverarbeitung und Vektordimension werden festgehalten. Dokument- und Anfragevektoren müssen aus demselben kompatiblen Verfahren stammen. Geänderte Embeddings werden daneben aufgebaut und erst nach Fertigstellung aktiviert. [E5-Modellkarte](https://huggingface.co/intfloat/multilingual-e5-small/raw/main/README.md), [BGE-M3-Modellkarte](https://huggingface.co/BAAI/bge-m3/raw/main/README.md)

Für den Anfang liegen Float32-Vektoren als BLOB in SQLCipher. Ein Rust-Worker berechnet die Ähnlichkeit über die erlaubte Kandidatenmenge. 50.000 Vektoren mit 384 Dimensionen benötigen allein 76,8 MB Nutzdaten; Metadaten und Laufzeit kommen hinzu. Das ist eine Größenrechnung, kein Geschwindigkeitsbenchmark. Bei Bedarf wird ein Speicher-Cache beim Entsperren aufgebaut und beim Sperren freigegeben.

`sqlite-vec` ist eine interessante spätere Optimierung, aber laut Projekt noch vor Version 1 mit möglichen inkompatiblen Änderungen. SQLite `vec1` bietet ANN; seine Roadmap nennt noch unzureichende Tests. Keine der beiden Erweiterungen wird deshalb zur Voraussetzung des ersten Tresors. Vor Einsatz: SQLCipher-Zusammenspiel, Transaktionen, Filter, Löschung, Migration und Packaging prüfen. [sqlite-vec](https://github.com/asg017/sqlite-vec), [vec1](https://sqlite.org/vec1/doc/trunk/doc/vec1.md)

### D. Beziehungen erweitern und Ergebnisse prüfen

Zu passenden Zertifikaten werden Inhaber und Quelldokumente nachgeladen; zu einem Brief der verknüpfte Vorgang und vorhandene Antworten. Nur nachgewiesene Beziehungen gelten als bestätigt. Von der KI vermutete Zusammenhänge bleiben entsprechend markiert.

Volltext- und Vektortreffer werden zunächst getrennt ermittelt und über ihre Rangpositionen kombiniert, etwa mit Reciprocal Rank Fusion. Anschließend werden Treffer pro Dokument gebündelt und optional erneut bewertet. Rankingparameter sind Messgrößen, keine Wahrheitswahrscheinlichkeit. Qdrant dokumentiert dieses Verfahren einschließlich der Probleme beim direkten Addieren unterschiedlicher Rohscores. Der Ansatz lässt sich auch im ME.-Core umsetzen. [Hybride Suche/RRF](https://qdrant.tech/documentation/search/hybrid-queries/)

Autorisierung, Zeitfilter und Belegstatus haben Vorrang vor Ranking. Keine unerlaubten Texte an einen Cloud-Reranker senden. Bei ANN später Filter in der Suche berücksichtigen; bloß nachträgliches Entfernen aus einer kleinen Top-k-Liste kann relevante Treffer verlieren.

## 4. Der kombinierte Suchfall

Auftrag: „Bereite die Antwort vor. Benötigt werden meine Sozialversicherungsnummer, Körpergröße, ein bestimmtes Zertifikat und der passende Brief.“

1. Die Aufgabe bestimmt Person, Zweck und erlaubte Datenbereiche. „Ich“ stammt aus dem ME.-Profil, nicht aus einer Vermutung des Modells.
2. Der Agent zerlegt den Bedarf in vier Teile. Ist der Zertifikatstyp im Brief definiert, wird zuerst der Brief gelesen.
3. Exakte Angaben kommen aus `facts.get`; fehlende oder strittige Angaben bleiben als solche markiert.
4. `documents.search` sucht Zertifikat und Schriftverkehr über Volltext und Bedeutung.
5. `entities.related` und Quellenzuordnungen prüfen Inhaber, Aussteller, Zeitraum und Vorgang. Der Nachweis einer anderen Person ist kein Treffer für die eigene Qualifikation.
6. `evidence.read` lädt nur die maßgeblichen Stellen beziehungsweise Seiten nach.
7. Ein Kontextpaket enthält die benötigten Werte und Dokumentreferenzen, außerdem Konflikte, fehlende Angaben und Quellen. Ein irrelevanter Datenbereich wird nicht beigefügt.
8. Die Antwort wird als Entwurf mit verwendeten Quellen gespeichert. ME. verlangt eine erneute Prüfung, wenn sich Werte oder Anhänge seit Erstellung geändert haben.

Es gibt keine Forderung, dass sämtliche Inhalte schon beim Import perfekt strukturiert sein müssen. Dokumente können auch ohne vollständige Faktenextraktion durchsucht werden. Neu entdeckte Angaben werden als Vorschläge ergänzt; das Vorhandensein in einem Suchtreffer ist keine automatische Annahme.

Bei erfolgloser Suche wird kontrolliert erweitert: Alias/Synonym, größerer Zeitraum, weitere Kandidaten, dann Originalseite. Nach einem festen Suchbudget meldet ME. eine Lücke. Ein einzelnes leeres Top-k-Ergebnis beweist nicht, dass ein Dokument nicht existiert.

## 5. Werkzeugvertrag für die KI

Alle Adapter benutzen dieselben fachlichen Werkzeuge. Kein allgemeines `execute_sql`, keine Datenbankdatei im Agenten-Arbeitsordner und kein `export_all` als Bequemlichkeitswerkzeug.

| Werkzeug | Zweck |
|---|---|
| `schema.discover` | Relevante Feldbedeutungen, Datentypen und verfügbare Suchmöglichkeiten entdecken |
| `entities.resolve` | Namen/Aliase mit Mehrdeutigkeiten auf Entitäten auflösen |
| `facts.get` | Begrenzte Liste exakter Angaben, optional zu Datum/Zeitraum |
| `facts.aggregate` | Zulässige Summen/Zählungen über einen vollständig definierten Bereich |
| `documents.search` | Hybride Suche mit Personen-, Typ-, Zeit- und Vorgangsfiltern |
| `entities.related` | Benannte Beziehungstypen mit begrenzter Tiefe verfolgen |
| `evidence.read` | Zugelassene Quellenstellen oder Seiten abrufen |
| `changes.propose` | Extrahierte Angaben oder Änderungen zur Core-Prüfung einreichen |
| `tasks.prepare` | Antwort/Formular/Anhangsliste als versionierten Entwurf erstellen |

Freigabe und verbindliche Ausführung erfolgen über einen getrennten, vertrauenswürdigen Pfad. Der Agent kann keine Nutzerbestätigung erzeugen. Die Freigabe bindet sich an konkrete Entwurfsrevision, Empfänger und Anhänge. Ein geänderter Entwurf darf eine alte Freigabe nicht weiterverwenden.

Beispiel einer vorgeschlagenen Antwortform, mit ausschließlich synthetischen Daten:

```json
{
  "subject_id": "person-demo",
  "facts": [
    {
      "property": "person.height",
      "status": "resolved",
      "value": {"value": "182", "unit": "cm"},
      "assertion_id": "assertion-demo",
      "evidence": [{"source_id": "source-demo", "locator": {"page": 1}}],
      "validity": {"kind": "point", "date": "2026-08-01"}
    }
  ],
  "missing": ["requested_certificate"],
  "conflicts": [],
  "coverage": "requested_fields_only",
  "retrieval_revision": "revision-demo"
}
```

Toolargumente werden strukturell und fachlich validiert. Der Core verwendet parametrisierte SQL-Abfragen, normalisiert Werte selbst und setzt Obergrenzen für Treffer, Textmenge, Laufzeit und Beziehungstiefe. Suchbegriffe werden in eine kontrollierte FTS-Anfrage übersetzt. Interne Scores und ungefilterte Trefferzahlen müssen nicht an den Agenten gelangen.

Die Rechte stammen aus der realen Nutzersitzung und ihrer Aufgabe. Eine vom Modell behauptete `purpose`-Zeichenfolge erweitert sie nicht. Filter werden bei Kandidatenauswahl und unmittelbar vor Rückgabe geprüft; Ausführungen zusätzlich bei Verwendung. Widerrufene Rechte verhindern neue Datenlieferungen, können bereits übertragenen Kontext jedoch nicht zurückholen.

## 6. Inbox und Aufbewahrung

Alle Kanäle schreiben dieselben Eingangsobjekte. ME. bestätigt Übernahme erst nach verlässlicher lokaler Speicherung. Watchfolder warten auf vollständig geschriebene Dateien. Scannerdateien bleiben zunächst unangetastet. Technische Jobs verwenden Wiederholungskennungen, kurze Transaktionen, Leases und wiederaufnehmbare Schritte.

Dateiduplikat, wiederholte Aussage und korrigiertes Dokument sind unterschiedliche Fälle. Ein gleicher Dateiinhalt darf mehrere Eingänge erklären; eine neue Abrechnung mit gleichem Gehalt ist ein neuer Zeitraum. Der Inhaltsfingerprint ist deshalb kein globaler Unique-Key für Quellen: Identische Bytes können in einem anderen zeitlichen Kontext erneut eintreffen. Objektbytes dürfen dedupliziert werden, der Eingangskontext bleibt erhalten. Die Lieferkennung eines Connectors verhindert die versehentliche Wiederholung genau desselben Imports.

Die Pipeline entscheidet getrennt über Aufbewahrung, Wissensänderung und Handlung:

- TK-Mitteilung ohne neue relevante Angabe/Frist: kurze Information, vorübergehend lesbares Original, anschließend regelgesteuertes Entfernen.
- Lohnabrechnung: Original behalten, Monatswerte ergänzen; Grundgehaltsänderung nur bei entsprechendem Beleg.
- Abweichende Kennnummer oder Nachname: kritischen Konflikt bündeln; kein stilles Überschreiben.
- Unleserliche/mehrdeutige Quelle: keine kritischen Änderungen und kein automatisches Verwerfen.

„Entfernen“ umfasst Original, Vorschauen, OCR-Text, FTS-Einträge, Embeddings, temporäre Extraktionsergebnisse und gegebenenfalls Agentenprotokolle. Noch benötigte Belege müssen zuvor aufgelöst werden. Beim ausdrücklichen Löschen eines Nachweises muss die fachliche Entscheidung lauten: auch daraus entstandenes Wissen entfernen oder dieses auf ausdrücklichen Nutzerwunsch ohne Original weiterführen. Eine Aussage darf nicht weiter so erscheinen, als sei eine entfernte Quelle noch einsehbar.

Das Demoschema prüft **logisches** Entfernen aus Suche und Vektortabelle. Es beweist keine physische Löschung aus Dateisystem, Backups oder Cloud-Provider-Speichern. Wiederanlauf, Schlüsselstrategie und Backup-Aufbewahrung benötigen eigene Tests. Appseitige Löschung kann die Aufbewahrung beim gewählten Modellanbieter nicht garantieren.

## 7. Pi und andere Agenten

### Empfehlung

Die fachliche Schnittstelle heißt intern etwa `AgentRuntime`. ME. besitzt Datenmodell, Auftragswarteschlange, Suche und Berechtigungen; die Laufzeit bleibt austauschbar. Importjobs sind dauerhafte ME.-Jobs und keine bloß im Chat gespeicherten Pläne.

**Codex App Server ist der erste Referenzadapter für das vorhandene ChatGPT-Pro-Abo. Pi SDK ist der bevorzugte zweite Kandidat für die breitere Modellwahl.** Beide werden an derselben Aufgabe bewertet. Pi muss Codex nicht als weiteren Harness starten; verschachtelte Agentenschleifen erhöhen hier nur Aufwand und Kontextverbrauch.

| Kandidat | Passender Einsatz | Grenze |
|---|---|---|
| Codex App Server | Lokale Einbettung mit offizieller ChatGPT-Anmeldung und Ereignissen | Schnittstellenreife/Versionen prüfen; Codex-Werkzeuge auf ME.-Bedarf beschränken |
| Pi SDK in kleinem Node-Sidecar | Eigene ME.-Werkzeuge, kontrollierte Ressourcen, mehrere Modellanbieter | Zusätzliche Runtime und eigene Isolation erforderlich |
| Pi RPC | Schneller sprachunabhängiger Prototyp über JSONL | Ressourcen und Erweiterungen genauso bewusst begrenzen |
| Claude Agent SDK | Claude-spezifischer Adapter | Öffentliche Abointegration/Abrechnung gesondert klären |
| Direkte Modell-API / lokales Modell | Eng begrenzte Extraktion und alternative Provider | Eigenen Tool-Loop und Fehlerbehandlung übernehmen; API kann separat kosten |

Codex dokumentiert den App Server als Einbettungsschnittstelle mit verwalteter ChatGPT-Anmeldung. Einige Funktionen sind experimentell. Die Entscheidung nutzt die vorhandene Integration, setzt aber keine allgemeine API-Flatrate voraus. [Codex App Server](https://learn.chatgpt.com/docs/app-server), [Authentifizierung](https://learn.chatgpt.com/docs/auth)

Pi bietet SDK, RPC und benutzerdefinierte Werkzeuge. Beim SDK ist für ME. ein ausdrücklich konfigurierter ResourceLoader sinnvoll; globale Projektdateien, beliebige Erweiterungen und automatische Skills sollen nicht unbeabsichtigt in Dokumentverarbeitung gelangen. Sessions zunächst im Speicher, nur notwendige Ergebnisse verschlüsselt durch ME. persistieren. MCP ist bei Pi kein eingebauter Kernbestandteil: ME.-Tools direkt registrieren, MCP für andere Clients optional ergänzen. [Pi SDK](https://pi.dev/docs/latest/sdk), [Pi RPC](https://pi.dev/docs/latest/rpc), [Pi-Funktionsumfang](https://pi.dev/)

Pi besitzt laut eigener Dokumentation **keine eingebaute Sandbox**. Werkzeuge und Erweiterungen laufen mit Prozessrechten. Daher: kein Datenbankschlüssel im Sidecar, keine Tresordateien als Mount, keine allgemeinen Shell-/Dateiwerkzeuge für reine Extraktion, keine ungeprüften Erweiterungen. Prozessgrenzen allein sind keine Sicherheitsgrenzen gegen einen kompromittierten Prozess desselben Benutzers; echte Betriebssystemisolation muss auf beiden Zielsystemen nachgewiesen werden. [Pi Security](https://pi.dev/docs/latest/security)

Die Pi-Dokumentation nennt ChatGPT Plus/Pro als Abozugang und für Claude im Drittanbieter-Harness zusätzliche tokenbasierte Nutzung. Gleichzeitig beschreibt das Claude Help Center eine pausierte SDK-Abrechnungsänderung; die SDK-Dokumentation begrenzt Drittanbieter-Logins ohne Genehmigung. Diese Wege sind nicht gleichzusetzen. Kein „alle Abos ohne Zusatzkosten“-Versprechen. Limitüberschreitung führt zu `waiting_provider`, nicht zu einem stillen kostenpflichtigen Fallback. [Pi Provider](https://pi.dev/docs/latest/providers), [Claude SDK-Abrechnung](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan), [Claude SDK](https://code.claude.com/docs/en/agent-sdk)

## 8. Schlüssel, Geheimnisse und spätere Synchronisierung

ME. hält Datenbank- und Objektschlüssel im vertrauenswürdigen Core. Passwortableitung, Geräteschlüssel, Wiederherstellung und Schlüsselrotation bekommen eine gesondert geprüfte Konstruktion auf bestehenden Kryptobibliotheken. Originaldateien erhalten zufällige Objektkennungen; Dateinamen und Inhaltsfingerprints bleiben im verschlüsselten Bereich. Verschlüsselte Objekte müssen Formatversion, Integritätsprüfung und sicheren Schreibabschluss besitzen.

Loginpasswörter, TOTP-Secrets, private Schlüssel und ELSTER-Zertifikatsgeheimnisse gehören in einen eigenen Credential-Bereich. Sie werden weder in Volltext noch in Embeddings aufgenommen. PDF-Dokumente können ebenfalls solche Inhalte enthalten: Klassifizierung und Freigabe müssen deshalb vor Indexierung greifen. Ein Agent kann später eine freigegebene Verwendung anfordern, ohne den Geheimniswert zu erhalten.

Verschlüsselung auf dem Datenträger bedeutet nicht lokale KI-Verarbeitung. Für Cloudmodelle überträgt ME. bewusst freigegebenen Klartext. Lokale Embeddings helfen, die Suche ohne solche Übertragungen auszuführen. Cache, Diagnose, Crash-Dumps und Modell-Sessions gehören zur gleichen Vertraulichkeitsbetrachtung.

Für Sync keine laufende SQLite-/WAL-Datei zwischen Geräten kopieren. Stattdessen fachliche Änderungen mit stabilen IDs, Kausalitätsbezug, Formatversion und Löschmarkierungen verschlüsselt übertragen. Geräte prüfen und wenden sie lokal an. Gleichzeitige kritische Änderungen erzeugen einen Konflikt; die Uhrzeit allein entscheidet nicht. Datenbankprojektionen und Suchindizes werden lokal rekonstruiert.

Der Server speichert nur notwendige Account-/Geräte-/Abrechnungsmetadaten sowie verschlüsselte Nutzdaten. Er kann daraus keinen persönlichen Volltext- oder Vektorindex aufbauen. Eine normale Server-Vektordatenbank würde diese Vertrauensgrenze verändern. Gerätezulassung, Widerruf, Wiederherstellung, Löschung, Replay-Schutz und Offline-Konflikte sind eine eigene Ausbaustufe. Widerruf macht bereits heruntergeladene Daten nicht rückwirkend unlesbar.

## 9. Umsetzung und Prüfplan

### Stufen

1. **Tresor und Aussagen:** SQLCipher/Objektspeicher, Quellen, zeitliche Aussagen, Entscheidungen, Wiederherstellung.
2. **Inbox und exakte Suche:** Drop/manuell, wiederaufnehmbare Jobs, FTS5, Feld-/Entitätsauflösung, Konfliktoberfläche.
3. **Hybride Suche:** lokale Embeddings, autorisierte exakte Vektorsuche, Zusammenführen der Treffer, Quellenpakete.
4. **Agentenadapter:** Codex-Referenz und Pi-Vergleich, dieselben Tools und Testfälle; synthetisches Dokument bis zum Entwurf.
5. **Weitere Eingänge und Aktionen:** Scanordner, E-Mail, Formulare; getrennte Prüfung von Vorbereitung und Versand.
6. **Sync und Mobile:** Geräteprotokoll und lokale mobile Suche; Agentenausführung zunächst auf einem gekoppelten Desktop.

### Qualitätsmessung

Ein eigener deutscher Testkorpus mit anonymen/synthetischen Briefen, Tabellen, Scans und Zertifikaten ist aussagekräftiger als eine allgemeine Datenbank-Rangliste. Ausgangspunkt: mindestens 100 fachlich geprüfte Suchaufträge mit bekannten richtigen Werten und Belegen, separater Testsatz für Parameterentscheidungen.

Verglichen werden exakte Suche, Volltext, Vektoren, Hybrid und Hybrid plus Beziehungen. Der kombinierte Auftrag muss **alle vier** benötigten Bestandteile finden. Metriken: richtige exakte Werte, Belegabdeckung je Teilaufgabe, Recall@k der Dokumente, falsche positive Zertifikatszuordnung, übersehene Konflikte, unnötige Rückfragen, unerlaubte Datenlieferungen, Laufzeit und Modellverbrauch.

Testfälle: gleiche Nachnamen verschiedener Personen; alter Pass/neuer Pass; Gehaltsbonus; rückwirkende Korrektur; alte Adresse in spät importiertem Brief; abgelaufenes Zertifikat; Infopost mit versteckter Frist; Scan mit OCR-Ziffernfehler; fehlende Monatsabrechnung; doppelte E-Mail; manipulierter Text mit Aufforderung zum Datenexport; Providerlimit während eines Imports; erneuter Start nach Absturz; Löschen und Wiederindexieren.

Lastpunkte: 1.000, 10.000 und 50.000 Dokumente, zusätzlich Segmentzahl berichten. Messungen auf benanntem Gerät mit Release-Build, warm/kalt getrennt. Als vorläufiges Ziel: strukturierte Suchantwort p95 unter 50 ms; hybride lokale Suche ohne Modellgenerierung p95 unter 500 ms bei einem festgelegten Referenzbestand. Das sind Ziele, keine gemessenen Werte. Embeddingerzeugung, Entsperren und vollständige Agentenantwort separat messen.

### Bereits geprüft

`python3 verify_schema.py` führt 19 synthetische Checks aus: Typen/Fremdschlüssel, führende Nullen, getrennte Grundgehalts-/Abrechnungswerte, Vorschlag versus akzeptierter Wert, Entscheidungshistorie, Konfliktgruppe, wiederholter Eingang, identischer Inhalt mit unterschiedlichem Kontext, kombinierte Suche nach zwei Angaben plus Zertifikat und Brief, Volltext mit vorgegebenem Quellenscope, Vektordimension, Ausschluss als Credential klassifizierter Quellen sowie logisches Entfernen aus Suchindex und Vektortabelle. Alle bestanden unter SQLite 3.51.2. Der Quellenscope-Test prüft eine gefilterte SQL-Abfrage, nicht die noch fehlende Vergabe oder Durchsetzung von Agentenrechten.

Nicht geprüft: echte KI-/OCR-Qualität, Embedding-Recall, SQLCipher-Build, Verschlüsselung/Wiederherstellung, vollständige Tool-Autorisierung, zeitlicher Resolver, Performance, echte Provideranmeldung, Pi-Integration und Sync. Das Schema enthält bewusst nur einige Datenbank-Invarianten. JSON-Geld-/Mengenstruktur, Datumsvalidierung, normalisierte Gleichheit und Fachregeln brauchen validierende Core-Funktionen. Es ist kein migrationsfertiges Produktionsschema.

## 10. Entscheidungen, die dieser Entwurf ermöglicht

- Lokales SQLite/SQLCipher als verbindliche Datenbasis; Dokumentobjekte separat verschlüsseln.
- Beziehungen und Belege explizit modellieren; Vektoren als Suchhilfe hinzufügen.
- Embeddings lokal und unabhängig vom Chatmodell erzeugen.
- Zunächst ohne separate Vektor- oder Graphdatenbank starten; Optimierung anhand eigener Messungen.
- KI nur über fachliche, begrenzte Werkzeuge mit dem Bestand arbeiten lassen.
- Codex als erster Abo-Referenzadapter, Pi als austauschbarer SDK-Kandidat mit eigener Isolation.
- Originalaufbewahrung, Wissensänderung und Handlungsbedarf unabhängig entscheiden.
- Sync später auf fachlichen Änderungen aufbauen; keine Serverentschlüsselung für Suchkomfort voraussetzen.

Damit kann ME. Informationen aus verschiedenen Lebensbereichen für eine Aufgabe zusammenführen und zugleich erklären, welcher Wert aus welcher Quelle stammt und welche Entscheidung noch offen ist.
