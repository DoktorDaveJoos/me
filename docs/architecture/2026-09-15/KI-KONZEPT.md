# ME. — KI für Inbox, Fragen und ausführende Aufgaben

Konzept vom 15. September 2026 · Modell- und Protokollstand an diesem Tag geprüft

## 1. Festlegung und Leitidee

**Für das MVP festgelegt:** ME. nutzt Codex App Server für die eigene Hintergrundverarbeitung mit dem bestehenden ChatGPT-/Codex-Abo. Interaktive Fragen und ausführende Aufgaben laufen zunächst in der vorhandenen Codex-App, die ME. über MCP verwendet. Andere Agentenruntimes und zusätzliche API-Abrechnung kommen später. Diese Festlegung ersetzt den früheren Vorschlag, bereits jetzt Pi und andere Adapter parallel zu vergleichen.

**Aufteilung nach der Rückmeldung zum Computer Use:** ME. wird der persönliche Datenspeicher und Werkzeuganbieter für Codex. Die Inbox erhält einen begrenzten eigenen KI-Ablauf. Die Codex-App übernimmt Gespräch, Agentensteuerung und ihre vorhandenen Browser-/Computer-Use-Funktionen. Eine eigene Chatoberfläche und ein eigener Computer-Executor sind keine Voraussetzung des MVP.

ME. verwaltet das Wissen: Werte, Personen, Quellen, Zeiträume, Entscheidungen und Aufgaben. Codex versteht Sprache, interpretiert Dokumente und plant Arbeitsschritte. Ein Gesprächsverlauf ersetzt keinen bestätigten Datenbestand.

Dieses Dokument beschreibt den nächsten Ausbau. Die hier beschriebenen Extraktoren, vollständige Faktenauflösung und MCP-Werkzeuge sind noch nicht implementiert. Die Modelle wurden anhand ihrer dokumentierten Eignung ausgewählt; ein Vergleich auf ME.-Dokumenten steht aus. Die Einbindung vorhandener Codex-Computerfunktionen in einen ME.-gestützten Ablauf ist ebenfalls noch zu testen.

## 2. Modellstrategie

| Arbeitsprofil | Startkandidat | Denkaufwand | Erwartetes Ergebnis |
|---|---|---|---|
| Klare Dokumente auswerten | `gpt-5.6-luna` | low | Strukturierte Vorschläge mit Fundstellen |
| Fragen in der Codex-App | Dort gewähltes Modell; Luna für klare Fragen, Terra für breitere Fragen | low / medium | Fakten- und Dokumentabrufe über ME.-Werkzeuge |
| Schwierige Dokumente / Fragen über mehrere Quellen | `gpt-5.6-terra` | medium | Belegte Auswertung oder gezielte Klärung |
| Komplexe Aufgaben mit Werkzeugen und Computer Use | `gpt-6-astra` | medium oder high | Plan, geprüfter Entwurf, nach Freigabe ausgeführte Schritte |

