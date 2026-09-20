# Import pipeline update — September 2026

The current flow is documented in [Import pipeline](import-pipeline.md) and
[TypeSafe, recovery and spending controls](import-reliability.md). Files require
confirmation. Imports shows overall and per-stage bars, recoverable steps and
provider errors. Two workers run concurrently. Pipeline `document-v5` uses local
parsing, TypeSafe decisions and small OpenAI extraction sections; no recursive
splitting or automatic inference retries. Intermediate results and call allowances
survive restarts. Context is local guidance, not web research. The historical
serial-flow and retry descriptions below are superseded by this update.

# Lokale Dokumentverarbeitung

## Nutzung

**Automatisch mit KI auswerten** ist in **Einstellungen** standardmäßig aktiviert.
Unterstützte neue und noch ausstehende Dateien werden nacheinander verarbeitet,
solange ME. geöffnet, Codex bereit und der Tresor entsperrt ist. Bereits importierte
Dateien müssen nicht erneut importiert werden. Erst lokal auslesen, dann den
erkannten Text über das ChatGPT-Abo an OpenAI senden; Werte bleiben Vorschläge.

Die Einstellung wird verschlüsselt pro Tresor gespeichert und bleibt nach Neustart
erhalten. Ausschalten stoppt laufende automatische Auswertung und hält wartende
Dateien für die manuelle Freigabe zurück. Bereits übermittelter Text lässt sich
damit nicht zurückholen. Bei ausgeschalteter Automatik: Dokument öffnen und
**Inhalt freigeben und mit KI auswerten** wählen. **Keine Zugangsdaten · Inhaltsuche
freigeben** liest ausschließlich lokal; eingeschaltete Automatik kann danach
weiterhin die KI starten.

