//! Synthetic import UI harness; no user vault or provider connection is used.
//! `ME_VAULT_DIR=/tmp/<fresh>/vault ./scripts/cargo run -p me-app --example
//! import_gallery -- <mode> [small] [expanded] [scroll=<px>]`; modes: confirm, progress,
//! failure, budget, batch (stopped at its import's allowance), complete, read (a
//! document detail after a full read).
#![allow(dead_code)]
#[path = "../src/assets.rs"]
mod assets;
#[path = "../src/design_system.rs"]
mod design_system;
#[path = "../src/input.rs"]
mod input;
#[path = "../src/theme.rs"]
mod theme;
mod shell {
    include!("../src/shell.rs");
    pub fn fixture(
        cx: &mut Context<MeApp>,
        mut vault: Vault,
        samples: Vec<PathBuf>,
        mode: String,
    ) -> MeApp {
        let mut app = MeApp::new(cx);
        if mode == "read" {
            let item = super::read_fixture(&mut vault, samples[0].parent().unwrap());
            app.document_open = Some(item);
            app.document_read = Some(vault.document_read(item).unwrap());
        }
        // Prevent the deferred connection check from launching a provider process.
        app.codex_cancel = Some(Arc::new(std::sync::atomic::AtomicBool::new(true)));
        app.codex_ready = true;
        app.unlocked = true;
        app.initialized = true;
        app.busy = false;
        app.root = None;
        app.focus_filter_on_ready = false;
        app.page = Page::Imports;
        app.settings.automatic_evaluation = false;
        app.collection = vault.collection("", false).unwrap();
        app.import_jobs = vault.import_jobs().unwrap();
        app.session = Arc::new(Mutex::new(Some(vault)));
        if mode == "confirm" {
            app.accept_documents(&samples, cx);
        }
        if mode == "progress" {
            for (index, job) in app.import_jobs.iter().take(2).enumerate() {
                let mut active =
                    ActiveImport::new(Arc::new(std::sync::atomic::AtomicBool::new(false)), true);
                active.stage = if index == 0 {
                    me_core::ImportStage::Normalizing
                } else {
                    me_core::ImportStage::Verifying
                };
                active.current = if index == 0 { 3 } else { 2 };
                active.total = if index == 0 { 8 } else { 4 };
                active.steps = if index == 0 {
                    [
                        me_core::StepProgress {
                            current: 3,
                            total: 8,
                        },
                        me_core::StepProgress::default(),
                        me_core::StepProgress::default(),
                        me_core::StepProgress::default(),
                        me_core::StepProgress::default(),
                    ]
                } else {
                    [
                        me_core::StepProgress {
                            current: 1,
                            total: 1,
                        },
                        me_core::StepProgress {
                            current: 4,
                            total: 4,
                        },
                        me_core::StepProgress {
                            current: 4,
                            total: 4,
                        },
                        me_core::StepProgress {
                            current: 4,
                            total: 4,
                        },
                        me_core::StepProgress {
                            current: 2,
                            total: 4,
                        },
                    ]
                };
                active.message = if index == 0 {
                    "Recognizing text and preserving page evidence…"
                } else {
                    "Checking sources and looking for missing details…"
                }
                .into();
                app.active_imports.insert(job.item, active);
            }
        }
        if mode == "failure" || mode == "budget" || mode == "batch" {
            let job = &mut app.import_jobs[0];
            job.state = "failed".into();
            job.stage = me_core::ImportStage::Extracting;
            job.steps = [
                me_core::StepProgress {
                    current: 1,
                    total: 1,
                },
                me_core::StepProgress {
                    current: 6,
                    total: 8,
                },
                me_core::StepProgress {
                    current: 6,
                    total: 8,
                },
                me_core::StepProgress {
                    current: 5,
                    total: 8,
                },
                me_core::StepProgress {
                    current: 5,
                    total: 8,
                },
            ];
            job.usage.openai_calls = if mode == "budget" { 12 } else { 7 };
            job.usage.typesafe_calls = 12;
            job.usage.input_tokens = 19560;
            job.usage.output_tokens = 2940;
            job.usage.unreported_calls = 1;
            let (provider, kind, message) = match mode.as_str() {
                "budget" => (
                    me_core::ImportProvider::Local,
                    me_core::ImportErrorKind::Budget,
                    "This file reached its analysis allowance. Saved steps are kept. Review usage before allowing more calls.",
                ),
                "batch" => (
                    me_core::ImportProvider::Local,
                    me_core::ImportErrorKind::BatchBudget,
                    "This import reached its OpenAI allowance. Saved steps are kept. Allow more OpenAI calls for this import to continue.",
                ),
                _ => (
                    me_core::ImportProvider::OpenAi,
                    me_core::ImportErrorKind::Quota,
                    "OpenAI usage limit reached. Imports are paused. Resume after your account allowance resets.",
                ),
            };
            job.error = Some(message.into());
            job.failure = Some(me_core::ImportFailure::new(provider, kind, message));
            if mode == "batch" {
                job.usage.batch = Some(me_core::BatchAllowance {
                    id: "synthetic-import".into(),
                    openai_calls: 200,
                    openai_allowance: 200,
                });
                app.import_batch_stops = vec![me_core::ExhaustedBatch {
                    id: "synthetic-import".into(),
                    label: "Folder import".into(),
                    openai_calls: 200,
                    openai_allowance: 200,
                    stopped: 3,
                }];
            }
            if mode == "failure" {
                app.import_pause = Some(message.into());
            }
        }
        if mode == "complete" {
            let job = &mut app.import_jobs[0];
            job.state = "done".into();
            job.steps = [me_core::StepProgress {
                current: 1,
                total: 1,
            }; 5];
            job.proposals = 17;
            job.questions = 2;
        }
        if std::env::args().any(|arg| arg == "expanded") {
            app.expanded_imports.insert(app.import_jobs[0].item);
        }
        app
    }
}
use gpui::{
    App, Application, Bounds, KeyBinding, WindowBounds, WindowOptions, prelude::*, px, size,
};
use me_core::{
    Candidate, CandidateKind, CandidateValue, ConfidenceSource, DocumentGraph, DocumentPart,
    DocumentRead, FactState, ReadFact, SlotContent, SlotValue, Uncovered,
};

