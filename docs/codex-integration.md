# Codex-Anbindung: erster nutzbarer Ausbau

**Update 18. September 2026:** Für Dokumentimporte gilt jetzt die
[TypeSafe-Pipeline mit Wiederaufnahme und Ausgabenbegrenzung](import-reliability.md).
TypeSafe trifft typisierte Routing-/Prüfentscheidungen mit einem privaten API-Key;
Sol extrahiert die freien Werte über das ChatGPT-Abo. Einzelne Schritte sind
verschlüsselt zwischengespeichert. Automatische Wiederholungen und rekursives
Aufteilen sind entfernt. Die fünf Fortschrittsbalken zeigen erledigte Arbeit.
Die folgenden historischen Angaben zu Klassifikation, Abschnittsgröße und
serieller Verarbeitung werden dadurch ersetzt.

Stand: 15. September 2026. ME. stellt Daten bereit; interaktive Aufgaben und
Computer Use bleiben in der Codex-App. Die Inbox hat einen separaten lokalen
Codex-App-Server-Prozess und nutzt das eigene ChatGPT-Abo.

## Codex liest ME.

1. App und Bridge bauen: `./scripts/cargo build -p me-app -p me-agent`.
   `./scripts/bundle-macos debug` enthält beide Programme.
2. Den lokalen STDIO-Server in Codex registrieren, beispielsweise:

   ```sh
   codex mcp add me -- /ABSOLUTER/PFAD/ME.app/Contents/MacOS/me-mcp
   ```

   Bei einem abweichenden Tresor `--vault-dir /ABSOLUTER/PFAD/vault` an das
   Bridge-Programm anhängen. Der Bridge-Prozess öffnet die Datenbank nicht.
3. ME. entsperren. Unten **Codex verbinden · Sicherung** öffnen und den
   **Codex-Lesezugriff für diese Sitzung freigeben**.
4. Eine neue Codex-Aufgabe öffnen und zum Beispiel fragen:
   „Lies meine Steuer-ID und mein Geburtsdatum aus ME. und nenne die Quellen.“

Die vier Werkzeuge sind `me_status`, `me_facts_get`, `me_documents_search` und
`me_evidence_read`. Die Bridge ist lesend. Sie hat keine Export-, Schreib-,
SQL-, Shell- oder Entsperrfunktion. `me_status` liefert die unterstützten
Feldschlüssel; Dokumenttreffer liefern IDs für den Belegabruf.

Die Freigabe umfasst einen Schnappschuss der aktuell vorhandenen Quellen mit
Klassifizierung `personal`: manuelle Angaben und ausdrücklich freigegebene
Dokumente. Unklassifizierte Quellen und Credentials sind ausgeschlossen.
Neue Quellen werden erst mit einer neuen Sitzungsfreigabe zugänglich. Die
Recherche meldet ihre Abdeckung als `shared_sources_only`; „fehlend“ bedeutet
nicht, dass der gesamte Tresor durchsucht wurde.

Die App besitzt Verbindung und Schlüssel. Ein privates Unix-Socket-Verzeichnis
und ein zufälliges Sitzungstoken verbinden sie mit `me-mcp`. Der Deskriptor
liegt mit Modus 0600 im privaten Tresorordner; er enthält keine Dokumentwerte
und keine Tresorschlüssel. Sperren widerruft den Zugang unmittelbar im Speicher,
anschließend werden Listener und Deskriptor auf einem Worker geschlossen.
Bereits übertragene Ergebnisse und Codex' andere Werkzeuge bleiben außerhalb
dieses Widerrufs. Programme mit kompromittiertem Zugriff auf denselben
Benutzeraccount sind keine durch diese Bridge isolierten Mandanten.

## Standardangaben

Neu angelegte Angaben mit exakt passenden Bezeichnungen (einschließlich der
implementierten Aliase) werden typisiert:

| Bezeichnung | Feldschlüssel | Format |
|---|---|---|
| Steuer-ID | `person.tax_id` | elf Ziffern, führende Nullen bleiben erhalten |
| Sozialversicherungsnummer / SV-Nummer | `person.social_insurance_number` | acht Ziffern, Buchstabe, drei Ziffern |
| Geburtsdatum | `person.birth_date` | Eingabe TT.MM.JJJJ oder JJJJ-MM-TT, Speicherung ISO-Datum |