Die separate Codex-Anmeldung erfolgt verpflichtend vor der normalen App-Nutzung
in der [Codex-Einrichtung](codex-integration.md#verbindliche-codex-einrichtung).
Originale, Einstellung, Warteschlange, Fehlermeldungen und Seitenbelege bleiben
verschlüsselt in diesem Tresor. Explizit als Zugangsdaten klassifizierte Quellen
sind ausgeschlossen. Automatisch ausgewertete Quellen erhalten dieselbe persönliche
Quellenklassifizierung wie manuell freigegebene Dokumente; eine anschließend
aktivierte Codex-Lesefreigabe kann sie lesen.

## Formate

| Format | macOS | Linux |
| --- | --- | --- |
| PDF, einschließlich Scans und gemischter Seiten | PDFKit + Vision | Poppler + Tesseract |
| JPG/JPEG, PNG, TIFF/TIF, BMP | Vision, EXIF-Ausrichtung berücksichtigt | Tesseract |
| HEIC/HEIF, WebP, GIF | ImageIO + Vision | Zunächst als JPEG/PNG/PDF exportieren |
| DOCX, ODT | Lokaler ZIP/XML-Parser | Derselbe Parser |
| RTF | Nativer Textimport | UnRTF |
| TXT, MD, CSV, TSV, LOG | UTF-8-Text | UTF-8-Text |
| DOC, XLS/XLSX/ODS, PPT/PPTX/ODP | Verschlüsselte Ablage; für Auswertung konvertieren | Ebenso |

PDFs werden seitenweise verarbeitet. Auch Seiten mit vorhandenem Text werden
gerastert und erkannt, damit eingebettete Scans nicht übersehen werden. OCR-Zeilen,
die schon im Text vorkommen, werden anhand eines Vergleichs ohne Großschreibung
und Leerraum unterdrückt. Abweichende OCR-Lesarten bleiben erkennbar als OCR-Belege;
sie können fehlerhaft sein und müssen vor Übernahme geprüft werden.

DOCX liest Haupttext, Tabellenzellen, Kopf-/Fußzeilen und Fuß-/Endnoten. ODT liest
Textabsätze und Tabelleninhalte. Office-Dokumente bekommen Abschnittsbelege, keine
erfundenen Seitenzahlen. Enthaltene Bilder, Diagramme, eingebettete Dateien und
Layoutinterpretation sind bei Office-Dateien noch nicht Teil der Auswertung.
Makros, externe Verweise und XML-DTDs werden nicht ausgeführt bzw. aufgelöst.

## Grenzen und Fehler

- Originale: höchstens 64 MiB; extrahierter Dokumenttext: höchstens 8 MiB.
  Reine Textdateien bleiben auf 1 MiB begrenzt.
- PDFs/Bildserien: höchstens 500 Seiten/Bilder. macOS verkleinert OCR-Bilder auf
  maximal 3200 Pixel Kantenlänge; Bilder über 100 Megapixel werden abgewiesen.
- Office: höchstens 4096 ZIP-Einträge, 100 ausgewählte Textbestandteile,
  8 MiB pro XML-Datei und 16 MiB insgesamt. Keine entpackten Dateien auf dem Datenträger.
- **KI:** bis zu 8 MiB erkannten Text in geplanten Abschnitten mit üblicherweise
  höchstens 3.200 Textbytes verarbeiten; Grenzen liegen zwischen unveränderten
  Quellsegmenten. Ein überlappendes Segment erhält benachbarten Kontext.
  Jeder OpenAI-Aufruf verwendet einen eigenen flüchtigen Thread. Ab 96 Vorschlägen
  stoppt der Abschnitt mit sichtbarem Fehler; es gibt keine rekursiven Teilungen.
  Pro Datei gelten persistente Aufruf- und Tokenbudgets. Große Dateien können vor
  Abschluss eine ausdrückliche Erweiterung benötigen.
  Insgesamt höchstens 1024 unterschiedliche Vorschläge. Geprüfte Abschnitte werden
  verschlüsselt zwischengespeichert und bei Wiederholung wiederverwendet.
  Transportfehler oder Abbruch in einem späteren Abschnitt ergeben keinen als
  vollständig dargestellten Teilerfolg; fertige Abschnitte bleiben für Wiederaufnahme erhalten.
  TypeSafe prüft Dokumentart, Lesbarkeit und Prüfbedarf. GPT-5.6 Sol mit mittlerem
  Reasoning extrahiert belegte Angaben; bei Bedarf folgt genau ein Audit.
  Die Felder sind offen: unter anderem Personalnummer, Name, Anschrift, Arbeitgeber,
  Eintritt, Zeiträume, Brutto-/Nettowerte, Abzüge, Bank, IBAN und BIC.
- Passwortgeschützte PDFs benötigen zunächst eine entsperrte Kopie. Beschädigte
  Dateien, leere Erkennung, fehlende Werkzeuge und Limits liefern sichtbare Fehler.
- Abbruch und Tresorsperre beenden den laufenden Prozess. Abgebrochene oder
  fehlgeschlagene Auswertungen zeigen einen Fehler an der Datei und lassen sich
  manuell erneut starten. Die Warteschlange fährt mit der nächsten Datei fort;
  dieselbe fehlerhafte Datei wird nicht automatisch endlos wiederholt.
- Nach einem Prozessabsturz bleiben laufende Aufträge unterbrochen; die gespeicherten
  Schritte lassen sich ausdrücklich fortsetzen. Kontingent-, Ratenlimit- und
  Authentifizierungsfehler pausieren zusätzlich die Warteschlange.

## Aufbau

`me-core` verwaltet Freigabe, verschlüsselte Originale, Versuchszähler, atomare
Segmentübernahme und Fundstellen. Die Originalbytes werden auf einem Worker
entschlüsselt, dann wird der Tresor-Mutex vor PDF/OCR/Office-Arbeit freigegeben.
Ein Ergebnis darf nur den noch aktiven Versuch abschließen. Fehlgeschlagene
Verarbeitung erzeugt keine halben Quellsegmente und keine bestätigten Angaben.

`me-documents` enthält lokale Parser und die Prozesssteuerung mit begrenzten Pipes,
Abbruch und 240 Sekunden ohne bestätigten Seitenfortschritt. Ein fortschreitender
macOS-Aufruf darf insgesamt bis zu einer Stunde laufen. Unter Linux sind Rendering
und OCR pro Seite getrennte begrenzte Werkzeugaufrufe. Auf macOS kompiliert
`build.rs` den Swift-Helfer mit PDFKit, Vision und ImageIO und bettet ihn in das
Rust-Binary ein. Das funktioniert mit `cargo run` und dem bestehenden App-Bundle.
Zur Laufzeit wird nur der ausführbare Helfer in einem privaten temporären Ordner
bereitgestellt. Originale und erkannter Text laufen durch Speicher/Pipes; sie werden
nicht als temporäre Dokumentdateien angelegt. OS-/Framework-internes Caching sowie
vollständige RAM-/Swap-Bereinigung werden damit nicht garantiert.

Linux benötigt Werkzeuge in `/usr/bin`. Beispiel für Debian/Ubuntu:

```sh
sudo apt install poppler-utils tesseract-ocr tesseract-ocr-deu tesseract-ocr-eng unrtf
```

Es gibt keine automatische Installation und keinen Cloud-OCR-Fallback. macOS benötigt
zur Entwicklung Xcode Command Line Tools; Nutzer des Bundles brauchen weder Swift
noch Poppler/Tesseract zu installieren.

## Verifikation

Synthetische Fixtures prüfen Text-PDF, Scan-PDF, gemischte Seiten, passwortgeschützte
und übergroße PDFs, JPG/PNG/TIFF/BMP/GIF/HEIC/WebP, EXIF-Drehung und RTF. ZIP/XML-Tests
prüfen DOCX/ODT, geteilte Textruns, Unicode, Zeichenreferenzen und Größenlimits.
Der echte Scan durchläuft Originalimport, lokale OCR, verschlüsselten Suchindex,
Seitenbeleg und Erstellung des Inbox-Modellinputs. Core-Tests prüfen Review,
Neustart, verspätete Ergebnisse und Zugangsdaten-Ausschluss. Prozess-Tests prüfen
Abbruch und Überlauf ohne Teilerfolg. Es werden ausschließlich synthetische Daten benutzt.

Linux-Code wird auch in den macOS-Tests kompiliert; eine Ausführung mit den echten
Linux-Werkzeugen und ein vollständiger Linux-Desktop-Build stehen noch aus.
Format-Tests rufen kein Modell auf. Der gesonderte Live-Test `inbox_live` prüft
synthetisches Text-PDF, Scan-PDF, JPEG sowie ein 60-seitiges PDF über die echte Codex-CLI bis zu belegten,
unbestätigten Vorschlägen. Er erfordert einen explizit angegebenen Testzugang
(`ME_CODEX_TEST_HOME`) und mehrere Sol-Aufrufe. Der 60-Seiten-Test prüft zusätzlich Cache-Wiederverwendung.

Primärquellen: [Apple Vision](https://developer.apple.com/documentation/vision/vnrecognizetextrequest),
[PDFKit-Seitenrendering](https://developer.apple.com/documentation/pdfkit/pdfpage/thumbnail(of:for:)),
[Tesseract-Aufruf](https://github.com/tesseract-ocr/tesseract/blob/main/doc/tesseract.1.asc),
[GNU UnRTF](https://www.gnu.org/software/unrtf/).

## Robuste Belegprüfung und Wiederaufnahme

Jeder KI-Abschnitt wird direkt geprüft. Unterschiedliche Leerzeichen, Zeilenumbrüche,
gruppierte Nummern und deutsche/ISO-Datumsdarstellung werden deterministisch mit dem
Original abgeglichen. Gespeichert werden ausschließlich wiedergefundene Originalzitate
und Rohwerte. Es gibt keine unscharfe Korrektur von Ziffern, Namen oder Buchstaben.
Die abschließende Tresorprüfung kontrolliert erneut die unveränderten Quellen.

Nicht belegte Vorschläge lösen einen zweiten, gezielten Beleg-Aufruf für diesen
Abschnitt aus. Bleiben einzelne Angaben unbelegt, werden sie ausgeschlossen;
belegte Vorschläge erscheinen mit **Auswertung mit Hinweisen**. Der Hinweis bleibt
verschlüsselt gespeichert. Der Nutzer kann die belegten Angaben prüfen und die
betroffenen Abschnitte später manuell erneut auswerten. Diese Abschnitte werden
nicht als vollständig geprüfter Cache abgelegt.

Vorübergehende Verbindungs-/Serverfehler bekommen einen begrenzten zweiten Versuch.
Anmeldung, Kontingent, verbotene Tool-Aufrufe und dauerhafte Anfragefehler werden
nicht endlos wiederholt. Cache-Schlüssel berücksichtigen Pipeline/Modell, Quelle,
Segment-IDs und Text; eine neue Lauf-ID verhindert die Wiederaufnahme nicht.
Alte oder gesperrte Läufe dürfen keine Zwischenergebnisse nachträglich speichern.

## Rückfragen zu unsicheren Angaben

Nach dem automatischen Korrekturversuch bleiben unbelegte Kandidaten als einzelne
Rückfragen erhalten. Die Oberfläche zeigt Wert, Unsicherheitsgrund, die von der
KI vermutete Person und ihren ausdrücklich als ungeprüft bezeichneten Wortlaut.
Eine zuordenbare Fundstelle zeigt zusätzlich echten lokal gelesenen Dokumenttext.
Der Nutzer kann jeden Wert bestätigen, korrigieren oder verwerfen. Bis dahin sind
diese Kandidaten keine Profilangaben und keine Antworten der MCP-Faktenabfrage.

Bestätigung und Korrektur speichern den Wert als ausdrückliche Nutzerangabe mit
eigener Quelle. Das PDF bleibt als Kontext verknüpft; ein ungeprüftes KI-Zitat wird
nicht nachträglich zu einem Quellenbeleg erklärt. Andere bestätigte Werte bleiben
bei Abweichungen als Konflikt erhalten. Rückfragen und Entscheidungen liegen nur
im verschlüsselten Tresor, überstehen Neustarts und werden bei Wiederholungen
anhand von Feld, Wert und zugeordneter Person zusammengeführt.

Migration 5 erhält bestehende Dokumente und Einstellungen. Frühere Versionen
speicherten nur die Zahl ausgelassener Kandidaten. Für diese Dokumente erklärt der
Hinweis, dass eine einmalige erneute Auswertung nötig ist; kein erneuter Import.

## Vollständige Dokumentanalyse (document-v3)

Die Feldbegrenzung auf drei Identitätsmerkmale wurde entfernt. Nur die drei
bisherigen Profilfelder behalten ihre bestehenden Typen; andere Angaben werden
als offen benannte Dokumentfelder gespeichert. Beträge behalten Druckformat,
Vorzeichen, Dezimalkomma und belegte Währung. Es erfolgt keine stillschweigende
Währungsannahme, Berechnung, Hochrechnung oder Ergänzung fehlender Werte.

Pro Textabschnitt laufen drei getrennte Modellaufrufe: Dokumentart/Land/Sprache
und Tabellenstruktur erkennen, alle relevanten Angaben auslesen, anschließend
Quelle und Feldinventar erneut nach fehlenden Angaben durchsuchen. Die zweite
Extraktion ergänzt belegte fehlende Fakten. Sie ersetzt keine Prüfung der
inhaltlichen Richtigkeit. Eine gescheiterte Vollständigkeitsprüfung verhindert
einen als abgeschlossen dargestellten Teilerfolg und das Speichern dieses Caches.
Die Belegkorrektur bleibt zusätzlich erhalten. Der Pipeline-Schlüssel wurde
geändert, damit Ergebnisse des alten Drei-Felder-Modells nicht wiederverwendet werden.

Die lokale Anleitung für deutsche Gehaltsabrechnungen berücksichtigt die
Unterschiede zwischen Gesamt-/Steuer-/SV-Brutto, Netto-Verdienst, Auszahlung,
Arbeitnehmer-/Arbeitgeberanteilen und kumulierten Jahreswerten. Sie verwendet
[DATEVs Erläuterung](https://www.datev.de/web/de/berufsgruppenuebergreifend/ratgeber/lohn-und-gehalt/gehaltsabrechnung-berechnen)
und die [DATEV-Musterabrechnung](https://www.datev.de/content/dam/markenassets/themen-und-produktgruppen/zielgruppen/zielgruppenuebergreifend/shop-assets/personalwirtschaft/lohn-und-gehalt-musterauswertung-2026-deutsch.pdf)
als Formatwissen. Das Modell recherchiert keine privaten Dokumentinhalte im Web.
Unbekannte Dokumentarten nutzen die allgemeine Feldinventur. Gedruckte Kurzdatumswerte
mit unklarem Jahrhundert bleiben als Rohwerte erhalten, ohne ein Jahrhundert zu erfinden.

Ein `context_quote` belegt Zeitraum/Konto/Abschnitt durch einen unveränderten
Wortlaut aus derselben Quelle. Dokumentfelder erhalten von vertrauenswürdigem
Core-Code einen Schlüssel aus Quelle, Feld, dokumentierter Person und Kontext;
ohne Kontext wird die Fundstelle als Abgrenzung verwendet. Dokumentname und
Kontext erscheinen in Vorschlag und gespeicherter Angabe. Dadurch verschmelzen
Beträge verschiedener Monate oder Dokumente nicht zu einem zeitlosen Profilwert.
Allgemeine Geldberechnungen und ein dokumentübergreifender zeitlicher Faktenresolver
sind damit noch nicht implementiert. Die vorhandene Dokumentensuche findet die
Originale; bestätigte Angaben sind in der Sammlung suchbar.

macOS-OCR ordnet erkannte Textblöcke nach ihren Koordinaten in Zeilen an und
behält nebeneinander stehende Beschriftungen und Werte zusammen. Der Modellinput
bleibt erkannter Text, keine Seitenbilder. Sehr schwierige Layouts und unleserliche
Scans können deshalb weiterhin Rückfragen oder fehlende Werte ergeben.

**Erneut gründlich auswerten** ist auch nach einer abgeschlossenen Auswertung
verfügbar. Es nutzt das gespeicherte Original bzw. dessen vorhandenen Text, verwirft
bei einer vollständig abgeschlossenen Auswertung den Abschnittscache und startet
die Analyse erneut. Vorhandene Bestätigungen und verworfene Vorschläge bleiben
erhalten. Die automatische Warteschlange wiederholt abgeschlossene Dateien nicht.
Bei unterbrochenen Läufen oder offenen Belegfragen bleiben geprüfte Abschnitte
für die Wiederaufnahme erhalten. Bereits vorhandener OCR-Text wird nicht neu indiziert.

Die Wahl von Sol wurde mit `model/list` über den tatsächlichen ME.-Zugang geprüft;
Astra wird dort derzeit nicht angeboten. Die Auswertung benötigt entsprechend mehr
Abo-Kontingent und mehrere Modellantworten. Verfügbarkeit und Reasoning richten sich
nach [Codex App Server](https://learn.chatgpt.com/docs/app-server#list-models-modellist).

Zusätzliche Regressionen: 30 vollständig synthetische Gehaltsfelder in Text-PDF,
Scan-PDF und JPEG, Zuordnung von Tabellenzeilen, beliebige Feldnamen, persistente
Bestätigung und Korrektur, zwei Monate mit demselben Brutto, negative Beträge,
führende Nullen, erfundene Perioden, Ausfall des Prüfaufrufs und Wiederanalyse.
Der Live-Test `payroll_pdf_scan_and_jpeg_extract_full_document` prüft alle 30
Druckwerte und mindestens 30 belegte Vorschläge bis zur Speicherung; er nutzt
keine persönlichen Dateien. Die Prüfungen sind Regressionen für diese Beispiele,
keine Zusage vollständiger Erkennung jeder realen Abrechnung.