/// A synthetic two-page payslip, read in full: four profile values (one waiting
/// in Quick checks), other details (two uncertain, one with a long label and
/// value) and three values nobody interpreted. Returns its collection item.
fn read_fixture(vault: &mut me_core::Vault, dir: &std::path::Path) -> u64 {
    let path = dir.join("Payslip January 2026 · synthetic.txt");
    let pages = [
        "Entgeltabrechnung Januar 2026\nErika Beispiel\nBruttoentgelt 4.250,00\nLohnsteuer 612,41\nKirchensteuer 49,99\nNettoentgelt 2.745,18\nKV-Beitrag 348,50",
        "Kostenstelle 4711\nPersonalnummer 000123\nArbeitgeberanteil Rentenversicherung 395,25\nResturlaub aus dem Vorjahr, übertragbar bis 31.03. 12,5 Tage\nVermögenswirksame Leistungen, Arbeitgeberzuschuss laut Tarifvertrag 40,00 EUR monatlich bis Dezember 2026\nUmlage U1 12,75\n7,30",
    ];
    std::fs::write(&path, pages.join("\n")).unwrap();
    let item = vault
        .import_document(
            &path,
            "synthetic-payslip",
            me_core::DocumentClass::Unclassified,
        )
        .unwrap();
    // Released explicitly, with page locators as a PDF reader would record them.
    let input = vault.begin_document_processing(item).unwrap().unwrap();
    let parts: Vec<_> = (1..)
        .zip(pages)
        .map(|(page, text)| DocumentPart {
            text: text.into(),
            page: Some(page),
            section: None,
            method: "pdf_text".into(),
        })
        .collect();
    vault.finish_document_processing(&input, &parts).unwrap();
    let (source, _, segments) = vault.document_texts(item).unwrap();
    // Where a printed value is: its segment and byte range.
    let at = |text: &str| {
        segments
            .iter()
            .find_map(|s| {
                s.text
                    .find(text)
                    .map(|start| (s.segment_id.clone(), start, start + text.len()))
            })
            .unwrap()
    };
    let money = |slot: &str, printed: &str, amount: &str, confidence: f64| {
        let (segment_id, start, end) = at(printed);
        SlotValue {
            slot: slot.into(),
            content: SlotContent::Candidate(Box::new(Candidate {
                id: String::new(),
                kind: CandidateKind::Money,
                text: printed.into(),
                value: CandidateValue::Money {
                    amount: amount.into(),
                    currency: "EUR".into(),
                },
                segment_id,
                start,
                end,
                line: printed.into(),
                label: None,
                checksum: false,
            })),
            period: None,
            confidence,
            source: ConfidenceSource::Typesafe,
            value_checked: false,
            check: false,
        }
    };
    let month = {
        let (segment_id, start, end) = at("Januar 2026");
        SlotValue {
            slot: "pay_month".into(),
            content: SlotContent::Candidate(Box::new(Candidate {
                id: String::new(),
                kind: CandidateKind::Period,
                text: "Januar 2026".into(),
                value: CandidateValue::Period {
                    start: "2026-01-01".into(),
                    end: "2026-01-31".into(),
                },
                segment_id,
                start,
                end,
                line: "Entgeltabrechnung Januar 2026".into(),
                label: None,
                checksum: false,
            })),
            period: None,
            confidence: 0.95,
            source: ConfidenceSource::Typesafe,
            value_checked: false,
            check: false,
        }
    };
    let fact = |label: &str, printed: &str, slot: Option<&str>, context: &str, state| {
        let (segment_id, start, end) = at(printed);
        ReadFact {
            label: label.into(),
            value: printed.into(),
            segment_id,
            start,
            end,
            context: context.into(),
            owner: "self".into(),
            owner_entity: None,
            owner_name: None,
            period: Some("document".into()),
            slot: slot.map(str::to_owned),
            state,
            confidence: Some(0.95),
        }
    };
    let uncovered = |label: Option<&str>, printed: &str| {
        let (segment_id, start, end) = at(printed);
        Uncovered {
            segment_id,
            start,
            end,
            kind: CandidateKind::Amount,
            label: label.map(str::to_owned),
            text: printed.into(),
            line: printed.into(),
        }
    };
    let me = vault.profile_entity_id().unwrap();
    let january = "Januar 2026";
    let read = DocumentRead {
        run_id: "synthetic-read".into(),
        graph: Some(DocumentGraph {
            doc_type: "payslip".into(),
            subject: Some(me),
            subject_confidence: 0.95,
            values: vec![
                money("gross", "4.250,00", "4250.00", 0.95),
                money("wage_tax", "612,41", "612.41", 0.95),
                money("church_tax", "49,99", "49.99", 0.75),
                money("net", "2.745,18", "2745.18", 0.95),
                month,
            ],
            models: vec![],
            correction: false,
        }),
        facts: vec![
            fact(
                "Bruttoentgelt",
                "4.250,00",
                Some("gross"),
                january,
                FactState::Verified,
            ),
            fact(
                "Lohnsteuer",
                "612,41",
                Some("wage_tax"),
                january,
                FactState::Verified,
            ),
            fact(
                "Kirchensteuer",
                "49,99",
                Some("church_tax"),
                january,
                FactState::Verified,
            ),
            fact(
                "Nettoentgelt",
                "2.745,18",
                Some("net"),
                january,
                FactState::Verified,
            ),
            fact("Kostenstelle", "4711", None, "", FactState::Verified),
            fact("Personalnummer", "000123", None, "", FactState::Verified),
            fact(
                "Arbeitgeberanteil Rentenversicherung",
                "395,25",
                None,
                january,
                FactState::Verified,
            ),
            fact(
                "Resturlaub aus dem Vorjahr, übertragbar bis 31.03.",
                "12,5 Tage",
                None,
                "",
                FactState::Uncertain,
            ),
            fact(
                "Vermögenswirksame Leistungen, Arbeitgeberzuschuss laut Tarifvertrag",
                "40,00 EUR monatlich bis Dezember 2026",
                None,
                january,
                FactState::Uncertain,
            ),
        ],
        uninterpreted: vec![
            uncovered(Some("KV-Beitrag"), "348,50"),
            uncovered(Some("Umlage U1"), "12,75"),
            uncovered(None, "7,30"),
        ],
        rejected: Default::default(),
    };
    vault.apply_read(&source, &read).unwrap();
    item
}
fn main() {
    let root = std::env::var_os("ME_VAULT_DIR")
        .map(std::path::PathBuf::from)
        .expect("Set ME_VAULT_DIR to a fresh synthetic vault path");
    assert!(
        root.starts_with("/tmp") || root.starts_with("/private/tmp"),
        "Only synthetic /tmp vaults are allowed"
    );
    let parent = root.parent().unwrap();
    std::fs::create_dir_all(parent).unwrap();
    let mut vault = me_core::Vault::create(&root, "synthetic-gallery-passphrase").unwrap();
    vault.set_automatic_evaluation(false).unwrap();
    let names = [
        "Health insurance · September.eml",
        "Letter from the insurance provider with a long descriptive filename.pdf",
        "Personal notes.txt",
    ];
    let mut samples = Vec::new();
    for name in names {
        let path = parent.join(name);
        std::fs::write(&path, "SYNTHETIC EXAMPLE\nReference: 00042").unwrap();
        vault
            .import_document(&path, name, me_core::DocumentClass::Unclassified)
            .unwrap();
        samples.push(path);
    }
    let mode = std::env::args().nth(1).unwrap_or_default();
    let small = std::env::args().any(|arg| arg == "small");
    let (width, height) = if small { (800., 600.) } else { (1120., 780.) };
    // `scroll=<px>` sets the imports page's and the open document detail's GPUI
    // scroll offset once the window is drawn, so a capture can show the lower
    // part of a scrolling dialog. Safe GPUI API only (`ScrollHandle::set_offset`,
    // clamped to content height on the next layout); no OS-level input is
    // posted, so this works on every platform, not just macOS.
    let scroll = std::env::args().find_map(|arg| {
        arg.strip_prefix("scroll=")
            .and_then(|v| v.parse::<f32>().ok())
    });
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            assets::load_fonts(cx).unwrap();
            input::register_bindings(cx);
            cx.bind_keys([
                KeyBinding::new("escape", shell::Dismiss, Some("Me")),
                KeyBinding::new("enter", shell::Confirm, Some("Me")),
            ]);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                            None,
                            size(px(width), px(height)),
                            cx,
                        ))),
                        ..Default::default()
                    },
                    move |_, cx| cx.new(|cx| shell::fixture(cx, vault, samples, mode)),
                )
                .unwrap();
            window
                .update(cx, |_, window, _| {
                    window.resize(size(px(width), px(height)));
                    window.set_window_title("ME Import Gallery — Synthetic")
                })
                .unwrap();
            if let Some(offset) = scroll {
                window
                    .update(cx, |app, _, cx| {
                        // Whichever of the two is on screen for this mode is what a
                        // capture needs scrolled; setting the other's offset is inert.
                        let target = gpui::point(px(0.), px(-offset));
                        app.import_scroll.set_offset(target);
                        app.document_scroll.set_offset(target);
                        cx.notify();
                    })
                    .unwrap();
            }
            cx.activate(true);
        });
}