Es handelt sich um Formatprüfung, nicht um eine Prüfung beim Aussteller.
Die Steuernummer ist absichtlich kein Alias der Steuer-ID. Alte freie Notizen
werden bei der Schema-2-Migration nicht stillschweigend umklassifiziert; sie
bleiben über die Dokumentensuche lesbar. Andere frei benannte Felder bleiben
möglich. Der erste exakte Faktenresolver deckt diese drei Standardfelder für
das lokale „Ich“-Profil ab; ein allgemeiner zeitlicher Resolver folgt.

Mehrere verschiedene bestätigte Werte oder ein abweichender offener Vorschlag
liefern `conflicting`, keinen willkürlich ausgewählten Wert. Vorschläge haben
unbestätigte Personenzuordnung. Nur ein eindeutiger akzeptierter Wert wird als
`resolved` zurückgegeben. Gleichlautende akzeptierte Angaben behalten Belege.

## Asynchrone Codex-Verbindung

ME. prüft die eigene Codex-Konfiguration im Hintergrund parallel zum App-Start.
Tresor-Erstellung, Wiederherstellung und Entsperren warten ausschließlich auf den
lokalen Tresor. Nach korrektem Passwort erscheint sofort der Arbeitsbereich;
Entsperren startet keine zweite Verbindungsprüfung. Auf dem Passwortbildschirm
erscheinen weder Verbindungsstatus noch Anmeldeaufforderungen.

Die Prüfung umfasst App-Server-Handshake, ChatGPT-Anmeldung mit Token-Refresh,
Sol-Verfügbarkeit und einen authentifizierten Abruf der Kontingentinformationen.
Sie startet keinen Modellturn und liest keinen Dokumentinhalt.

- Fehler oder ausgeschöpfte Kontingente erscheinen erst im geöffneten Arbeitsbereich
  als ausblendbarer Hinweis. Details, erneute Prüfung und explizite Anmeldung sind
  im Verbindungsdialog und über Einstellungen erreichbar. Escape oder „Back to
  workspace“ schließt den Dialog, ohne den Tresor zu sperren.
- Lokale Suche, Lesen, Bearbeiten, Importieren und Sicherungen bleiben verfügbar.
  KI-Suche, automatische Analyse und KI-Organisation warten auf die Verbindung.
  Ein erfolgreiches Prüfergebnis startet die aktivierte automatische Warteschlange.
- Kontingente werden aus dem Codex-Bucket (oder dem älteren Einzel-Bucket) gelesen.
  Ein bekanntes Fenster mit 100 Prozent Nutzung pausiert KI-Funktionen. Fehlende
  Angaben gelten nicht als ausgeschöpft. Die Prüfung verbraucht keine Credits und
  löst keinen Reset aus.
- Browser-Anmeldung startet nur nach einem ausdrücklichen Klick. Ein fehlerhafter
  oder abgebrochener Versuch beeinflusst den lokalen Tresorzugriff nicht.
- Ergebnisse, die vor dem Entsperren eintreffen, werden bis zum Arbeitsbereich
  zurückgehalten. Sperren verbirgt den Dialog; ein offener Browser-Login wird
  abgebrochen. Beenden stoppt die Prüfung und die eigene Codex-Prozessgruppe.
- Spätere KI-Aufrufe prüfen ihren Zugang erneut. Ein Verbindungsfehler zeigt den
  Hinweis im Arbeitsbereich; ME. kehrt nicht zu einer Einrichtungssperre zurück.
  Es gibt keinen laufenden Hintergrund-Verbindungsmonitor.

Die Codex-Lesefreigabe für ME.-Werkzeuge bleibt eine eigene bewusste Freigabe im
Tresor. Erfolgreiche Anmeldung aktiviert keinen MCP-Lesezugriff automatisch.

## Erste Inbox-Auswertung

1. PDF, Foto/Scan, DOCX, ODT, RTF oder UTF-8-Text importieren und das Dokument öffnen.
   [Formatübersicht und lokale Verarbeitung](document-processing.md).
2. Standardmäßig startet die automatische KI-Auswertung. Unter **Einstellungen**
   lässt sie sich dauerhaft ausschalten; dann im Dokument **Inhalt freigeben und
   mit KI auswerten** wählen. Die Oberfläche erklärt die Übermittlung des erkannten
   Texts an OpenAI. PDF-Text und OCR werden vorher lokal verarbeitet.
