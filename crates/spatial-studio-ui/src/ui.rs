//! WASM shell (Layer E, docs/17 D5): thin dioxus views over the
//! platform logic — the onboarding state machine, the 90-day ROI
//! ledger and the EU AI Act scorecard. All behaviour lives in the
//! sibling modules; this file only binds them to RSX.

use dioxus::prelude::*;

use crate::ledger::{LedgerWindow, RoiChannels, validate_window};
use crate::onboarding::{SiteFacts, Stage, advance, check_stage};
use crate::scorecard::{CheckId, audit};

/// Call from the app crate's `main` to boot the control-room shell.
pub fn mount() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let stage: Signal<Stage> = use_signal(|| Stage::PlugIn);
    let facts: Signal<SiteFacts> = use_signal(|| SiteFacts {
        jetson_engines_ready: false,
        tpm_handle_ready: false,
        espnow_lock: None,
        discovery_cameras: 0,
        draft: None,
    });
    let stage_error: Signal<Option<String>> = use_signal(|| None);

    rsx! {
        div { class: "shell",
            h1 { "Shado Spatial Studio" }
            OnboardingPanel { stage, facts, stage_error }
            LedgerPanel {}
            ScorePanel {}
        }
    }
}

#[component]
fn OnboardingPanel(
    stage: Signal<Stage>,
    facts: Signal<SiteFacts>,
    stage_error: Signal<Option<String>>,
) -> Element {
    let stage_name = match *stage.read() {
        Stage::PlugIn => "1 · Plug in",
        Stage::Discovery => "2 · Discovery",
        Stage::ChatToContract => "3 · Chat to contract",
        Stage::Operation => "4 · Operation",
    };
    let toggle_facts = move |_| {
        let mut f = facts.write();
        f.jetson_engines_ready = !f.jetson_engines_ready;
        if f.jetson_engines_ready {
            f.tpm_handle_ready = true;
            f.espnow_lock = Some((2412, 1000));
            f.discovery_cameras = 3;
            f.draft = None;
        }
    };
    let advance_stage = move |_| {
        let current = *stage.read();
        let cameras = facts.read().discovery_cameras;
        match advance(current) {
            Ok(next) => {
                stage.set(next);
                stage_error.set(None);
                if next == Stage::Discovery {
                    facts.write().discovery_cameras = cameras.max(1);
                }
            }
            Err(e) => stage_error.set(Some(e.to_string())),
        }
    };
    let check = check_stage(*stage.read(), &facts.read()).is_ok();
    rsx! {
        section {
            h2 { "Onboarding · {stage_name}" }
            button { onclick: toggle_facts, "toggle site facts" }
            button { disabled: !check, onclick: advance_stage, "advance stage" }
            match stage_error.read().as_deref() {
                Some(msg) => rsx! { p { class: "err", "{msg}" } },
                None => rsx! { p { "stage checks pass" } },
            }
        }
    }
}

#[component]
fn LedgerPanel() -> Element {
    let channels = RoiChannels {
        revenue_uplift: 1200.0,
        labor_saved: 400.0,
        losses_avoided: 250.0,
        energy_cost: 30.0,
    };
    let window = LedgerWindow {
        start_day: 1,
        end_day: 90,
    };
    let net = channels.net(90.0);
    let window_ok = validate_window(window.end_day - window.start_day + 1).is_ok();
    rsx! {
        section {
            h2 { "90-day ROI ledger" }
            p { "window valid: {window_ok}" }
            p { class: "net", "net: {net:.0} USD" }
        }
    }
}

#[component]
fn ScorePanel() -> Element {
    let contract = monitoring_contract::templates::manufacturing_line();
    match contract {
        Err(e) => rsx! {
            section {
                h2 { "EU AI Act scorecard" }
                p { class: "err", "no vertical loaded: {e}" }
            }
        },
        Ok(contract) => {
            let sc = audit(&contract, Some(200), true, true);
            let done = sc.done_checks();
            let total = sc.results.len();
            rsx! {
                section {
                    h2 { "EU AI Act scorecard" }
                    p { class: "score", "{done}/{total} checks green" }
                    ul {
                        for result in sc.results {
                            CheckRow { key: "{result.check:?}", check: result.check, passed: result.passed }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn CheckRow(check: CheckId, passed: bool) -> Element {
    let word: &str = if passed { "green" } else { "red" };
    rsx! { li { "{check:?} {word}" } }
}
