//! Synthetic harness for "Who are you?", the folder dump, the constellation and
//! quick checks. Fictional data only; no user vault or provider connection is used.
//! Modes: who, born, home, files, plan, world, checks. Add `small` for 800×600.
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
    pub fn fixture(cx: &mut Context<MeApp>, vault: Vault, mode: String) -> MeApp {
        let mut app = MeApp::new(cx);
        app.codex_cancel = Some(Arc::new(std::sync::atomic::AtomicBool::new(true)));
        app.codex_ready = true;
        app.unlocked = true;
        app.initialized = true;
        app.busy = false;
        app.root = None;
        app.focus_filter_on_ready = false;
        app.settings.automatic_evaluation = false;
        app.settings.onboarding_complete = true;
        app.collection = vault.collection("", false).unwrap();
        app.import_jobs = vault.import_jobs().unwrap();
        let mut anchors = vault.identity_anchors().unwrap();
        app.graph.checks = vault.quick_checks().unwrap();
        app.graph.households = vault.household_proposals().unwrap();
        app.graph.merges = vault.merge_proposals().unwrap();
        app.graph.constellation = vault.constellation(30).unwrap();
        let step = ["who", "born", "home", "files"]
            .iter()
            .position(|m| *m == mode);
        anchors.setup_complete = step.is_none() && mode != "plan";
        if let Some(step) = step {
            app.graph.setup_step = step;
        }
        if mode == "who" {
            app.identity_name
                .update(cx, |i, cx| i.set_text("Max Mustermann", cx));
        }
        if mode == "plan" {
            app.graph.setup_step = 3;
            let plan = me_core::FolderPlan {
                files: (0..2870)
                    .map(|i| PathBuf::from(format!("/tmp/dump/{i}.pdf")))
                    .collect(),
                bytes: 3_400_000_000,
                unsupported: 96,
                skipped: 12,
                hidden: 341,
                truncated: false,
            };
            app.graph.plan = Some((vec![PathBuf::from("/tmp/dump")], plan));
        }
        if mode == "world" {
            app.page = Page::Imports;
            app.graph.batch = Some(me_core::BatchProgress {
                id: "synthetic".into(),
                label: "Folder import".into(),
                files: 2870,
                duplicates: 341,
                finished: 1214,
                failed: 3,
                openai_calls: 18,
                openai_allowance: 200,
                typesafe_requests: 2100,
            });
        }
        if mode == "checks" {
            app.page = Page::Review;
        }
        app.graph.anchors = Some(anchors);
        app.session = Arc::new(Mutex::new(Some(vault)));
        app
    }
}
use gpui::{
    App, Application, Bounds, KeyBinding, WindowBounds, WindowOptions, prelude::*, px, size,
};
use me_core::{
    Candidate, CandidateKind, CandidateValue, ConfidenceSource, DocumentGraph, SlotContent,
    SlotValue,
};

fn value(
    slot: &str,
    kind: CandidateKind,
    text: &str,
    v: CandidateValue,
    confidence: f64,
) -> SlotValue {
    SlotValue {
        slot: slot.into(),
        content: SlotContent::Candidate(Box::new(Candidate {
            id: format!("c-{slot}"),
            kind,
            text: text.into(),
            value: v,
            segment_id: "synthetic".into(),
            start: 0,
            end: text.len(),
            line: text.into(),
            label: None,
            checksum: false,
        })),
        period: None,
        confidence,
        source: ConfidenceSource::Typesafe,
        value_checked: false,
        check: false,
    }
}
fn text(slot: &str, kind: CandidateKind, s: &str) -> SlotValue {
    value(slot, kind, s, CandidateValue::Text(s.into()), 0.95)
}
fn date(slot: &str, d: &str) -> SlotValue {
    value(
        slot,
        CandidateKind::Date,
        d,
        CandidateValue::Date(d.into()),
        0.95,
    )
}
fn ident(slot: &str, kind: CandidateKind, s: &str, confidence: f64) -> SlotValue {
    value(
        slot,
        kind,
        s,
        CandidateValue::Identifier(s.into()),
        confidence,
    )
}