OpenAI beschreibt Luna als Kandidaten für Extraktion und Klassifikation, Terra für alltägliche Arbeit mit Werkzeugen und Astra für schwierige Abläufe über mehrere Schritte. Das begründet den Startvergleich, beweist aber keine Extraktionsqualität für deutsche persönliche Dokumente. Modellverfügbarkeit hängt vom Konto und Client ab; ME. fragt sie über `model/list` ab. [Modellübersicht](https://learn.chatgpt.com/docs/models)

ME. wählt das Modell seiner Inbox-Aufträge. In einer interaktiven Codex-Aufgabe gilt die Modellauswahl der Codex-App. Ein Aufruf von `me.facts_get` verwendet kein weiteres Modell: Der Core liefert gespeicherte Werte. Der MCP-Server kann nicht stillschweigend das Modell des aufrufenden Codex-Gesprächs umstellen.

Ein stärkeres Modell ist sinnvoll, wenn Verständnis oder Planung das Problem ist. Ein unleserlicher Scan braucht eine bessere Text-/Bildgrundlage; ein Widerspruch zwischen zwei amtlichen Briefen braucht gegebenenfalls eine Klärung. Mehr Denkaufwand darf fehlende Belege nicht ersetzen.

Für die Inbox zunächst Luna mit gezielter Terra-Eskalation konfigurieren; Astra als Empfehlung für anspruchsvolle Aufgaben in Codex. Ein zusätzlicher Sol-Zwischenschritt wird erst interessant, wenn Messungen einen konkreten Vorteil zeigen. Modell und Denkaufwand der ME.-Aufträge bleiben versionierte Konfiguration.

## 3. Inbox: aus Dokumenten überprüfbare Angaben gewinnen

### Was die KI hier tatsächlich macht

Der Eingang ist häufig **unstrukturierter oder nur teilweise strukturierter Inhalt**: Brief, PDF, Foto, Tabelle oder E-Mail. Die KI muss erkennen, was dort steht, wen es betrifft und auf welchen Zeitraum es sich bezieht. Das Ergebnis soll strukturiert sein.

Ein geeignetes kleines Modell kann diese Aufgabe übernehmen, wenn der Auftrag begrenzt ist: bekannte Feldbedeutungen, verständlicher Dokumenttext und überprüfbare Fundstellen. Es soll keine beliebige Zusammenfassung erzeugen, aus der ME. anschließend Zahlen herausparst.

### Verarbeitung

1. **Eingang sichern:** Original verschlüsseln und Eingangskennung speichern. Ein erneuter Zustellversuch erzeugt keinen zweiten Auftrag. Gleicher Dateiinhalt in einem anderen Vorgang bleibt als eigener Eingang nachvollziehbar.
2. **Inhalt vorbereiten:** PDF-Text möglichst lokal lesen; für Scans OCR ergänzen. Seiten, Tabellen und Textpositionen erhalten. Schlechte Lesbarkeit sichtbar markieren. Bildausschnitte nur bereitstellen, wenn der Text für die Aufgabe nicht reicht.
3. **Gezielt extrahieren:** Luna erhält die freigegebenen Abschnitte, relevante Felddefinitionen und ein verbindliches Ausgabeschema. Dokumenttyp, Personenzuordnung und Aussagen sind zunächst Vorschläge. Neue Feldbedeutungen werden separat vorgeschlagen, nicht frei in bestehende Felder gezwungen.
4. **Im Core prüfen:** Datentyp, Feldbedeutung, erlaubte Person, Belegstelle, Normalisierung und Zeitbezug kontrollieren. Vorhandene Angaben vergleichen. Geld wird mit Dezimalarithmetik verarbeitet; Kennnummern bleiben Zeichenfolgen mit führenden Nullen.
5. **Ergebnis zeigen:** Ein gebündelter Prüffall pro Dokument beziehungsweise zusammenhängender Änderung. Originalstelle, bisheriger Wert, Vorschlag und offene Fragen stehen nebeneinander.
6. **Entscheidung speichern:** Bestätigung oder Ablehnung mit Herkunft speichern. Erst akzeptierte Aussagen tragen zum bestätigten Wissen bei. Gleichwertige Aussagen können einen weiteren Beleg bekommen; widersprüchliche Angaben überschreiben sich nicht stillschweigend.

### Vertrag für einen Extraktionsvorschlag

Die Ausgabe enthält mindestens:

- eine vom Core vergebene Quellen- und Segmentreferenz;
- vorgeschlagene Person oder `unknown` mit passenden Belegen;
- kanonische Feldbedeutung oder einen gesonderten Vorschlag für ein neues Feld;
- den gelesenen Wert und gegebenenfalls eine vorgeschlagene Normalisierung;
- Zeitart und belegten Zeitraum; unbekannte Zeit bleibt unbekannt;
- konkrete Fundstelle, möglichst mit kurzem Originalwortlaut;
- maschinenlesbare Problemhinweise, etwa unleserlicher Text oder mehrere mögliche Personen.

Der Core prüft Referenzen und normalisiert selbst. Ein existierendes Zitat beweist noch nicht, dass die KI dessen Bedeutung richtig verstanden hat. Auch ein gültiges Format oder eine Prüfziffer beweist nicht, wem eine Nummer gehört. Die Selbsteinschätzung des Modells ist kein verlässlicher Annahmeschwellwert.

### Eskalation und Produktverhalten

| Befund | Nächster Schritt |
|---|---|
| Klare Aussage, passende Fundstelle, keine Konflikte | Als prüfbaren Vorschlag anzeigen |
| Ausgabe verletzt das Schema | Ein begrenzter Korrekturversuch; danach Verarbeitung als fehlgeschlagen markieren |
| Gute Lesbarkeit, aber schwierige Tabelle / Zuordnung | Relevanten Kontext erweitern und Terra einsetzen |
| Zeichen im Scan nicht verlässlich lesbar | Originalseite prüfen beziehungsweise bessere Vorlage anfordern |
| Andere Person oder zwei widersprüchliche Werte | Gemeinsamen Klärungsfall erzeugen |
| Im Dokument nicht vorhanden | Feld als fehlend behandeln |

Startmodus: neue persönliche Fakten gebündelt bestätigen lassen. Später dürfen ausgewählte, getestete Regeln risikoarme Ergebnisse automatisch annehmen. Identitätsdaten und kritische Änderungen bleiben konservativer behandelt. Der Nutzer wird nicht für jedes erkannte Wort separat unterbrochen.

Fristen, Handlungsvorschläge und Ablageinformationen sind eigene Ergebnisse. Aus „Bitte überweisen Sie …“ darf ein Aufgabenvorschlag entstehen; die Dokumentverarbeitung löst keine Überweisung aus.

## 4. Fragen: zuerst Bedarf verstehen, dann Wissen abfragen

### Beispiel: „Wie ist meine Steuer-ID, meine SV-Nummer und mein Geburtsdatum?“

1. Der Nutzer stellt die Frage in Codex und weist ME. als Datenquelle zu.
2. Codex erkennt die drei Feldbedeutungen und ruft das gemeinsame Faktenwerkzeug auf. „Meine“ wird auf das von ME. festgelegte Profil bezogen; die KI wählt keine Person anhand des zuletzt gelesenen Dokuments.
3. Der ME.-Core fragt diese Angaben gesammelt ab und liefert exakte Werte, Status, Zeitbezug und Quellenreferenzen.
4. Codex formuliert daraus die Antwort. Werte sollen unverändert übernommen und mit ihren Quellen verbunden werden. Die Originalangaben bleiben in ME. direkt überprüfbar und kopierbar.

ME. startet für diesen Werkzeugaufruf keinen zweiten Modelllauf. Für eine reine Anzeige oder Kopie bekannter Angaben innerhalb der ME.-Oberfläche ist überhaupt keine Inferenz notwendig. Im Codex-Gespräch gelangen die abgerufenen Werte dagegen in dessen Modellkontext; eine garantiert modellfreie Ausgabe lässt sich dort nicht behaupten.

### Zustände gehören in die Antwort

| Wissensstand | Darstellung |
|---|---|
| Bestätigter, eindeutiger Wert | Wert mit Beleg |
| Nur unbestätigter Fund | Als Vorschlag kennzeichnen, Bestätigung ermöglichen |
| Konflikt oder unklare Zuordnung | Unsicherheit und maßgebliche Belege zeigen |
| Fehlender Wert | In passenden Dokumenten suchen oder gezielt nachfragen |
| Für den gewünschten Zeitraum nicht belegbar | Zeitliche Einschränkung nennen |

Ein Suchtreffer wird durch seine Verwendung in einer Antwort nicht automatisch zu einer bestätigten Angabe. Bei einer Rückfrage muss ME. gegen den aktuellen Wissensstand auflösen; frühere Gesprächsantworten sind keine dauerhafte Quelle.

### Fragen über Dokumente

„Was hat sich an meiner Versicherung geändert?“ benötigt eher Terra: passende Dokumente nach Person, Vertrag und Zeitraum suchen, Abschnitte lesen, Änderungen mit Belegen erklären. Das Modell darf nur Aussagen als belegt darstellen, die die geladenen Stellen tragen.

Die erste Version verwendet strukturierte Abfragen und Volltextsuche mit Aliasen. Lokale semantische Suche ergänzt dies später, wenn der Suchvergleich ihren Nutzen zeigt. Ein separater Embedding-Dienst ist keine Voraussetzung für diese erste KI-Anbindung.

„Wie viel habe ich insgesamt bezahlt?“ benötigt eine vollständige, definierte Ergebnismenge und eine Berechnung im Core. Eine Handvoll ähnlich klingender Suchtreffer genügt dafür nicht. ME. muss auch kenntlich machen, wenn Dokumente oder Monate fehlen.

## 5. Agentische Aufgaben und Computer Use

### Beispiel: „Mach meine Steuererklärung“

Für Planung, Dokumentabgleich, Rückfragen und die Navigation durch einen längeren Ablauf ist Astra ein sinnvoller Startkandidat. Seine Leistung wird an erfolgreich abgeschlossenen, überprüften Arbeitsschritten bewertet.

Die Aufgabe wird in konkrete Ergebnisse zerlegt:

1. Jahr, betroffene Person und Umfang klären, soweit nicht aus dem Auftrag bekannt.
2. Benötigte Unterlagen und Angaben auflisten; vorhandene Quellen zuordnen und Lücken sichtbar machen.
3. Zahlen und Zeiträume im Core zusammenführen; fachliche Berechnungen beziehungsweise Regeln über geeignete geprüfte Funktionen oder die verwendete Fachsoftware beziehen.
4. Einen nachvollziehbaren Entwurf erstellen und Felder mit Herkunft verknüpfen.
5. Den Entwurf in der Zielanwendung vorbereiten. Fehler, offene Rückfragen und fehlende Nachweise festhalten.
6. Vor einer verbindlichen Einreichung den konkreten Stand mit Ziel und Anhängen zur Freigabe zeigen.
7. Nach Ausführung das tatsächliche Ergebnis oder die Empfangsbestätigung prüfen und speichern. Ein Klick allein gilt nicht als Erfolg.

Die Rolle des Modells ist Interpretation und Ablaufsteuerung. Rechnen, Quellenauflösung, Berechtigungen und Zustandsänderungen benötigen überprüfbare Funktionen. Für eine tatsächliche Steuerfunktion kommen aktuelle fachliche Regeln und eigene fachliche Prüfungen hinzu; dieses Konzept spezifiziert noch keine Steuerlogik.

### Ausführung in der vorhandenen Codex-App

Die Codex-App übernimmt den Ablauf mit ihren verfügbaren Werkzeugen. Für ME.-Daten verwendet sie die strukturierte MCP-Anbindung; für andere Anwendungen vorhandene Integrationen und bei Bedarf Computer Use. Das MVP baut keinen eigenen Browser-/Desktop-Executor.

Die offizielle Dokumentation bestätigt Computer Use in der Desktop-App auf macOS und Windows in unterstützten Regionen, nach Einrichtung und Erteilung der erforderlichen Rechte. Die ME.-App bleibt auf macOS/Linux ausgerichtet; dieser konkrete Computer-Use-Ablauf wird zunächst auf dem Mac nachgewiesen und ist kein Linux-Funktionsversprechen. [Computer Use in Codex](https://learn.chatgpt.com/docs/computer-use)

Das Zielbild lautet: „Bereite mit meinen Unterlagen aus ME. meine Steuererklärung vor.“ Codex liest bestätigte Fakten und passende Quellen über ME., stellt Rückfragen und bedient die Zielanwendung mit vorhandenen Werkzeugen. Entwürfe und später Nachweise können über ausdrücklich freigegebene ME.-Werkzeuge zurückgegeben werden.

Die Verantwortlichkeiten bleiben sichtbar:

- **ME. kontrolliert seine Datenzugriffe und Änderungen:** erlaubte Datenbereiche, entsperrter Tresor, Quellenstatus, Vorschläge und bestätigte Entscheidungen.
- **Codex kontrolliert seine Ausführungsumgebung:** Zugang zu Apps und Webseiten, Werkzeugfreigaben, Gesprächsverlauf und externe Aktionen.
- **Verbindliche Einreichungen** müssen im Codex-Ablauf am konkreten Entwurf geprüft und freigegeben werden. ME. kann eine über unabhängige Codex-Werkzeuge erfolgende Einreichung technisch nicht allein über seinen MCP-Server sperren.

Eine ME.-Sperre beendet weitere ME.-Abrufe und ausstehende eigene Schreiboperationen. Sie entfernt keine bereits übertragenen Werte aus Codex und stoppt nicht automatisch dessen andere Werkzeuge. Diese Grenze darf die Oberfläche nicht als umfassenden Widerruf darstellen.

Der erste gemeinsame Test ist ein überschaubares Formular mit synthetischen Daten: Angaben aus ME. lesen, über Codex eintragen und den tatsächlichen Inhalt prüfen. Dateiübergabe und Uploads werden gesondert erprobt: Der eingebaute Browser unterstützt laut Dokumentation derzeit keine automatisierten Datei-Uploads. Ein vollständiger Steuerablauf wird deshalb nicht allein durch eine funktionierende MCP-Verbindung versprochen. [Codex-Browser](https://learn.chatgpt.com/docs/browser)

## 6. Gemeinsame technische Grundlage

```text
ME.-Inbox → lokaler Codex App Server → Luna / bei Bedarf Terra
    ↓ geprüfte Vorschläge
ME.-Core: verschlüsselter Tresor, Fakten, Quellen, Entscheidungen
    ↕ begrenzte Werkzeuge über lokalen MCP-Server
Codex-App: Gespräch, gewähltes Modell und Aufgabensteuerung
    ↕ vorhandene Integrationen, Browser und Computer Use
Zielanwendungen
```

Der Core bleibt unabhängig von GPUI. Netzwerk, Dokumentverarbeitung und Inferenz laufen außerhalb des UI-Threads. Die Oberfläche erhält Fortschritt, Ergebnisse und Klärungsbedarf aus nachvollziehbaren Auftragszuständen.

### Werkzeugumfang nach Profil

| Profil | Daten und Werkzeuge |
|---|---|
| Inbox | Zugewiesene Quelle und Felddefinitionen; strukturierte Ausgabe wird vom Core als Vorschlag verarbeitet |
| Fragen | `schema.discover`, `entities.resolve`, `facts.get`, `documents.search`, `evidence.read`; bei Bedarf begrenzte Beziehungen und Aggregate |
| Aufgaben in Codex | ME. liefert Fakten und Dokumente; später zusätzlich `tasks.prepare` und Rückgabe von Ergebnissen. Externe Ausführungswerkzeuge gehören zur Codex-App. |

Die Namen sind fachliche Schnittstellen aus der bestehenden Architektur. Eine Datenbankdatei, ein Datenbankschlüssel, allgemeines SQL oder ein beliebiger Dateiexport gehören nicht zum Werkzeugvertrag.

### ME. als lokaler MCP-Server

Die Codex-App unterstützt lokale STDIO-MCP-Server und teilt deren Konfiguration mit anderen lokalen Codex-Clients auf demselben Host. Ein solcher Server reicht für den ersten Integrationsnachweis. [MCP in Codex](https://learn.chatgpt.com/docs/extend/mcp)

Vorgeschlagener Aufbau: Codex startet einen kleinen `me-mcp`-Prozess. Dieser vermittelt über authentisierte lokale Kommunikation zum entsperrten ME.-Core. Der Bridge-Prozess erhält keinen Tresorschlüssel und öffnet keine zweite unkoordinierte Datenbankinstanz. ME. bleibt Eigentümer von Entsperren, Schreibtransaktionen und Datenfreigaben. Eine bloße Socket-Datei ohne Zugriffskontrolle reicht als Identitätsprüfung nicht.

Der erste Werkzeugumfang kann klein bleiben:

| Vorgeschlagener MCP-Name | Zweck |
|---|---|
| `me_status` | Verbindung, Sperrzustand und freigegebenen Umfang erkennen |
| `me_facts_get` | Bestätigte Angaben inklusive Konflikten und Quellen lesen |
| `me_documents_search` | Erlaubte Dokumente finden |
| `me_evidence_read` | Maßgebliche Textstellen lesen |
| später `me_changes_propose` | Neue Angaben oder Korrekturen zur Prüfung zurückgeben |
| später `me_tasks_prepare` | Einen versionierten Entwurf speichern |

Diese Namen sind Entwurf, keine bereits registrierten Werkzeuge. Eine in ME. erteilte Freigabe bestimmt den Datenumfang. Vom Modell mitgeschickte Zweck- oder Aufgabenkennungen erweitern ihn nicht. Nach der einmaligen Verbindung brauchen erlaubte lesende Zugriffe im entsperrten Zustand keine Bestätigung pro Feld; Änderungen laufen durch das Quellen-/Entscheidungsmodell.

Ein ME.-Plugin kann anschließend den Server und einen Skill für den Umgang mit Personen, Quellen, Widersprüchen und Entwürfen bündeln. Der Skill beschreibt korrektes Arbeiten; Zugriffsschutz bleibt im Core. Plugins können laut Dokumentation MCP-Server und Skills gemeinsam ausliefern. [Plugins](https://learn.chatgpt.com/docs/plugins)

### App-Server-Anbindung für die Inbox

- Lokaler Prozess über stdio; initialisieren, Konto prüfen und verfügbare Modelle abfragen.
- Modell und Denkaufwand explizit je Arbeitsprofil setzen. Wechsel erfolgen an Aufrufgrenzen; ein laufender Versuch wird nicht durch einen unbemerkten Wechsel fortgeführt.
- Inbox-Ausgaben über `turn/start.outputSchema` begrenzen und anschließend unabhängig im Core prüfen.
- Für den ersten Inbox-Lauf genügt die zugewiesene Dokumentgrundlage mit strukturierter Ausgabe. Wenn später Werkzeuge nötig sind, denselben fachlichen Zugriffsdienst verwenden und den erlaubten Umfang auf diesen Eingang begrenzen.
- Streaming, Abschluss, Fehler und Unterbrechung als technische Ereignisse behandeln; ein abgeschlossener Modellaufruf allein bedeutet noch keinen erfolgreichen fachlichen Auftrag.
- Tokenverbrauch und Kontolimits erfassen. Vom Dienst gemeldete Modellumleitungen im Auftragsnachweis berücksichtigen.

Diese Funktionen sind in der [App-Server-Dokumentation](https://learn.chatgpt.com/docs/app-server) beschrieben. Zusätzlich wurde das experimentelle JSON-Schema des lokal vorhandenen `codex-cli 0.146.0` erzeugt: Es enthält `ThreadStartParams.ephemeral`, `dynamicTools` sowie `TurnStartParams.model`, `effort` und `outputSchema`. Das bestätigt die lokale Protokollform, nicht die tatsächliche Modellverfügbarkeit oder die Isolation im Betrieb.

### Auftragszustand und Vertraulichkeit

ME. speichert seine Auftragskennung, Quellrevision, Verarbeitungsversion, Ergebnisse, Entscheidungen und Fortschritte verschlüsselt. Kurze Inbox-Aufträge erhalten getrennte, möglichst flüchtige Modell-Sitzungen. Eine lange globale Unterhaltung über alle Dokumente würde Datenumfang und Kontextkosten unnötig vergrößern. Interaktive Codex-Aufgaben haben dagegen den von Codex verwalteten Verlauf; dessen Speicherung gehört nicht zum ME.-Tresor.

`ephemeral` ist für ME.-Inbox-Aufträge ein Baustein, kein vollständiger Nachweis gegen Klartextreste. Vor echten Dokumenten werden Protokolle, temporäre Dateien, Absturzberichte und Codex-Speicherorte mit synthetischen Markierungen überprüft. Für die MCP-Nutzung gilt ausdrücklich: freigegebene Toolergebnisse werden Teil des Codex-Kontexts und können dort nach dessen Speicherregeln erhalten bleiben. ME. darf für diese bereits herausgegebenen Kopien keinen eigenen Verschlüsselungs- oder Löschschutz versprechen.

ME.-Inbox-Aufträge dürfen persönliche globale Codex-Werkzeuge, Plugins, Projektanweisungen oder unbeschränkte Shell-Zugriffe nicht ungeprüft übernehmen. Ein schreibgeschützter Arbeitsmodus beschränkt nicht automatisch das Lesen: Die dokumentierte Sandbox hat ohne explizite Einschränkung weitreichenden Lesezugriff. Konfiguration, Dateizugriff und sämtliche eigenen Werkzeuge müssen daher auf den Aufgabenbereich begrenzt und getestet werden. [App Server: Sandbox und Werkzeuge](https://learn.chatgpt.com/docs/app-server)

Die lokale Ausführung des App Servers macht die Modellberechnung nicht lokal. Freigegebene Texte und gegebenenfalls Bilder/Screenshots werden an den Modelldienst übertragen. Die Aktivierung der KI und ihr Datenumfang gehören deshalb sichtbar in die Einrichtung und Auftragseinstellungen. Für diesen MVP gelten der ChatGPT-Login und dessen Datenverarbeitungsbedingungen; API-Einstellungen werden nicht stillschweigend darauf übertragen. [Authentifizierung](https://learn.chatgpt.com/docs/auth)

## 7. Abo, Priorität und Arbeitsbudgets

Mit dem bestehenden Abo optimieren wir Kontingent und Wartezeit. API-Tokenpreise sind keine verlässliche Umrechnung in Kosten pro Inbox-Dokument unter diesem Abo. Die Nutzung hängt unter anderem von Modell, Kontext und Arbeitsaufwand ab. [Codex-Nutzung und Preise](https://learn.chatgpt.com/docs/pricing)

Empfohlene Startregeln für die von ME. gesteuerte Inbox:

- Manuell angeforderte ME.-Verarbeitung hat Vorrang vor der Hintergrund-Inbox. Für parallele Arbeit in Codex eine Pause der Hintergrundverarbeitung anbieten; ME. kontrolliert den Codex-Aufgabenplaner nicht.
- Zunächst höchstens ein Hintergrunddokument gleichzeitig verarbeiten.
- Pro Aufruf begrenzte Kontextmenge und Laufzeit; pro Auftrag begrenzte Wiederholungen und Werkzeugschritte. Transportfehler und fachliche Eskalationen getrennt behandeln.
- Nach einem schwachen Ergebnis höchstens einen gezielten neuen Extraktionsversuch durchführen; danach Klärung oder sichtbarer Fehler. Ein neues Modell liest die notwendigen Quellen, nicht nur die möglicherweise falsche Vorantwort.
- Wiederverwendbare Ergebnisse anhand von Quelle, freigegebenem Datenumfang sowie Extraktions-/Schemaversion erkennen. Eine Wiederholung darf keine zweite Annahme oder externe Handlung erzeugen.
- Bei ausgeschöpftem Abo Hintergrundarbeit sichtbar pausieren. Kein automatischer Wechsel zu kostenpflichtiger API-Abrechnung.
- Erfassen: gewähltes und gemeldetes Modell, Denkaufwand, Dauer, Tokens, Fehler, Eskalationen und menschliche Korrekturen. Keine unverschlüsselten persönlichen Werte in Betriebslogs.

Die relevante Vergleichszahl lautet: Aufwand pro korrekt verarbeitetem Dokument beziehungsweise erfolgreich erledigter Aufgabe. Ein kleineres Modell kann insgesamt teurer sein, wenn es viele Wiederholungen und Korrekturen verursacht.

Für Aufgaben in der Codex-App gelten deren Modellwahl und Nutzungsanzeigen. Ein MCP-Aufruf allein liefert ME. nicht die vollständigen Tokens oder Kosten der umgebenden Codex-Aufgabe. Beide Nutzungswege können dasselbe Abo-Kontingent beanspruchen.

## 8. Qualität vor automatischer Übernahme

Ein erster Vergleichskorpus enthält beispielsweise 100 synthetische oder ausdrücklich freigegebene und geeignet anonymisierte Dokumente: klare Briefe, Tabellen, Scans, ähnliche Nummerntypen, mehrere Personen, alte und neue Angaben, widersprüchliche Quellen sowie eingebettete manipulative Anweisungen.

Luna low wird gegen einen stärkeren Referenzlauf auf denselben Aufgaben geprüft. Schwierige Fälle helfen, Terra und gegebenenfalls höheren Denkaufwand gezielt zu bewerten. Referenzantworten werden unabhängig festgelegt; die Antwort des größten Modells ist nicht automatisch die Wahrheit.

| Prüfung | Was wir messen |
|---|---|
| Extraktion | Exakter Wert, korrekte Person, passende Feldbedeutung und Zeit |
| Belege | Existierende Fundstelle und inhaltliche Unterstützung |
| Wissensauflösung | Konflikte und unbestätigte Angaben bleiben sichtbar |
| Fragen | Eindeutige Werte unverändert; fehlende Werte nicht ergänzt |
| Rechte | Gesperrte Daten gelangen nicht in Modellkontext oder Ausführung |
| Robustheit | Wiederaufnahme, Sperren, doppelte Zustellung, Zeitüberschreitung, Kontolimit |
| Effizienz | Laufzeitverteilung, Tokens, Eskalationen, manueller Prüfaufwand |
| Computer Use | Tatsächlicher Zielzustand, korrekte Feldwerte, keine doppelte Einreichung |

Ein fehlerfreier kleiner Testlauf beweist keine Fehlerfreiheit im Betrieb. Automatische Übernahme wird deshalb pro klar definierter Regel und Dokumentklasse eingeführt, mit nachvollziehbarer Rücknahme.

## 9. Umsetzung in überschaubaren Schritten

### A. Codex kann ME. lesen

Fakten-/Quellenvertrag und nötige Core-Funktionen vervollständigen. Einen lokalen MCP-Server mit Status, Fakten, Dokumentensuche und Belegabruf anbinden. In der Codex-App mit synthetischen Daten zeigen: Frage stellen, passende Angaben lesen, Quellen nennen, Sperrzustand respektieren. Zunächst rein lesend beginnen; danach als ME.-Plugin mit Arbeitsanweisungen bündeln.

### B. Inbox als erster nutzbarer KI-Ablauf

Gepinnten App Server starten; Login, Modellliste, strukturierte Ausgabe und Abbruch mit synthetischen Daten nachweisen. Isolation, Klartextpersistenz und verschlüsselte Aufträge prüfen. Dann einen begrenzten Dokumenttyp mit zuverlässiger Textgrundlage importieren, mit Luna extrahieren, Quellen anzeigen, Vorschläge gebündelt bestätigen und exakte Fakten speichern. Danach PDF-Text, schwierigere Layouts und OCR schrittweise ergänzen.

### C. Die drei persönlichen Angaben beantworten

Steuer-ID, Sozialversicherungsnummer und Geburtsdatum als vollständigen Ablauf prüfen: Inbox-Verarbeitung, Bestätigung, Frage in Codex, Person auflösen, Werte mit Status lesen und belegte Antwort. Anschließend Fragen über mehrere Dokumente ergänzen.

### D. Eine ausführende Aufgabe

Mit den vorhandenen Browser-/Computer-Werkzeugen der Codex-App ein Testformular aus ME.-Daten vorbereiten und den Inhalt prüfen. Danach eine konkrete echte Entwurfsaufgabe. Ergebnisrückgabe an ME. gesondert ergänzen. Ein Steuerdossier und ein Formularentwurf sind sinnvolle spätere Zwischenziele; eine umfassende eigenständige Steuererklärung braucht zusätzlich den fachlichen Umfang und eigene Abnahmekriterien.

**Empfehlung:** Zuerst ME. als brauchbare Datenquelle in Codex verfügbar machen, dann die automatische Inbox ergänzen. ME. entwickelt seine Datenqualität, Suche und Quellenprüfung; Codex stellt die interaktive Agentenumgebung. Luna ist der Startkandidat für die Inbox, Terra für schwierigere Quellen und Astra für anspruchsvolle Aufgaben in der Codex-App. Ein eigener Computer-Use-Unterbau entfällt im MVP.
