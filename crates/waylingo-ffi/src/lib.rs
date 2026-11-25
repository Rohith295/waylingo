//! Waylingo for Python, Swift and Kotlin.
//!
//! One object, [`Waylingo`], made once per app or server process:
//!
//! * `plan(trip)`: the end user's request in, a GeoJSON result out.
//! * `reroute(previous_geojson, choices)`: after the end user picks another stop or
//!   place, the new route. The LLM is not called again.
//!
//! Both are async; `plan_blocking` and `reroute_blocking` are for code without an
//! event loop. Keys are passed to the constructor and never stored in a record, so
//! they cannot show up in a printed config.

mod choices;
mod config;
mod error;
mod result;
mod trip;

use std::sync::{Arc, LazyLock};

use waylingo::{Defaults, Engine, SigningKey};
use waylingo_reqwest::{AsyncHost, Keys};

pub use choices::{Choices, LocationChoice, LocationRole, StopChoice, StopPick};
pub use config::{LlmBackend, MapProvider, SearchSettings, WaylingoConfig};
pub use error::WaylingoError;
pub use result::{
    Candidate, Location, Place, Stop, TripOutput, TripStatus, UnresolvedStop, UserChoice,
};
pub use trip::{AvoidKind, ConnectorKind, EvSpec, LatLng, SavedPlace, TripInput};

uniffi::setup_scaffolding!("waylingo");

/// Runs the blocking variants. One runtime for the process, created on first use.
static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("waylingo")
        .build()
        .expect("the Waylingo runtime starts")
});

#[derive(uniffi::Object)]
pub struct Waylingo {
    engine: Engine,
    host: AsyncHost,
    defaults: Defaults,
}

impl std::fmt::Debug for Waylingo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Waylingo").finish_non_exhaustive()
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Waylingo {
    /// `map_key` is the TomTom or HERE key (or your proxy's token, possibly empty,
    /// with `map_base_url`). `llm_key` is the LLM provider's key; it may be left out
    /// for a local server or when the backend has a `base_url` proxy.
    /// `signing_key`, when set, signs results and makes `reroute` accept only results
    /// this instance signed: use it on servers that take results back from clients.
    ///
    /// Create one instance per process and reuse it; create it after forking.
    #[uniffi::constructor(default(llm_key = None, signing_key = None))]
    pub fn new(
        config: WaylingoConfig,
        map_key: String,
        llm_key: Option<String>,
        signing_key: Option<Vec<u8>>,
    ) -> Result<Arc<Self>, WaylingoError> {
        config.check_keys(&map_key, llm_key.as_deref())?;
        let mut engine_config = config.engine_config()?;
        engine_config.signing_key = signing_key.map(SigningKey::new);
        let defaults = config.defaults();
        defaults
            .validate()
            .map_err(|message| WaylingoError::InvalidConfig { message })?;
        let host = AsyncHost::new(
            Keys {
                llm: llm_key.unwrap_or_default(),
                map: map_key,
            },
            config.deadline()?,
            config.map_spacing()?,
        )?;
        Ok(Arc::new(Self {
            engine: Engine::new(engine_config),
            host,
            defaults,
        }))
    }

    /// Plan a trip from the end user's request.
    pub async fn plan(&self, trip: TripInput) -> Result<TripOutput, WaylingoError> {
        let geojson = self
            .host
            .plan(&self.engine, trip.request(self.defaults))
            .await?;
        Ok(TripOutput::from_geojson(geojson))
    }

    /// Continue a previous result after the end user's choices. `previous_geojson`
    /// is a result from this instance (the route line may be removed to save space).
    #[uniffi::method(default(request_id = None))]
    pub async fn reroute(
        &self,
        previous_geojson: String,
        choices: Choices,
        request_id: Option<String>,
    ) -> Result<TripOutput, WaylingoError> {
        let geojson = self
            .host
            .resume(&self.engine, &previous_geojson, choices.into(), request_id)
            .await?;
        Ok(TripOutput::from_geojson(geojson))
    }
}

#[uniffi::export]
impl Waylingo {
    /// [`Waylingo::plan`] for code without an event loop. Blocks the calling thread
    /// for up to `timeout_ms`: never call it on a UI thread.
    pub fn plan_blocking(&self, trip: TripInput) -> Result<TripOutput, WaylingoError> {
        RUNTIME.block_on(self.plan(trip))
    }

    /// [`Waylingo::reroute`] for code without an event loop. Blocks the calling
    /// thread: never call it on a UI thread.
    #[uniffi::method(default(request_id = None))]
    pub fn reroute_blocking(
        &self,
        previous_geojson: String,
        choices: Choices,
        request_id: Option<String>,
    ) -> Result<TripOutput, WaylingoError> {
        RUNTIME.block_on(self.reroute(previous_geojson, choices, request_id))
    }
}

/// The version of the Waylingo library.
#[uniffi::export]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