fn seed(vault: &mut me_core::Vault, parent: &std::path::Path) {
    vault
        .set_identity("Max Mustermann", &[], Some("1988-03-14"))
        .unwrap();
    vault
        .add_household_member("Lena Mustermann", "partner")
        .unwrap();
    let me = vault.profile_entity_id().unwrap();
    let document = |vault: &mut me_core::Vault, name: &str| {
        let path = parent.join(name);
        std::fs::write(&path, format!("SYNTHETIC EXAMPLE {name}")).unwrap();
        let item = vault
            .import_document(&path, name, me_core::DocumentClass::Unclassified)
            .unwrap();
        vault.enable_text_search(item).unwrap();
        vault.document_texts(item).unwrap().0
    };
    let graphs = vec![
        (
            document(vault, "reisepass.txt"),
            DocumentGraph {
                doc_type: "passport".into(),
                subject: Some(me.clone()),
                subject_confidence: 1.,
                values: vec![
                    ident("number", CandidateKind::Identifier, "C01X00T47", 1.),
                    date("expires", "2031-04-30"),
                ],
                models: vec!["synthetic".into()],
                correction: false,
            },
        ),
        (
            document(vault, "gehalt-januar.txt"),
            DocumentGraph {
                doc_type: "payslip".into(),
                subject: Some(me.clone()),
                subject_confidence: 0.95,
                values: vec![
                    text("employer", CandidateKind::Organization, "Acme GmbH"),
                    date("period_start", "2026-01-01"),
                    date("period_end", "2026-01-31"),
                ],
                models: vec!["synthetic".into()],
                correction: false,
            },
        ),
        (
            document(vault, "versicherungsschein.txt"),
            DocumentGraph {
                doc_type: "insurance_policy".into(),
                subject: Some(me.clone()),
                subject_confidence: 0.9,
                values: vec![
                    text("insurer", CandidateKind::Organization, "HUK-COBURG"),
                    ident("number", CandidateKind::Identifier, "KFZ-4711-0815", 0.66),
                ],
                models: vec!["synthetic".into()],
                correction: false,
            },
        ),
        (
            document(vault, "steuerbescheid-2024.txt"),
            DocumentGraph {
                doc_type: "tax_assessment".into(),
                subject: Some(me.clone()),
                subject_confidence: 0.9,
                values: vec![ident("tax_id", CandidateKind::TaxId, "12345678903", 0.9)],
                models: vec!["synthetic".into()],
                correction: false,
            },
        ),
        (
            document(vault, "steuerbescheid-2025.txt"),
            DocumentGraph {
                doc_type: "tax_assessment".into(),
                subject: Some(me.clone()),
                subject_confidence: 0.9,
                values: vec![ident("tax_id", CandidateKind::TaxId, "12345678930", 0.9)],
                models: vec!["synthetic".into()],
                correction: false,
            },
        ),
    ];
    for name in ["Techniker Krankenkasse", "Techniker KK"] {
        let source = document(vault, &format!("{name}.txt"));
        vault
            .apply_document_graph(
                &source,
                &DocumentGraph {
                    doc_type: "health_insurance_notice".into(),
                    subject: Some(me.clone()),
                    subject_confidence: 0.9,
                    values: vec![text("insurer", CandidateKind::Organization, name)],
                    models: vec!["synthetic".into()],
                    correction: false,
                },
            )
            .unwrap();
    }
    for (source, graph) in graphs {
        vault.apply_document_graph(&source, &graph).unwrap();
    }
    for i in 0..3 {
        let source = document(vault, &format!("brief-paul-{i}.txt"));
        vault
            .save_document_profile(
                &source,
                "correspondence",
                0.9,
                Some("letter"),
                Some(0.9),
                None,
                Some("Paul Mustermann"),
                None,
            )
            .unwrap();
    }
    let candidates = vault.merge_candidates(10).unwrap();
    for c in candidates {
        vault.record_merge_judgment(&c.a, &c.b, 0.9, true).unwrap();
    }
}

fn main() {
    let root = std::env::var_os("ME_VAULT_DIR")
        .map(std::path::PathBuf::from)
        .expect("Set ME_VAULT_DIR to a fresh synthetic vault path");
    assert!(
        root.starts_with("/tmp") || root.starts_with("/private/tmp"),
        "Only synthetic /tmp vaults are allowed"
    );
    let parent = root.parent().unwrap().to_owned();
    std::fs::create_dir_all(&parent).unwrap();
    let mut vault = me_core::Vault::create(&root, "synthetic-gallery-passphrase").unwrap();
    vault.set_automatic_evaluation(false).unwrap();
    let mode = std::env::args().nth(1).unwrap_or_else(|| "who".into());
    if !["who"].contains(&mode.as_str()) {
        seed(&mut vault, &parent);
    }
    let small = std::env::args().any(|arg| arg == "small");
    let (w, h) = if small { (800., 600.) } else { (1120., 780.) };
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
                            size(px(w), px(h)),
                            cx,
                        ))),
                        ..Default::default()
                    },
                    move |_, cx| cx.new(|cx| shell::fixture(cx, vault, mode)),
                )
                .unwrap();
            window
                .update(cx, |_, window, _| {
                    window.resize(size(px(w), px(h)));
                    window.set_window_title("ME Identity Gallery — Synthetic")
                })
                .unwrap();
            cx.activate(true);
        });
}
