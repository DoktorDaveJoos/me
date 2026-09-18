# Lokale Fehlerdiagnose

Unter **Einstellungen → Diagnose** lassen sich der Bericht der laufenden Sitzung
kopieren und der Logordner öffnen. Der Bericht enthält Build, Prozessstatus und
bis zu 200 aktuelle Ereignisse. `file_status=active` bestätigt den Dateischreiber;
`file_unavailable` bedeutet, dass nur der Bericht im Arbeitsspeicher verfügbar ist.
`dropped_events` zählt Ereignisse, die nicht in die volle Schreibwarteschlange passten.

Die JSONL-Dateien liegen neben dem Tresor in `logs/`, standardmäßig auf macOS unter
`~/Library/Application Support/ME/logs/`, auf Linux unter `$XDG_DATA_HOME/ME/logs/`
(bzw. `~/.local/share/ME/logs/`). `diagnostics.jsonl` ist aktuell, `.1` bis `.3`
sind ältere Dateien. Jede Datei umfasst höchstens 1 MiB. Ordner und Dateien sind
privat (0700/0600). Schreiben und Rotation laufen außerhalb des UI-Threads.

Erfasst werden Zeitstempel, Build, PID, lokale numerische Import-ID,
Verarbeitungsphase, Byte-/Abschnittszahlen, Laufzeiten, technische Statuswerte,
feste Fehlermeldungen des Core und erlaubte Codex-Fehlercodes.
Das Logging-API akzeptiert ausschließlich statische Bezeichnungen und Zahlen;
Dateinamen, Dokumenttext, Fundstellen, Provider-Payloads, rohe stderr-Ausgabe und
Zugangsdaten werden nicht protokolliert. Es gibt keine automatische Übertragung.

## Fehler eingrenzen

1. `document.started` und `document.deferred` zeigen, ob der Auftrag angenommen
   wurde oder gerade ein anderer Auftrag beziehungsweise die Einrichtung blockiert.
2. `document.read` → `document.extract_text` → `text_helper.finished` →
   `document.index` zeigen lokale Entschlüsselung, Erkennung und Textmenge.
3. `ai.prepare` → `ai.provider` → `codex.rpc.*` → `codex.batch.*` zeigen Vorbereitung,
   Verbindung und jeden KI-Abschnitt. Timeouts und unerwartetes Prozessende haben
   eigene Ereignisse. Providertexte werden durch feste Codes ersetzt.
4. `ai.validate_and_save` und `vault.validation` zeigen abgewiesene Belege. `codex.evidence.rejected` nennt den konkreten Grund
   (unbekanntes Segment, fehlendes Zitat, Wert oder Personenbeleg).
   `codex.batch.retry`, `.split` und `.resumed` zeigen Wiederholungen, weitere
   Aufteilung und Wiederverwendung; `text_helper.page` zeigt den Seitenfortschritt.
   `document.finished` enthält Erfolg, Abbruch, letzte Phase und Gesamtdauer.
5. Bei stillstehender Oberfläche den Bericht kopieren: `app.snapshot` enthält
   Einrichtung, Beschäftigung, Automatik und Warteschlangenstatus.

Fehler werden verschlüsselt am Import gespeichert und prominent im Dokumentdialog
angezeigt. Neue Fehler erscheinen außerdem oben in der Inbox, auch wenn die Queue
mit einer anderen Datei fortfährt. Erneutes Auswerten erfolgt im Dokumentdialog.
Die automatische Auswertung bleibt eine ausdrückliche Tresoreinstellung.

## Regression des 24-KB-Fehlers

Früher brach `prepare_extraction` schon vor dem Codex-Aufruf bei 24.000 Textbytes
ab. Das war erheblich strenger als das lokale 1-MiB-Limit; lange PDFs waren daher
lokal lesbar, aber nicht mit KI auswertbar. Der aktuelle Ablauf verarbeitet alle
Segmente in mehreren Aufrufen und übernimmt Ergebnisse erst nach vollständigem Durchlauf. Nicht belegte einzelne Kandidaten werden nach
einem Korrekturversuch als offene Rückfragen gespeichert und einzeln angezeigt.
`review.answered` protokolliert nur Dokument-ID und Bestätigung/Verwerfen, keine
Werte, Namen, KI-Wortlaute oder Nutzereingaben.
Geprüfte Abschnitte bleiben verschlüsselt für eine Wiederaufnahme erhalten.

Tests prüfen Unicode-Grenzen, vollständige Abschnittsabdeckung, Fehler im zweiten
Aufruf, Abbruch, atomare Belegvalidierung, Logrotation und private Dateirechte.
Der explizite Live-Test `long_pdf_finds_facts_beyond_the_former_limit` nutzt ein
synthetisches PDF mit den relevanten Angaben auf Seite 12 hinter der früheren
Grenze und prüft gleichzeitig, dass diese Inhalte nicht im Diagnosebericht stehen.
