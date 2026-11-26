//! A blocking host: one request at a time, map calls spaced for the provider's rate
//! limit. Simple to use from scripts and command-line tools.

use std::time::Duration;

use waylingo::{
    Choices, CredentialSlot, Engine, HttpRequest, HttpResponse, Session, Step, TripRequest,
};

use crate::wire::{prepare, response, transport_failure};
use crate::{DriveError, Keys, MAP_SPACING, USER_AGENT};

/// Run one query to completion and return the GeoJSON.
///
/// # Errors
/// Engine misuse or a failure to create the HTTP client. Provider and LLM failures
/// are reported inside the GeoJSON, not as errors.
pub fn drive(engine: &Engine, request: TripRequest, keys: &Keys) -> Result<String, DriveError> {
    let (session, step) = engine.start(request)?;
    run(session, step, keys)
}

/// Run only the LLM step (see [`Engine::parse`]) and return the
/// [`waylingo::ParseResult`] JSON. Makes no map call.
///
/// # Errors
/// As [`drive`].
pub fn drive_parse(
    engine: &Engine,
    request: TripRequest,
    keys: &Keys,
) -> Result<String, DriveError> {
    let (session, step) = engine.parse(request)?;
    run(session, step, keys)
}

/// Continue a previous result with the end user's choices (see
/// [`Engine::resume`]) and return the new GeoJSON.
///
/// # Errors
/// As [`drive`], plus the errors of [`Engine::resume`].
pub fn drive_resume(
    engine: &Engine,
    previous: &str,
    choices: Choices,
    request_id: Option<String>,
    keys: &Keys,
) -> Result<String, DriveError> {
    let (session, step) = engine.resume(previous, choices, request_id)?;
    run(session, step, keys)
}

fn run(mut session: Session, mut step: Step, keys: &Keys) -> Result<String, DriveError> {
    let client = crate::tls_blocking(reqwest::blocking::Client::builder().user_agent(USER_AGENT))
        .build()
        .map_err(|_| DriveError::Client)?;
    loop {
        match step {
            Step::Done { geojson } => return Ok(geojson),
            Step::NeedRequests { requests } => {
                let responses = requests.iter().map(|r| send(&client, r, keys)).collect();
                step = session.step(responses)?;
            }
        }
    }
}

fn send(client: &reqwest::blocking::Client, req: &HttpRequest, keys: &Keys) -> HttpResponse {
    let mut wait = Duration::from_millis(u64::from(req.not_before_ms));
    if req.credential == CredentialSlot::Map {
        wait += MAP_SPACING;
    }
    std::thread::sleep(wait);

    let p = prepare(req, keys);
    let mut b = if p.post {
        client.post(&p.url)
    } else {
        client.get(&p.url)
    }
    .timeout(p.timeout);
    for (name, value) in &p.headers {
        b = b.header(name, value);
    }
    if let Some(body) = p.body {
        b = b.body(body);
    }
    match b.send() {
        Ok(r) => {
            let status = r.status().as_u16();
            let retry_after = r
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .map(String::from);
            let body = r.bytes().map(|b| b.to_vec()).unwrap_or_default();
            response(&req.id, status, retry_after, body)
        }
        Err(e) => transport_failure(&req.id, e.is_timeout()),
    }
}