3. Der bereits eingerichtete Codex-Zugang verwendet das bestehende ChatGPT-Abo.
   Der getrennte Login wird von Codex unter `ME/codex-inbox` neben dem Tresor
   verwaltet. ME. kopiert keine Anmeldetokens aus der interaktiven Codex-App.
   Ein API-Key wird für diese Pipeline nicht verwendet.
4. GPT-5.6 Sol analysiert Dokumentart und Aufbau, extrahiert alle belegten
   Angaben und prüft gesondert auf Auslassungen, aus bis zu 8 MiB
   Dokumenttext, aufgeteilt in Abschnitte mit höchstens 12.000 Textbytes pro Aufruf.
   Der Fortschritt nennt den aktuellen Abschnitt. Erst wenn alle Aufrufe erfolgreich
   sind, prüft der Core die belegten Vorschläge erneut gemeinsam. Unbelegte Kandidaten
   werden nach einem Korrekturversuch als einzelne, verschlüsselt gespeicherte Rückfragen präsentiert. Vorschläge zeigen Wert, erkannten Quelltext, Seite/Abschnitt, Personenbeleg und
   bereits bestätigte Werte.
5. **Diese belegten Angaben gehören zu mir · bestätigen** oder **Vorschläge
   verwerfen** wählen. Eine Bestätigung erzeugt Aussagen mit Quellen und
   Entscheidungen. Abweichende Bestandswerte werden nicht überschrieben.

Der Core kontrolliert Standardfelder bzw. begrenzte freie Dokumentfeldnamen, exakte Zitate in
kanonischen Quellsegmenten und einen Personenbeleg. Diese Kontrollen beweisen
keine korrekte inhaltliche Interpretation; die Personenzuordnung bleibt eine
bewusste Nutzerentscheidung. Belegte Vorschläge lassen sich gemeinsam bestätigen. Unsichere Angaben fragen
einzeln: **Stimmt und gehört zu mir**, **Korrigieren** oder **Verwerfen / gehört
nicht zu mir**. Eine Antwort erzeugt einen manuell bestätigten Wert; der ungeprüfte
KI-Wortlaut wird niemals als Dokumentbeleg übernommen. Rückfragen bleiben bis zur
Antwort offen und erscheinen nach Wiederholungen nicht doppelt. Bereits
beantwortete Rückfragen werden nicht erneut geöffnet.

Ein fehlgeschlagener oder unterbrochener Aufruf hinterlässt einen erneut
startbaren Auftrag. Laufende Jobs werden beim nächsten Entsperren als
unterbrochen markiert. Auswertung abbrechen oder Tresor sperren beendet den
Modellprozess. Erledigte Dokumente werden nicht versehentlich doppelt ausgewertet.
Die verschlüsselte Warteschlange verarbeitet unterstützte Dateien nacheinander.
Automatik ist standardmäßig aktiv, einschließlich noch offener Bestandsimporte.
Ausschalten stoppt laufende automatische Verarbeitung und setzt wartende Dateien
auf manuelle Freigabe. Fehlgeschlagene Dateien werden nur auf expliziten erneuten
Start wiederholt; ein Prozessabsturz erlaubt Wiederaufnahme beim Entsperren.
Die Einstellung gilt pro Tresor und ist Bestandteil seiner Sicherung.

## Codex-Prozess und Grenzen

Geprüfte lokale Protokollversion: `codex-cli 0.146.0`. Die Schnittstelle bleibt
experimentell; bei CLI-Updates die Vertragstests und einen Live-Test wiederholen.
`ME_CODEX_BIN` kann einen absoluten Pfad zur Codex-Binärdatei vorgeben.

Bei einer offiziellen npm-Installation löst ME. den JavaScript-Starter auf die
installierte native Codex-Datei auf. Dadurch funktioniert der Start auch aus
Finder/Dock, wenn Node.js nicht im Desktop-`PATH` liegt. Unterstützt werden
verschachtelte und hoisted Plattformpakete sowie das ältere Vendor-Layout.
Fehlt die native Datei, zeigt die Einrichtung einen Installationshinweis.
Der Live-Test prüft Handshake, fehlende Anmeldung und Login-Start/-Abbruch mit
einem minimalen Desktop-`PATH`, temporärem Codex-Home und ohne Modellaufruf.

