//! Continuing a previous result with the end user's choices, without the LLM.
//!
//! The previous result is input, so it is checked like input: versions, signature
//! (when the engine has a key), the plan's limits, coordinate ranges, and every
//! choice against what the result offered.

use std::collections::{HashMap, HashSet};

use waylingo_geo::{AnchorPoint, LatLng, Window};
use waylingo_plan::{Location, SCHEMA_VERSION, TripContext};
use waylingo_providers::{Candidate, GeocodeCandidate};

use super::{Engine, Outcome, Phase, Session, Slot, Step, WaylingoError};
use crate::choices::{Choices, LocationRole, StopPick};
use crate::result::{
    PlaceProps, Properties, RESULT_SCHEMA_VERSION, TripResult, UnresolvedReason, UserChoice,
};
use crate::select::{Choice, Placed};
use crate::signing;

/// (stop request id, occurrence).
type StopKey = (String, usize);

/// What the previous result offered for one stop, and what the end user had chosen.
struct PrevStop {
    /// Place ids the end user may choose: pick, alternatives, or a skipped stop's
    /// options.
    offered: HashSet<String>,
    /// The end user's earlier choice, if any.
    earlier: Option<StopPick>,
}

impl Engine {
    /// Continue a previous result with the end user's choices. The LLM is not
    /// called: the plan in `previous` is reused.
    ///
    /// * Only stop choices: one route call through the chosen places.
    /// * Another origin, waypoint or destination candidate: everything from the
    ///   base route on is redone, so stops anchored to the destination move with it.
    ///   Earlier and new stop choices are kept when the same place is found again;
    ///   otherwise the new best match is used and marked `user_choice: "dropped"`.
    ///
    /// Choices recorded in `previous` (picked or skipped stops) carry over. Any
    /// earlier result of the same trip may be passed, so an app can undo by resuming
    /// an older one. Only ids offered in `previous` are accepted. On a server that
    /// takes results back from clients, set [`crate::EngineConfig::signing_key`] so
    /// only results this engine produced are accepted. The route line is not read
    /// and not signed, so a client may send the result with the route feature's
    /// `coordinates` emptied or its `geometry` set to null; keep the feature itself.
    ///
    /// # Errors
    /// [`WaylingoError::IncompatibleResult`] when `previous` cannot be continued;
    /// [`WaylingoError::UnknownChoice`] when a choice names something not offered;
    /// [`WaylingoError::InvalidRequest`] when one stop gets two choices or one place
    /// is chosen for two stops.
    pub fn resume(
        &self,
        previous: &str,
        choices: Choices,
        request_id: Option<String>,
    ) -> Result<(Session, Step), WaylingoError> {
        let prev = TripResult::parse(previous)
            .map_err(|e| incompatible(format!("not a Waylingo result: {e}")))?;
        check_versions(&prev)?;
        if let Some(key) = &self.config.signing_key
            && !signing::verify(&prev, key)
        {
            return Err(incompatible(
                "the result is not signed by this engine (missing or wrong signature)",
            ));
        }
        let Some(plan) = prev.plan.clone() else {
            return Err(incompatible(
                "the result has no plan (the request was not understood); plan again",
            ));
        };
        plan.validate().map_err(|errs| {
            let list: Vec<String> = errs.iter().map(ToString::to_string).collect();
            incompatible(format!("the plan is invalid: {}", list.join("; ")))
        })?;
        let defaults = prev.search.defaults();
        defaults.validate().map_err(incompatible)?;

        let mut s = self.session(
            TripContext {
                defaults,
                ..TripContext::default()
            },
            request_id,
        );
        if prev.provider != s.adapter.kind() {
            return Err(incompatible(format!(
                "the result came from {:?}, this engine uses {:?}",
                prev.provider,
                s.adapter.kind()
            )));
        }
        let offered = previous_stops(&prev)?;
        let pins = merge_choices(&offered, &choices)?;
        s.resumed = true;
        s.plan = Some(plan);
        let location_changed = s.load_locations(&prev, &choices)?;

        let reqs = if location_changed {
            // A different place: search again from the base route; choices become
            // pins honoured where the same place is found again.
            s.pins = pins;
            let mut reqs = s.setup_requests();
            reqs.extend(s.base_route_request());
            s.phase = Phase::BaseRoute;
            reqs
        } else {
            // Stops are ordered against the waypoints by distance along the route;
            // without those distances the route would silently skip a waypoint.
            let n = s.locations.len();
            if (1..n.saturating_sub(1)).any(|i| s.along[i].is_none()) {
                return Err(incompatible("a waypoint has no along_m"));
            }
            s.load_slots(&prev)?;
            s.apply_stop_choices(&choices)?;
            s.drop_picked_alternatives();
            s.phase = Phase::FinalRoute;
            s.route_through_stops()
        };
        let step = if reqs.is_empty() {
            s.finish()
        } else {
            Step::NeedRequests { requests: reqs }
        };
        Ok((s, step))
    }
}

