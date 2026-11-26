//! `Engine::parse`: the LLM step only, no map calls.

mod common;

use common::*;
use waylingo::{CredentialSlot, ParseResult, ParseStatus, Step};

fn parse_with(llm: &dyn Fn(usize) -> (u16, String)) -> (ParseResult, Host<'_>) {
    let (mut session, mut step) = engine().parse(request()).unwrap();
    let mut h = host(llm);
    let doc = loop {
        match step {
            Step::Done { geojson } => break ParseResult::parse(&geojson).unwrap(),
            Step::NeedRequests { requests } => {
                let resps = requests.iter().map(|r| h.answer(r)).collect();
                step = session.step(resps).unwrap();
            }
        }
    };
    (doc, h)
}

#[test]
fn parse_returns_the_plan_without_map_calls() {
    let (doc, h) = parse_with(&good_llm);
    assert_eq!(doc.status, ParseStatus::Parsed);
    assert_eq!(doc.error, None);
    assert_eq!(doc.request_id.as_deref(), Some("req-1"));
    assert_eq!(doc.usage.llm_calls, 1);
    assert_eq!(doc.usage.map_calls, 0);
    assert!(h.log.iter().all(|r| r.credential == CredentialSlot::Llm));

    // The same plan, and the same hash, as a full run of the same request.
    let full = host(&good_llm).run();
    let plan = doc.plan.unwrap();
    assert_eq!(serde_json::to_value(&plan).unwrap(), full["plan"]);
    assert_eq!(doc.plan_hash.as_deref(), full["plan_hash"].as_str());
}

#[test]
fn parse_repairs_like_a_full_run() {
    let llm = |n: usize| {
        if n == 0 {
            (200, "not json".to_string())
        } else {
            (200, LLM_PLAN.to_string())
        }
    };
    let (doc, _) = parse_with(&llm);
    assert_eq!(doc.status, ParseStatus::Parsed);
    assert_eq!(doc.usage.llm_calls, 2);
}

#[test]
fn parse_failure_has_a_code_and_no_plan() {
    let llm = |_: usize| (401, String::new());
    let (doc, _) = parse_with(&llm);
    assert_eq!(doc.status, ParseStatus::Failed);
    assert_eq!(doc.plan, None);
    assert_eq!(doc.plan_hash, None);
    assert_eq!(
        serde_json::to_value(doc.error.unwrap().code).unwrap(),
        "LLM_AUTH"
    );
}

#[test]
fn parse_session_finishes_once() {
    let (mut session, step) = engine().parse(request()).unwrap();
    let Step::NeedRequests { requests } = step else {
        panic!("expected the LLM request")
    };
    let mut h = host(&good_llm);
    let resps = requests.iter().map(|r| h.answer(r)).collect();
    assert!(matches!(session.step(resps).unwrap(), Step::Done { .. }));
    assert!(session.step(vec![]).is_err());
}