- Separates `CODEX_HOME`, unveränderte Konfiguration, keine übernommenen MCP-
  Server oder Hooks. Plugins, Remote-Plugins, Shell, Apps und weitere Werkzeuge
  sind ausdrücklich deaktiviert; Websuche ebenfalls. Ein von Codex selbst
  angelegter Plugin-Cache ist kein Konfigurationsfehler und wird nicht aktiviert.
- Temporäres Arbeitsverzeichnis und das benannte Berechtigungsprofil `me-inbox`:
  `:root` verweigert, minimale Plattformpfade und temporäres Arbeitsverzeichnis
  lesend, keine Schreibrechte oder Netzwerkrechte für Werkzeuge. `thread/start`
  und `turn/start` wählen dieses Profil über `permissions`; das entfernte Feld
  `readOnly.access` wird nicht mehr verwendet. Keine Vault-Datei oder Schlüssel
  im Modellkontext.
- Flüchtiger Thread, begrenzte JSON-Ausgabe, Konto- und Modellprüfung vor Inferenz.
  Kein API-Fallback und kein automatischer Modellwechsel. GPT-5.6 Sol mit
  explizit mittlerem Reasoning wird für die gründliche Auswertung verwendet.
- Antwort-, Ereignis- und Zeitlimits; unerwartete modellinitiierte Werkzeug-
  oder Freigabeanforderungen werden abgewiesen. Fehlertexte enthalten keine
  Provider-Rohantworten oder persönlichen Dokumentwerte.
- Runtime-SQLite-Zustand und Logs liegen im privaten temporären Arbeitsbereich,
  der nach dem Prozess entfernt wird. Nach einem Prozess-/Systemabsturz können
  temporäre Runtime-Dateien zurückbleiben. Eine vollständige Prüfung gegen
  Klartextpersistenz nach echter Inferenz und bei Abstürzen steht aus.
- Die separate Anmeldung nutzt weiterhin dasselbe Abo-Kontingent wie Codex.
  Direkte Fragen an ME.-Werkzeuge verursachen keine zusätzliche ME.-Inferenz.

Längere Dokumente werden abschnittsweise verarbeitet. Die Extraktion unterstützt
frei benannte Dokumentangaben neben den drei typisierten Profilfeldern.
[Analyse, Kontext und Grenzen](document-processing.md#vollständige-dokumentanalyse-document-v3).
Verbrauchsanzeige und ein installierbares Plugin-Paket sind weitere Ausbauschritte.

## Prüfungen

Verhaltenstests arbeiten mit synthetischen Daten: echte SQLCipher-Dateien,
Typen und Konflikte, Quellenfreigaben, echte Unix-Socket-Kommunikation,
MCP-Initialisierung/Toolaufrufe, falsches Token, Widerruf, belegte Vorschläge,
Annahme, Wiederaufnahme sowie ein simulierter App-Server mit gestreamten
Ereignissen. Die Einrichtung ist zusätzlich gegen fehlende, abgelaufene und
falsche Anmeldung, Login-ID-Verwechslung, Abbruch, unerlaubte Login-URLs,
Neustart, fehlendes Modell und Verbindungsfehler getestet.
Ein realer lokaler App-Server-Handshake und Login-Start/-Abbruch werden zusätzlich
mit Desktop-PATH ohne Inferenz geprüft. `tests/inbox_live.rs` prüft synthetisches
Text-PDF, Scan-PDF und JPEG bis zu belegten Vorschlägen mit der echten CLI und Sol.
Dieser separat gestartete Test benötigt `ME_CODEX_TEST_HOME` mit Anmeldung und
führt mehrere Modellaufrufe pro Dokument aus. Die regulären Tests bleiben offline.
Schema-1/2-Migration, Standardwert, dauerhaftes Opt-out, manuelle Freigabe,
Queue-Reihenfolge, Wiederaufnahme, Fehler und Ausschlüsse sind regressionsgetestet.

Primärquellen: [Codex MCP](https://learn.chatgpt.com/docs/extend/mcp),
[App Server](https://learn.chatgpt.com/docs/app-server),
[Konfiguration](https://learn.chatgpt.com/docs/config-file/config-reference),
[Berechtigungsprofile](https://learn.chatgpt.com/docs/permissions),
[MCP-STDIO](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports),
[MCP-Werkzeuge](https://modelcontextprotocol.io/specification/2025-06-18/server/tools).