fn incompatible(why: impl Into<String>) -> WaylingoError {
    WaylingoError::IncompatibleResult(why.into())
}

fn check_versions(prev: &TripResult) -> Result<(), WaylingoError> {
    let major = |v: &str| v.split('.').next().unwrap_or("").to_string();
    if major(&prev.result_schema) != major(RESULT_SCHEMA_VERSION) {
        return Err(incompatible(format!(
            "result schema {} is not supported (expected {RESULT_SCHEMA_VERSION})",
            prev.result_schema
        )));
    }
    if prev.plan_schema != SCHEMA_VERSION {
        return Err(incompatible(format!(
            "plan schema {} is not supported (expected {SCHEMA_VERSION})",
            prev.plan_schema
        )));
    }
    Ok(())
}

/// The stops of the previous result: what each offered and the earlier choice.
fn previous_stops(prev: &TripResult) -> Result<HashMap<StopKey, PrevStop>, WaylingoError> {
    let mut out = HashMap::new();
    for f in &prev.features {
        let (key, stop) = match &f.properties {
            Properties::Stop(p) => (
                (p.stop_request_id.clone(), p.occurrence as usize),
                PrevStop {
                    offered: std::iter::once(&p.pick)
                        .chain(&p.alternatives)
                        .map(|x| x.provider_place_id.clone())
                        .collect(),
                    earlier: (p.user_choice == Some(UserChoice::Chosen)).then(|| StopPick::Place {
                        provider_place_id: p.pick.provider_place_id.clone(),
                    }),
                },
            ),
            Properties::UnresolvedStop(p) => (
                (p.stop_request_id.clone(), p.occurrence as usize),
                PrevStop {
                    offered: p
                        .options
                        .iter()
                        .map(|x| x.provider_place_id.clone())
                        .collect(),
                    earlier: (p.result == UnresolvedReason::SkippedByUser)
                        .then_some(StopPick::Skip),
                },
            ),
            _ => continue,
        };
        if out.insert(key.clone(), stop).is_some() {
            return Err(incompatible(format!(
                "stop {:?} occurrence {} appears twice",
                key.0, key.1
            )));
        }
    }
    Ok(out)
}

/// Earlier choices from the previous result, overlaid with this call's choices.
/// Every choice must name a stop and a place the previous result offered.
fn merge_choices(
    offered: &HashMap<StopKey, PrevStop>,
    choices: &Choices,
) -> Result<HashMap<StopKey, StopPick>, WaylingoError> {
    let mut pins: HashMap<StopKey, StopPick> = offered
        .iter()
        .filter_map(|(k, v)| v.earlier.clone().map(|p| (k.clone(), p)))
        .collect();
    let mut seen = HashSet::new();
    for c in &choices.stops {
        let key = (c.stop_request_id.clone(), c.occurrence as usize);
        if !seen.insert(key.clone()) {
            return Err(WaylingoError::InvalidRequest(format!(
                "stop {:?} occurrence {} has two choices",
                c.stop_request_id, c.occurrence
            )));
        }
        let Some(prev) = offered.get(&key) else {
            return Err(WaylingoError::UnknownChoice(format!(
                "no stop {:?} occurrence {} in the result",
                c.stop_request_id, c.occurrence
            )));
        };
        if let StopPick::Place { provider_place_id } = &c.pick
            && !prev.offered.contains(provider_place_id)
        {
            return Err(WaylingoError::UnknownChoice(format!(
                "{provider_place_id:?} is not an option for stop {:?} occurrence {}",
                c.stop_request_id, c.occurrence
            )));
        }
        pins.insert(key, c.pick.clone());
    }
    let mut places = HashSet::new();
    for p in pins.values() {
        if let StopPick::Place { provider_place_id } = p
            && !places.insert(provider_place_id.clone())
        {
            return Err(WaylingoError::InvalidRequest(format!(
                "{provider_place_id:?} is chosen for two stops"
            )));
        }
    }
    Ok(pins)
}

