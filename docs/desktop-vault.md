# Desktop-Tresor: erster Implementierungsschritt

Stand: 15. September 2026. Rust + GPUI 0.2.2, macOS und Linux als Ziele.

Der anschließende Ausbau mit Standardfeldern, lesendem MCP-Zugang und erster
Inbox-KI ist in [codex-integration.md](codex-integration.md) dokumentiert.
Die folgenden Grenzen und Testergebnisse beschreiben den ursprünglichen Tresorschritt.

## Architekturentscheidung

Grundlage sind die beiden Ausarbeitungen in `docs/architecture/` einschließlich
SQL-Entwurf und seiner synthetischen Tests. Die bevorzugte native Variante wird
für diesen ersten Schritt umgesetzt: SQLCipher im gemeinsamen `me-core`, separat
verschlüsselte Originale, lokale Suche. PostgreSQL/pgvector bleibt ein möglicher
Vergleichskandidat, kein zusätzlicher Desktop-Prozess.

Die Forschungsdokumente enthielten eine kostenlose quelloffene Produktannahme.
Die spätere Veröffentlichungsentscheidung ersetzt diese Lizenzannahme: Der Code
ist öffentlich einsehbar, seine Nutzung erfordert jedoch eine separate bezahlte
Vereinbarung gemäß [LICENSE](../LICENSE). `publish = false` bleibt bestehen. Die
frühere OpenAI-exklusive Pro-Annahme ist keine technische Voraussetzung dieses Kerns.

## Bereits implementiert

- Tresor anlegen, Passwort wiederholen, entsperren, manuell sperren und schließen.
- Masterpasswort: mindestens zehn Zeichen; keine Pflicht zu Großbuchstaben,
  Ziffern oder Sonderzeichen. Die Einrichtung zeigt einen unverbindlichen Hinweis
  zu leicht erratbaren Wörtern, ohne eine zusätzliche Bestätigung zu verlangen.
- Passwortableitung mit Argon2id v19, 64 MiB, drei Durchläufe, Parallelität eins.
  Zufälliges 16-Byte-Salt; Parameter sind an Formatversion 1 gebunden.
- Zwei unabhängige zufällige 256-Bit-Schlüssel für Datenbank und Objekte. Das
  Passwort verschlüsselt das Schlüsselpaket mit XChaCha20-Poly1305. Keine Schlüssel
  im Klartext auf dem Datenträger, kein Passwort- oder Schlüssel-Logging.
- Gepinntes `rusqlite 0.40.2` mit gebündeltem SQLCipher und OpenSSL. Der Core
  prüft `cipher_version`, Schlüssel, Schema und Tresor-ID. Kein SQLite-Fallback.
  Temporärer SQL-Speicher im RAM; FULL-Synchronisierung, verschlüsseltes Journal.
- Zufällige Objekt-IDs, zufällige 24-Byte-Nonces, authentifizierte Bindung an
  Tresor-ID und Objekt-ID. Dateiname und Fingerprint stehen in SQLCipher.
- Neue Dateien über temporäre Datei, `sync_all`, Veröffentlichung ohne Überschreiben
  und Verzeichnis-Flush. Tresorordner mit Modus 0700; keine Klartext-Vorschauen.
- Freie Angaben mit stabiler Identität, unveränderlichen Aussagen, manuellen
  Quellen, Bestätigungen und Rücknahmen. Änderungen überschreiben alte Werte
  nicht. Unbekannte Gültigkeit wird ausdrücklich als `unknown` gespeichert.
  Freie Werte sind Text: führende Nullen bleiben erhalten.
- Originalimport bis 64 MiB. Lieferkennungen verhindern die Wiederholung desselben
  Eingangs; bewusst erneuter Import gleicher Bytes erzeugt einen neuen Kontext.
- Unklassifizierte Dateien: nur Titel indexiert. UTF-8-TXT/Markdown/CSV bis 1 MiB
  nach ausdrücklicher Bestätigung ohne Zugangsdaten mit FTS5 durchsuchbar.
  Credential-Quellen sind von der Indexierung ausgeschlossen. PDF/OCR, DOCX, ODT,
  RTF und weitere Textformate sind über die [Dokumentverarbeitung](document-processing.md)
  angeschlossen.
- Dauerhafte Import-/Indexjobs. Abgebrochene laufende Textjobs werden beim nächsten
  Entsperren wiederholt. Fachänderungen und lokales Änderungsjournal in einer
  Transaktion. Das Journal ist noch kein Sync-Protokoll.
- Hintergrundausführung für KDF, Datenbank, Import, Suche und Backup. Suche mit
  160-ms-Verzögerung; Sitzungs-/Suchgenerationen verwerfen verspätete Ergebnisse.
- Sicherungen als neuer `.mebackup`-Ordner: konsistentes verschlüsseltes SQL-Backup,
  authentifizierte Originale, Metadaten zuletzt. Wiederherstellung in einen neuen
  Tresor; bestehender Tresor und vorhandene Exportdateien werden nie überschrieben.
- Expliziter Export einer unverschlüsselten Kopie über den Speicherdialog.
- Kopierte Werte werden nach 30 Sekunden, beim Sperren und beim Beenden geleert,
  sofern der Zwischenablageinhalt noch dem kopierten Wert entspricht.

