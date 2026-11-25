//! An async host for apps and servers: one long-lived HTTP client, the requests of
//! each batch sent in parallel, map calls paced by a gate shared by every query on
//! the host (so concurrent queries together stay under the provider's rate limit),
//! and an overall deadline per query. Dropping a query's future cancels it safely:
//! the engine holds no I/O.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use tokio::sync::Mutex;
use waylingo::{
    Choices, CredentialSlot, Engine, HttpRequest, HttpResponse, Session, Step, TripRequest,
};

use crate::wire::{prepare, response, transport_failure};
use crate::{DriveError, Keys, USER_AGENT};

/// Sends a query's HTTP requests. Create one per app or server process and reuse it.
pub struct AsyncHost {
    client: reqwest::Client,
    keys: Keys,
    /// Earliest time the next map request may start.
    map_gate: Arc<Mutex<Instant>>,
    /// Least time between map request starts; zero for no pacing.
    map_spacing: Duration,
    deadline: Duration,
}

impl std::fmt::Debug for AsyncHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncHost")
            .field("keys", &self.keys)
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

impl AsyncHost {
    /// `deadline` bounds one whole query (all its LLM and map calls), including
    /// time spent waiting for a map request slot. `map_spacing` is the least time
    /// between map request starts across every query on this host (for example
    /// [`MAP_SPACING`] for 4.5 per second); zero turns pacing off.
    ///
    /// # Errors
    /// [`DriveError::Client`] when the HTTP client cannot be created.
    pub fn new(keys: Keys, deadline: Duration, map_spacing: Duration) -> Result<Self, DriveError> {
        let client = crate::tls(reqwest::Client::builder().user_agent(USER_AGENT))
            .build()
            .map_err(|_| DriveError::Client)?;
        Ok(Self {
            client,
            keys,
            map_gate: Arc::new(Mutex::new(Instant::now())),
            map_spacing,
            deadline,
        })
    }

    /// Run one query to completion and return the GeoJSON.
    ///
    /// # Errors
    /// Engine misuse, or [`DriveError::Timeout`] when the deadline passes. Provider
    /// and LLM failures are reported inside the GeoJSON.
    pub async fn plan(&self, engine: &Engine, request: TripRequest) -> Result<String, DriveError> {
        let (session, step) = engine.start(request)?;
        self.run(session, step).await
    }

    /// Continue a previous result with the end user's choices (see
    /// [`Engine::resume`]).
    ///
    /// # Errors
    /// As [`AsyncHost::plan`], plus the errors of [`Engine::resume`].
    pub async fn resume(
        &self,
        engine: &Engine,
        previous: &str,
        choices: Choices,
        request_id: Option<String>,
    ) -> Result<String, DriveError> {
        let (session, step) = engine.resume(previous, choices, request_id)?;
        self.run(session, step).await
    }

    async fn run(&self, session: Session, step: Step) -> Result<String, DriveError> {
        tokio::time::timeout(self.deadline, self.drive(session, step))
            .await
            .map_err(|_| DriveError::Timeout)?
    }

    async fn drive(&self, mut session: Session, mut step: Step) -> Result<String, DriveError> {
        loop {
            match step {
                Step::Done { geojson } => return Ok(geojson),
                Step::NeedRequests { requests } => {
                    let responses = join_all(requests.iter().map(|r| self.send(r))).await;
                    step = session.step(responses)?;
                }
            }
        }
    }

    /// Wait for this map request's turn: requests start at least `MAP_SPACING` apart.
    async fn pace(&self) {
        if self.map_spacing.is_zero() {
            return;
        }
        let wait = {
            let mut next = self.map_gate.lock().await;
            let now = Instant::now();
            let start = (*next).max(now);
            *next = start + self.map_spacing;
            start - now
        };
        tokio::time::sleep(wait).await;
    }

    async fn send(&self, req: &HttpRequest) -> HttpResponse {
        if req.not_before_ms > 0 {
            tokio::time::sleep(Duration::from_millis(u64::from(req.not_before_ms))).await;
        }
        if req.credential == CredentialSlot::Map {
            self.pace().await;
        }
        let p = prepare(req, &self.keys);
        let mut b = if p.post {
            self.client.post(&p.url)
        } else {
            self.client.get(&p.url)
        }
        .timeout(p.timeout);
        for (name, value) in &p.headers {
            b = b.header(name, value);
        }
        if let Some(body) = p.body {
            b = b.body(body);
        }
        match b.send().await {
            Ok(r) => {
                let status = r.status().as_u16();
                let retry_after = r
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);
                let body = r.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
                response(&req.id, status, retry_after, body)
            }
            Err(e) => transport_failure(&req.id, e.is_timeout()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAP_SPACING;

    #[tokio::test(start_paused = true)]
    async fn map_requests_are_spaced_across_queries() {
        let host = AsyncHost::new(
            Keys {
                llm: String::new(),
                map: "k".into(),
            },
            Duration::from_secs(60),
            MAP_SPACING,
        )
        .unwrap();
        let t0 = tokio::time::Instant::now();
        // Three requests from "different queries" at once still go out spaced.
        futures_util::future::join3(host.pace(), host.pace(), host.pace()).await;
        assert!(t0.elapsed() >= MAP_SPACING * 2, "{:?}", t0.elapsed());
    }
}