impl Session {
    /// Take locations from the previous result and apply location choices. A
    /// geocoded place's position always comes from its chosen candidate. Returns
    /// true when the trip must be searched again: a location changed, or the
    /// previous result never got as far as a route.
    fn load_locations(
        &mut self,
        prev: &TripResult,
        choices: &Choices,
    ) -> Result<bool, WaylingoError> {
        let locs = self.plan_locations();
        let n = locs.len();
        self.reset_locations(n);
        for f in &prev.features {
            let (i, props) = match &f.properties {
                Properties::Origin(p) => (0, p),
                Properties::Destination(p) => (n - 1, p),
                Properties::Waypoint(p) => match p.index {
                    Some(k) if (k as usize) + 2 < n => (k as usize + 1, p),
                    _ => return Err(incompatible("a waypoint feature has a bad index")),
                },
                _ => continue,
            };
            let mut candidates = Vec::with_capacity(props.candidates.len());
            for c in &props.candidates {
                candidates.push(GeocodeCandidate {
                    id: c.id.clone(),
                    label: c.label.clone(),
                    position: checked(c.position)?,
                    score: c.score,
                });
            }
            self.geocoded[i] = candidates;
            self.chosen[i].clone_from(&props.chosen_id);
            self.along[i] = props.along_m.filter(|m| m.is_finite() && *m >= 0.0);
        }
        for (i, l) in locs.iter().enumerate() {
            self.locations[i] = match l {
                // Given coordinates (current location, saved places) come from the
                // validated plan.
                Location::Coords { lat, lng, .. } => Some(LatLng::new(*lat, *lng)),
                Location::PlaceQuery { .. } => self.chosen_position(i),
            };
        }

        let mut changed = false;
        for c in &choices.locations {
            let i = match (c.role, c.index) {
                (LocationRole::Origin, _) => 0,
                (LocationRole::Destination, _) => n - 1,
                (LocationRole::Waypoint, Some(k)) if (k as usize) + 2 < n => k as usize + 1,
                (LocationRole::Waypoint, k) => {
                    return Err(WaylingoError::UnknownChoice(format!("no waypoint {k:?}")));
                }
            };
            if !self.geocoded[i].iter().any(|x| x.id == c.candidate_id) {
                return Err(WaylingoError::UnknownChoice(format!(
                    "{:?} has no candidate {:?}",
                    c.role, c.candidate_id
                )));
            }
            if self.chosen[i].as_deref() != Some(c.candidate_id.as_str()) {
                changed = true;
                self.chosen[i] = Some(c.candidate_id.clone());
                self.locations[i] = self.chosen_position(i);
            }
        }
        if self.locations.iter().any(Option::is_none) {
            return Err(incompatible(
                "a place in this result has no chosen candidate; plan again",
            ));
        }
        let had_route = prev
            .features
            .iter()
            .any(|f| matches!(f.properties, Properties::Route(_)));
        Ok(changed || !had_route)
    }

    /// The position of the chosen candidate of location `i`, if it is one of the
    /// candidates.
    fn chosen_position(&self, i: usize) -> Option<LatLng> {
        let id = self.chosen.get(i)?.as_ref()?;
        self.geocoded[i]
            .iter()
            .find(|c| &c.id == id)
            .map(|c| c.position)
    }

    /// Rebuild the stops of the previous result.
    fn load_slots(&mut self, prev: &TripResult) -> Result<(), WaylingoError> {
        let index: HashMap<String, usize> = self
            .plan
            .as_ref()
            .map(|p| {
                p.stop_requests
                    .iter()
                    .enumerate()
                    .map(|(i, r)| (r.id.clone(), i))
                    .collect()
            })
            .unwrap_or_default();
        for f in &prev.features {
            let (id, occurrence, anchor, window, outcome) = match &f.properties {
                Properties::Stop(p) => (
                    &p.stop_request_id,
                    p.occurrence,
                    p.anchor,
                    p.window,
                    Outcome::Found(Choice {
                        pick: placed_from(&p.pick)?,
                        beyond_radius: p.beyond_radius,
                        alternatives: p
                            .alternatives
                            .iter()
                            .map(placed_from)
                            .collect::<Result<_, _>>()?,
                        // A choice that was dropped earlier is no longer pending.
                        user_choice: p.user_choice.filter(|c| *c == UserChoice::Chosen),
                    }),
                ),
                Properties::UnresolvedStop(p) => (
                    &p.stop_request_id,
                    p.occurrence,
                    p.anchor,
                    p.window,
                    if p.result == UnresolvedReason::SkippedByUser {
                        Outcome::SkippedByUser {
                            options: p
                                .options
                                .iter()
                                .map(placed_from)
                                .collect::<Result<_, _>>()?,
                        }
                    } else {
                        Outcome::Carried {
                            result: p.result,
                            detail: p.detail.clone(),
                        }
                    },
                ),
                _ => continue,
            };
            let Some(&stop) = index.get(id) else {
                return Err(incompatible(format!("stop {id:?} is not in the plan")));
            };
            let occurrence = occurrence as usize;
            let anchor = match anchor {
                Some(a) => Some(AnchorPoint {
                    point: checked(a.position)?,
                    cum_m: a.cum_m,
                    cum_s: a.cum_s,
                    occurrence,
                }),
                None => None,
            };
            let mut s = Slot::new(stop, occurrence, anchor, outcome);
            s.window = window.map(|w| Window {
                start_m: w.start_m,
                end_m: w.end_m,
            });
            self.slots.push(s);
        }
        Ok(())
    }