## Speicherort und Start

macOS: `~/Library/Application Support/ME/vault`.
Linux: `$XDG_DATA_HOME/ME/vault`, sonst `~/.local/share/ME/vault`.
`ME_VAULT_DIR` erlaubt einen **absoluten** alternativen Pfad, z. B. für Tests.

```sh
./scripts/cargo run
./scripts/bundle-macos debug
```

Die neue Oberfläche beginnt mit einem leeren Tresor. Die frühere Sitzungsvorschau
hat keine persistierten Daten, die automatisch migriert werden könnten.
Sperren: `Cmd+Shift+L`, unter Linux `Ctrl+Shift+L`.

## Testumfang und Grenzen

Verhaltenstests benutzen ausschließlich synthetische Daten. Geprüft werden reale
SQLCipher-Dateien, falsches Passwort, fehlender Schlüssel, Klartextmarker auf dem
Datenträger, Manipulation eines Dateiobjekts, Import-Idempotenz, führende Nullen,
Verlauf, UTF-8-Suche, Credential-Ausschluss, Transaktionsrollback, Wiederanlauf,
Schema-Abweisung, zweite Instanz, Backup und Wiederherstellung nach Verlust des
Originaltresors. Das ist kein externer Sicherheitsnachweis.

- macOS wurde gebaut; Linux-Build/Wayland/X11 und Release-Leistung müssen separat
  geprüft werden. Keine Performance-Zahlen aus Debug-Builds ableiten.
- Die Liste zeigt höchstens 200 Treffer. Es gibt noch keine Pagination.
- Noch kein vollständiger zeitlicher Resolver, Konfliktreview, typisierte
  PDF-Vorschau, Embeddings, Tray, automatische Sperre, Geräteverwaltung oder Sync.
  Der Stand von Standardfeldern, KI-Adapter und begrenzter Codex-Autorisierung
  steht in [Codex-Integration](codex-integration.md).
- Kein Credential-Manager: normale Angaben sind kein Ort für Passwörter oder
  private Schlüssel. Klassifizierung ist hier eine Nutzerentscheidung.
- Keine Passwortänderung, Schlüsselrotation oder Wiederherstellung ohne Passwort.
  Backup und Passwort werden benötigt; Sync ersetzt später kein Backup.
- Sperren gibt Verbindung, Schlüssel und UI-Ansichten frei. GPUI/OS können Kopien
  unveränderlicher Texte oder Grafikdaten behalten; vollständige RAM-/Swap-
  Bereinigung wird nicht behauptet. Passwortfelder sind maskiert und sperren
  Kopieren/Ausschneiden; natives Secure-Input und Accessibility sind offene Arbeit.
- Ein kompromittierter entsperrter Client liegt außerhalb dieser Schutzgrenze.
- Ein Absturz zwischen Objektablage und SQL-Commit kann ein unreferenziertes
  verschlüsseltes Objekt hinterlassen. Es gilt nicht als importiert und wird nicht
  in Backups aufgenommen. Garbage Collection und physische Löschung folgen.
- Keine Hintergrundlöschung von Originalen. Löschen mit Belegauflösung,
  Löschmarkierungen und Backup-Aufbewahrung muss gemeinsam implementiert werden.
- Exportdateien und vom Betriebssystem/anderen Apps kopierte Zwischenablageinhalte
  liegen außerhalb des Tresors. Export überschreibt bewusst keine vorhandene Datei.

## Nächster Schritt

Typisierte Aussagen, feldbezogene Zeitauflösung und Konfliktprüfung auf dem
vorhandenen Quellen-/Entscheidungsmodell. Danach lokale Dokumentextraktion und
hybride Suche. Agenten bekommen fachliche Werkzeuge statt Datenbankzugriff.
Sync wird als versioniertes, authentifiziertes Operations-/Objektprotokoll getrennt
spezifiziert und mit mehreren Offlineclients geprüft.

## Primärquellen der Implementierung

- [SQLCipher API](https://www.zetetic.net/sqlcipher/sqlcipher-api/)
- [rusqlite 0.40.2](https://docs.rs/rusqlite/0.40.2/rusqlite/)
- [Argon2 0.5.3](https://docs.rs/argon2/0.5.3/argon2/)
- [XChaCha20-Poly1305 0.10.1](https://docs.rs/chacha20poly1305/0.10.1/chacha20poly1305/)

## Prüfprotokoll dieses Schritts

- 13 Rust-Verhaltenstests mit realem SQLCipher: bestanden.
- 19 ursprüngliche Python-Schematests: bestanden (weiterhin reine Schematests).
- Workspace-Build, Formatierung und Clippy mit `-D warnings`: bestanden.
- macOS-Testbundle mit eigenem synthetischem Tresor: gestartet, Passwortanlage
  und Wechsel zur Hauptansicht visuell geprüft. Weitere Bedienungstests wurden
  durch die gesperrte macOS-Sitzung unterbrochen. Die nachfolgende Fokuskorrektur
  benötigt deshalb noch einen erneuten manuellen Tastaturtest.

## 1Password credentials

Schema 6 adds credential items and complete 1PUX imports inside SQLCipher.
They use the existing encrypted backup and remain outside document processing
and Codex access. See [the importer and its limits](1password-import.md).