    /// Apply this call's stop choices to the rebuilt stops. `merge_choices` has
    /// already checked each against what the result offered.
    fn apply_stop_choices(&mut self, choices: &Choices) -> Result<(), WaylingoError> {
        let ids: Vec<String> = self
            .plan
            .as_ref()
            .map(|p| p.stop_requests.iter().map(|r| r.id.clone()).collect())
            .unwrap_or_default();
        let radius_m = f64::from(self.ctx.defaults.anchor_radius_m);
        for c in &choices.stops {
            let Some(s) = self.slots.iter_mut().find(|s| {
                ids.get(s.stop) == Some(&c.stop_request_id) && s.occurrence == c.occurrence as usize
            }) else {
                return Err(WaylingoError::UnknownChoice(format!(
                    "stop {:?} has no occurrence {}",
                    c.stop_request_id, c.occurrence
                )));
            };
            let outcome = std::mem::replace(&mut s.outcome, Outcome::Waiting);
            s.outcome = match (outcome, &c.pick) {
                (Outcome::Found(ch), StopPick::Skip) => Outcome::SkippedByUser {
                    options: std::iter::once(ch.pick).chain(ch.alternatives).collect(),
                },
                (keep @ Outcome::SkippedByUser { .. }, StopPick::Skip) => keep,
                (Outcome::Found(ch), StopPick::Place { provider_place_id }) => {
                    let all = std::iter::once(ch.pick).chain(ch.alternatives);
                    Outcome::Found(choice_of(all, provider_place_id, radius_m)?)
                }
                (Outcome::SkippedByUser { options }, StopPick::Place { provider_place_id }) => {
                    Outcome::Found(choice_of(options.into_iter(), provider_place_id, radius_m)?)
                }
                (other, StopPick::Skip) => other,
                (_, StopPick::Place { .. }) => {
                    return Err(WaylingoError::UnknownChoice(format!(
                        "stop {:?} occurrence {} has no places to choose from",
                        c.stop_request_id, c.occurrence
                    )));
                }
            };
        }
        // One place cannot serve two stops, whoever picked it.
        let mut seen = HashSet::new();
        for s in &self.slots {
            if let Outcome::Found(c) = &s.outcome
                && !seen.insert(c.pick.c.provider_place_id.clone())
            {
                return Err(WaylingoError::InvalidRequest(format!(
                    "{:?} is chosen for two stops",
                    c.pick.c.provider_place_id
                )));
            }
        }
        Ok(())
    }
}

/// Make the place with `id` the pick; the rest, in order, are the alternatives.
fn choice_of(
    places: impl Iterator<Item = Placed>,
    id: &str,
    radius_m: f64,
) -> Result<Choice, WaylingoError> {
    let mut rest: Vec<Placed> = places.collect();
    let Some(k) = rest.iter().position(|p| p.c.provider_place_id == id) else {
        return Err(WaylingoError::UnknownChoice(format!(
            "{id:?} is not an option"
        )));
    };
    let pick = rest.remove(k);
    Ok(Choice {
        beyond_radius: pick.distance_m > radius_m,
        pick,
        alternatives: rest,
        user_choice: Some(UserChoice::Chosen),
    })
}

/// GeoJSON `[lng, lat]` to a coordinate, rejecting values off the globe.
fn checked(p: [f64; 2]) -> Result<LatLng, WaylingoError> {
    let ll = LatLng::new(p[1], p[0]);
    if ll.is_valid() {
        Ok(ll)
    } else {
        Err(incompatible(format!(
            "position {p:?} is not a valid coordinate"
        )))
    }
}

fn placed_from(p: &PlaceProps) -> Result<Placed, WaylingoError> {
    let finite = |v: f64| v.is_finite() && v >= 0.0;
    if !finite(p.along_m) || !finite(p.distance_from_anchor_m) {
        return Err(incompatible(format!(
            "place {:?} has an invalid distance",
            p.provider_place_id
        )));
    }
    Ok(Placed {
        c: Candidate {
            provider_place_id: p.provider_place_id.clone(),
            name: p.name.clone(),
            position: checked(p.position)?,
            address: p.address.clone(),
            brand: p.brand.clone(),
            detour_s: p.detour_s,
            detour_m: p.detour_m,
        },
        along_m: p.along_m,
        distance_m: p.distance_from_anchor_m,
    })
}
