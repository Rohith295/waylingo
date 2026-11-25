//! Tier 2: deterministic compilation of a [`SurfaceIntent`] into a canonical [`Plan`].

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{
    Amount, Anchor, Avoid, Category, Connector, Departure, Direction, EvFilter, Location, Plan,
    Quantity, Reference, SCHEMA_VERSION, StopRequest, SurfaceAnchor, SurfaceIntent, SurfaceStop,
    TripContext, What,
};

use crate::validate::{MAX_DISTANCE_M, MAX_DURATION_S, MAX_STOPS, MAX_TOP_N};

/// One validation problem, phrased so it can go straight into a repair prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{pointer}: {reason}")]
pub struct PlanError {
    /// JSON Pointer into the [`SurfaceIntent`], e.g. `/stops/0/anchor/amount/unit`.
    pub pointer: String,
    pub reason: String,
    /// Allowed values, when the field has a fixed set.
    pub allowed: Vec<String>,
    /// Set when the problem is missing context that only the host or the end user
    /// can supply. Such errors are not sent back to the LLM: it could only invent
    /// the missing value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs: Option<Need>,
}

/// Something missing that the LLM cannot supply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Need {
    /// Nothing was asked for: no destination, waypoint or stop. The text was not a
    /// trip request, so the end user must rephrase.
    Request,
    /// The trip starts at (or refers to) the current location, but the host gave none.
    CurrentLocation,
    /// No destination was named and the host gave no active route.
    Destination,
}

impl PlanError {
    pub(crate) fn new(pointer: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            pointer: pointer.into(),
            reason: reason.into(),
            allowed: Vec::new(),
            needs: None,
        }
    }
    fn needing(mut self, need: Need) -> Self {
        self.needs = Some(need);
        self
    }
    fn with_allowed<'a>(mut self, allowed: impl IntoIterator<Item = &'a str>) -> Self {
        self.allowed = allowed.into_iter().map(String::from).collect();
        self
    }
}

/// Compile and validate. Returns every problem found, not just the first.
///
/// # Errors
/// A non-empty list of [`PlanError`]s when the intent cannot become a valid plan.
pub fn compile(s: &SurfaceIntent, ctx: &TripContext) -> Result<Plan, Vec<PlanError>> {
    if let Some(e) = &s.error {
        return Err(vec![PlanError::new(
            "/error",
            format!("the request was reported as {e:?}"),
        )]);
    }
    if s.destination.is_none() && s.waypoints.is_empty() && s.stops.is_empty() {
        return Err(vec![
            PlanError::new(
                "",
                "nothing was asked for: no destination, waypoint or stop",
            )
            .needing(Need::Request),
        ]);
    }
    let mut errs = Vec::new();

    let origin = place(s.origin.as_deref(), "/origin", ctx, &mut errs, Role::Origin);
    let destination = place(
        s.destination.as_deref(),
        "/destination",
        ctx,
        &mut errs,
        Role::Destination,
    );
    let waypoints: Vec<Location> = s
        .waypoints
        .iter()
        .enumerate()
        .filter_map(|(i, w)| {
            place(
                Some(w),
                &format!("/waypoints/{i}"),
                ctx,
                &mut errs,
                Role::Waypoint,
            )
        })
        .collect();

    let mut avoid = ctx.avoid.clone();
    for (i, a) in s.avoid.iter().enumerate() {
        match lookup(&Avoid::ALL, a) {
            Some(v) if !avoid.contains(&v) => avoid.push(v),
            Some(_) => {}
            None => errs.push(
                PlanError::new(format!("/avoid/{i}"), format!("unknown value {a:?}"))
                    .with_allowed(Avoid::ALL.map(|(n, _)| n)),
            ),
        }
    }

    let max_detour_s = s
        .max_detour
        .as_ref()
        .and_then(|q| duration_s(q, "/max_detour", &mut errs))
        .map(|v| v.round().max(1.0) as u32)
        .or(ctx.max_detour_s)
        .unwrap_or(ctx.defaults.max_detour_s);

    if s.stops.len() > MAX_STOPS {
        errs.push(PlanError::new(
            "/stops",
            format!("at most {MAX_STOPS} stops are supported"),
        ));
    }

    let mut ids = HashSet::new();
    let mut stops = Vec::with_capacity(s.stops.len());
    for (i, st) in s.stops.iter().enumerate() {
        if let Some(stop) = stop(st, i, s.waypoints.len(), ctx, &mut errs) {
            if !ids.insert(stop.id.clone()) {
                errs.push(PlanError::new(
                    format!("/stops/{i}/id"),
                    format!("duplicate id {:?}", stop.id),
                ));
            }
            stops.push(stop);
        }
    }

    let (Some(origin), Some(destination)) = (origin, destination) else {
        return Err(errs);
    };
    if !errs.is_empty() {
        return Err(errs);
    }
    let plan = Plan {
        schema_version: SCHEMA_VERSION.to_string(),
        origin,
        destination,
        waypoints,
        departure: Departure {
            time: ctx.departure_time.clone(),
            timezone: ctx.timezone.clone(),
        },
        vehicle: ctx.vehicle.clone().unwrap_or_default(),
        avoid,
        max_detour_s,
        stop_requests: stops,
    };
    // The same limits apply to plans read back from a previous result.
    plan.validate()?;
    Ok(plan)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Origin,
    Destination,
    Waypoint,
}

fn current(
    ctx: &TripContext,
    ptr: &str,
    errs: &mut Vec<PlanError>,
    reason: &str,
) -> Option<Location> {
    match ctx.current_location {
        Some(c) if c.is_valid() => Some(Location::Coords {
            lat: c.lat,
            lng: c.lng,
            label: Some("current_location".into()),
        }),
        _ => {
            errs.push(PlanError::new(ptr, reason).needing(Need::CurrentLocation));
            None
        }
    }
}

fn place(
    v: Option<&str>,
    ptr: &str,
    ctx: &TripContext,
    errs: &mut Vec<PlanError>,
    role: Role,
) -> Option<Location> {
    let v = v.map(str::trim).filter(|t| !t.is_empty());
    match (v, role) {
        (None | Some("current_location"), Role::Origin) => current(
            ctx,
            ptr,
            errs,
            "the start is the current location, but the host did not provide one",
        ),
        (None, Role::Destination) => match ctx.active_destination {
            Some(c) if c.is_valid() => Some(Location::Coords {
                lat: c.lat,
                lng: c.lng,
                label: Some("active_destination".into()),
            }),
            _ => {
                errs.push(
                    PlanError::new(
                        ptr,
                        "no destination was named and the host passed no active route",
                    )
                    .needing(Need::Destination),
                );
                None
            }
        },
        (None, Role::Waypoint) => {
            errs.push(PlanError::new(ptr, "waypoint must not be empty"));
            None
        }
        (Some("current_location"), _) => current(
            ctx,
            ptr,
            errs,
            "current_location was used, but the host did not provide one",
        ),
        (Some(t), _) => match t.strip_prefix("saved:") {
            None => Some(Location::PlaceQuery {
                text: t.to_string(),
            }),
            Some(name) => {
                let found = ctx
                    .saved_places
                    .iter()
                    .find(|p| p.name.eq_ignore_ascii_case(name.trim()));
                match found {
                    Some(p) if p.location.is_valid() => Some(Location::Coords {
                        lat: p.location.lat,
                        lng: p.location.lng,
                        label: Some(p.name.clone()),
                    }),
                    _ => {
                        let mut e = PlanError::new(ptr, format!("no saved place named {name:?}"));
                        e.allowed = ctx
                            .saved_places
                            .iter()
                            .map(|p| format!("saved:{}", p.name))
                            .collect();
                        errs.push(e);
                        None
                    }
                }
            }
        },
    }
}

fn stop(
    st: &SurfaceStop,
    i: usize,
    n_waypoints: usize,
    ctx: &TripContext,
    errs: &mut Vec<PlanError>,
) -> Option<StopRequest> {
    let base = format!("/stops/{i}");
    let before = errs.len();

    let what = &st.what;
    let category = what.category.as_deref().and_then(|c| {
        let found = lookup(&Category::ALL, c);
        if found.is_none() {
            errs.push(
                PlanError::new(
                    format!("{base}/what/category"),
                    format!("unknown category {c:?}; use text for other places"),
                )
                .with_allowed(Category::ALL.map(|(n, _)| n)),
            );
        }
        found
    });
    let text = non_empty(what.text.as_deref());
    let brand = non_empty(what.brand.as_deref());
    if what.category.is_none() && text.is_none() && brand.is_none() {
        errs.push(PlanError::new(
            format!("{base}/what"),
            "give a category, a text, or a brand",
        ));
    }

    let mut connectors = Vec::new();
    for (j, c) in what.connector_types.iter().enumerate() {
        match lookup(&Connector::ALL, c) {
            Some(v) if !connectors.contains(&v) => connectors.push(v),
            Some(_) => {}
            None => errs.push(
                PlanError::new(
                    format!("{base}/what/connector_types/{j}"),
                    format!("unknown connector {c:?}"),
                )
                .with_allowed(Connector::ALL.map(|(n, _)| n)),
            ),
        }
    }
    if let Some(kw) = what.min_power_kw
        && !(kw.is_finite() && kw > 0.0 && kw <= 1000.0)
    {
        errs.push(PlanError::new(
            format!("{base}/what/min_power_kw"),
            "must be between 0 and 1000 kW",
        ));
    }
    // EV filters: from the stop, else inherited from the vehicle for charging stops.
    let ev = if what.min_power_kw.is_some() || !connectors.is_empty() {
        let inherited = ctx.vehicle.as_ref().and_then(|v| v.ev.as_ref());
        let connector_types = if connectors.is_empty() {
            inherited
                .map(|e| e.connector_types.clone())
                .unwrap_or_default()
        } else {
            connectors
        };
        Some(EvFilter {
            min_power_kw: what.min_power_kw,
            connector_types,
        })
    } else if category == Some(Category::EvCharging) {
        ctx.vehicle.as_ref().and_then(|v| v.ev.clone())
    } else {
        None
    };

    let anchor = anchor(&st.anchor, &base, n_waypoints, ctx, errs);

    let top_n = st.top_n.unwrap_or(ctx.defaults.top_n);
    if !(1..=MAX_TOP_N).contains(&top_n) {
        errs.push(PlanError::new(
            format!("{base}/top_n"),
            format!("must be between 1 and {MAX_TOP_N}"),
        ));
    }

    let id = non_empty(st.id.as_deref()).unwrap_or_else(|| format!("stop_{i}"));

    if errs.len() > before {
        return None;
    }
    Some(StopRequest {
        id,
        what: What {
            category,
            text,
            brand,
            ev,
        },
        anchor: anchor?,
        top_n,
    })
}

fn anchor(
    a: &SurfaceAnchor,
    base: &str,
    n_waypoints: usize,
    ctx: &TripContext,
    errs: &mut Vec<PlanError>,
) -> Option<Anchor> {
    let ptr = format!("{base}/anchor");
    match a {
        SurfaceAnchor::Offset {
            from,
            direction,
            amount,
        } => {
            let from_ref = reference(from, &ptr, n_waypoints, errs);
            let dir = match direction.trim() {
                "after" => Some(Direction::After),
                "before" => Some(Direction::Before),
                other => {
                    errs.push(
                        PlanError::new(
                            format!("{ptr}/direction"),
                            format!("unknown direction {other:?}"),
                        )
                        .with_allowed(["after", "before"]),
                    );
                    None
                }
            };
            let amt = amount_of(amount, &format!("{ptr}/amount"), errs);
            match (from_ref, dir) {
                (Some(Reference::Origin), Some(Direction::Before)) => {
                    errs.push(PlanError::new(
                        format!("{ptr}/direction"),
                        "a stop cannot be before the origin",
                    ));
                    None
                }
                (Some(Reference::Destination), Some(Direction::After)) => {
                    errs.push(PlanError::new(
                        format!("{ptr}/direction"),
                        "a stop cannot be after the destination",
                    ));
                    None
                }
                (Some(from), Some(direction)) => Some(Anchor::Offset {
                    from,
                    direction,
                    amount: amt?,
                }),
                _ => None,
            }
        }
        SurfaceAnchor::Every { amount } => {
            let interval = amount_of(amount, &format!("{ptr}/amount"), errs)?;
            Some(Anchor::Every {
                interval,
                window_pct: ctx.defaults.every_window_pct,
            })
        }
        SurfaceAnchor::Fraction { value } => {
            if value.is_finite() && *value > 0.0 && *value < 1.0 {
                Some(Anchor::Fraction { value: *value })
            } else {
                errs.push(PlanError::new(
                    format!("{ptr}/value"),
                    "must be between 0 and 1 (halfway is 0.5)",
                ));
                None
            }
        }
    }
}

fn reference(
    from: &str,
    ptr: &str,
    n_waypoints: usize,
    errs: &mut Vec<PlanError>,
) -> Option<Reference> {
    match from.trim() {
        "origin" => Some(Reference::Origin),
        "destination" => Some(Reference::Destination),
        other => match other
            .strip_prefix("waypoint:")
            .and_then(|n| n.trim().parse::<u8>().ok())
        {
            Some(n) if usize::from(n) < n_waypoints => Some(Reference::Waypoint(n)),
            Some(n) => {
                errs.push(PlanError::new(
                    format!("{ptr}/from"),
                    format!("waypoint:{n} does not exist; there are {n_waypoints} waypoints"),
                ));
                None
            }
            None => {
                errs.push(
                    PlanError::new(
                        format!("{ptr}/from"),
                        format!("unknown reference {other:?}"),
                    )
                    .with_allowed(["origin", "destination", "waypoint:<n>"]),
                );
                None
            }
        },
    }
}

const DISTANCE_UNITS: [(&str, f64); 6] = [
    ("m", 1.0),
    ("meter", 1.0),
    ("km", 1000.0),
    ("kilometer", 1000.0),
    ("mi", 1609.344),
    ("mile", 1609.344),
];
const DURATION_UNITS: [(&str, f64); 7] = [
    ("s", 1.0),
    ("sec", 1.0),
    ("second", 1.0),
    ("min", 60.0),
    ("minute", 60.0),
    ("h", 3600.0),
    ("hour", 3600.0),
];

/// Normalise a unit: lowercase, British spelling, singular, common abbreviations.
fn unit_key(u: &str) -> String {
    let u = u.trim().to_ascii_lowercase().replace("metre", "meter");
    match u.as_str() {
        "hr" | "hrs" => "h".into(),
        "mins" => "min".into(),
        "secs" => "sec".into(),
        "miles" => "mile".into(),
        _ if u.len() > 3 && u.ends_with('s') => u[..u.len() - 1].to_string(),
        _ => u,
    }
}

fn amount_of(q: &Quantity, ptr: &str, errs: &mut Vec<PlanError>) -> Option<Amount> {
    let key = unit_key(&q.unit);
    let (amount, max) = if let Some((_, f)) = DISTANCE_UNITS.iter().find(|(n, _)| *n == key) {
        (Amount::DistanceM(q.value * f), MAX_DISTANCE_M)
    } else if let Some((_, f)) = DURATION_UNITS.iter().find(|(n, _)| *n == key) {
        (Amount::DurationS(q.value * f), MAX_DURATION_S)
    } else {
        errs.push(
            PlanError::new(format!("{ptr}/unit"), format!("unknown unit {:?}", q.unit))
                .with_allowed(["m", "km", "mi", "s", "min", "h"]),
        );
        return None;
    };
    let v = match amount {
        Amount::DistanceM(v) | Amount::DurationS(v) => v,
    };
    if !(v.is_finite() && v > 0.0) {
        errs.push(PlanError::new(
            format!("{ptr}/value"),
            "must be a positive number",
        ));
        return None;
    }
    if v > max {
        errs.push(PlanError::new(
            format!("{ptr}/value"),
            "is longer than any supported trip (5000 km or 72 h)",
        ));
        return None;
    }
    Some(amount)
}

fn duration_s(q: &Quantity, ptr: &str, errs: &mut Vec<PlanError>) -> Option<f64> {
    match amount_of(q, ptr, errs)? {
        Amount::DurationS(v) => Some(v),
        Amount::DistanceM(_) => {
            errs.push(
                PlanError::new(format!("{ptr}/unit"), "must be a time unit")
                    .with_allowed(["s", "min", "h"]),
            );
            None
        }
    }
}

fn non_empty(v: Option<&str>) -> Option<String> {
    v.map(str::trim).filter(|t| !t.is_empty()).map(String::from)
}

fn lookup<T: Copy>(table: &[(&str, T)], v: &str) -> Option<T> {
    let v = v.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    table.iter().find(|(n, _)| *n == v).map(|(_, t)| *t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Coords, SavedPlace, SurfaceWhat, Vehicle};

    fn ctx() -> TripContext {
        TripContext {
            current_location: Some(Coords {
                lat: 52.3702,
                lng: 4.8952,
            }),
            departure_time: Some("2026-11-24T09:00:00+01:00".into()),
            timezone: Some("Europe/Amsterdam".into()),
            vehicle: Some(Vehicle {
                ev: Some(EvFilter {
                    min_power_kw: None,
                    connector_types: vec![Connector::Ccs2],
                }),
            }),
            saved_places: vec![SavedPlace {
                name: "home".into(),
                location: Coords {
                    lat: 52.0,
                    lng: 5.0,
                },
            }],
            ..TripContext::default()
        }
    }

    fn q(value: f64, unit: &str) -> Quantity {
        Quantity {
            value,
            unit: unit.into(),
        }
    }

    /// "Drive to Berlin, a 150 kW charger every 200 km, McDonald's 15 minutes before we arrive."
    fn reference_intent() -> SurfaceIntent {
        SurfaceIntent {
            destination: Some("Berlin".into()),
            avoid: vec!["tolls".into()],
            stops: vec![
                SurfaceStop {
                    id: Some("charger".into()),
                    what: SurfaceWhat {
                        category: Some("ev_charging".into()),
                        min_power_kw: Some(150.0),
                        ..Default::default()
                    },
                    anchor: SurfaceAnchor::Every {
                        amount: q(200.0, "km"),
                    },
                    top_n: None,
                },
                SurfaceStop {
                    id: Some("food".into()),
                    what: SurfaceWhat {
                        category: Some("fast_food".into()),
                        brand: Some("McDonald's".into()),
                        ..Default::default()
                    },
                    anchor: SurfaceAnchor::Offset {
                        from: "destination".into(),
                        direction: "before".into(),
                        amount: q(15.0, "minutes"),
                    },
                    top_n: Some(3),
                },
            ],
            ..SurfaceIntent::default()
        }
    }

    #[test]
    fn reference_trip_compiles() {
        let p = compile(&reference_intent(), &ctx()).unwrap();
        assert_eq!(p.schema_version, SCHEMA_VERSION);
        assert!(matches!(p.origin, Location::Coords { lat, .. } if (lat - 52.3702).abs() < 1e-9));
        assert_eq!(
            p.destination,
            Location::PlaceQuery {
                text: "Berlin".into()
            }
        );
        assert_eq!(p.avoid, vec![Avoid::Tolls]);
        assert_eq!(p.max_detour_s, 600);
        let c = &p.stop_requests[0];
        assert_eq!(
            c.anchor,
            Anchor::Every {
                interval: Amount::DistanceM(200_000.0),
                window_pct: 15
            }
        );
        assert_eq!(c.what.category, Some(Category::EvCharging));
        let ev = c.what.ev.as_ref().unwrap();
        assert_eq!(ev.min_power_kw, Some(150.0));
        assert_eq!(
            ev.connector_types,
            vec![Connector::Ccs2],
            "connectors inherited from vehicle"
        );
        let f = &p.stop_requests[1];
        assert_eq!(
            f.anchor,
            Anchor::Offset {
                from: Reference::Destination,
                direction: Direction::Before,
                amount: Amount::DurationS(900.0)
            }
        );
        assert_eq!(f.top_n, 3);
        assert_eq!(f.what.brand.as_deref(), Some("McDonald's"));
        assert!(f.what.ev.is_none());
    }

    #[test]
    fn charging_stop_inherits_vehicle_ev() {
        let mut s = reference_intent();
        s.stops[0].what.min_power_kw = None;
        let p = compile(&s, &ctx()).unwrap();
        assert_eq!(
            p.stop_requests[0].what.ev.as_ref().unwrap().connector_types,
            vec![Connector::Ccs2]
        );
    }

    #[test]
    fn saved_place_and_units() {
        let mut s = reference_intent();
        s.destination = Some("saved:Home".into());
        s.stops[1].anchor = SurfaceAnchor::Offset {
            from: "origin".into(),
            direction: "after".into(),
            amount: q(1.5, "hours"),
        };
        s.max_detour = Some(q(5.0, "min"));
        let p = compile(&s, &ctx()).unwrap();
        assert!(
            matches!(p.destination, Location::Coords { ref label, .. } if label.as_deref() == Some("home"))
        );
        assert!(
            matches!(p.stop_requests[1].anchor, Anchor::Offset { amount: Amount::DurationS(v), .. } if (v - 5400.0).abs() < 1e-9)
        );
        assert_eq!(p.max_detour_s, 300);
    }

    #[test]
    fn unit_spellings() {
        for (u, m) in [
            ("km", 1000.0),
            ("Kilometres", 1000.0),
            ("kilometers", 1000.0),
            ("metres", 1.0),
            ("miles", 1609.344),
            ("mi", 1609.344),
        ] {
            let mut errs = vec![];
            assert_eq!(
                amount_of(&q(1.0, u), "/x", &mut errs),
                Some(Amount::DistanceM(m)),
                "{u}"
            );
        }
        for (u, s) in [
            ("minutes", 60.0),
            ("mins", 60.0),
            ("hrs", 3600.0),
            ("h", 3600.0),
            ("seconds", 1.0),
        ] {
            let mut errs = vec![];
            assert_eq!(
                amount_of(&q(1.0, u), "/x", &mut errs),
                Some(Amount::DurationS(s)),
                "{u}"
            );
        }
    }

    #[test]
    fn errors_are_collected_with_pointers_and_allowed_values() {
        let mut s = reference_intent();
        s.avoid = vec!["tunnels".into()];
        s.stops[0].what.category = Some("charging_station".into());
        s.stops[1].anchor = SurfaceAnchor::Offset {
            from: "destination".into(),
            direction: "before".into(),
            amount: q(15.0, "furlongs"),
        };
        let errs = compile(&s, &ctx()).unwrap_err();
        let ptrs: Vec<&str> = errs.iter().map(|e| e.pointer.as_str()).collect();
        assert!(ptrs.contains(&"/avoid/0"), "{ptrs:?}");
        assert!(ptrs.contains(&"/stops/0/what/category"), "{ptrs:?}");
        assert!(ptrs.contains(&"/stops/1/anchor/amount/unit"), "{ptrs:?}");
        let cat = errs
            .iter()
            .find(|e| e.pointer == "/stops/0/what/category")
            .unwrap();
        assert!(cat.allowed.contains(&"ev_charging".to_string()));
    }

    #[test]
    fn missing_current_location_is_reported() {
        let mut c = ctx();
        c.current_location = None;
        let errs = compile(&reference_intent(), &c).unwrap_err();
        assert_eq!(errs[0].pointer, "/origin");
    }

    #[test]
    fn destination_falls_back_to_active_route() {
        let mut s = reference_intent();
        s.destination = None;
        let mut c = ctx();
        assert_eq!(compile(&s, &c).unwrap_err()[0].pointer, "/destination");
        c.active_destination = Some(Coords {
            lat: 52.52,
            lng: 13.405,
        });
        let p = compile(&s, &c).unwrap();
        assert!(matches!(p.destination, Location::Coords { .. }));
    }

    #[test]
    fn impossible_anchors_rejected() {
        let cases = [
            SurfaceAnchor::Offset {
                from: "origin".into(),
                direction: "before".into(),
                amount: q(5.0, "km"),
            },
            SurfaceAnchor::Offset {
                from: "destination".into(),
                direction: "after".into(),
                amount: q(5.0, "km"),
            },
            SurfaceAnchor::Offset {
                from: "waypoint:0".into(),
                direction: "after".into(),
                amount: q(5.0, "km"),
            },
            SurfaceAnchor::Every {
                amount: q(-1.0, "km"),
            },
            SurfaceAnchor::Every {
                amount: q(9_000.0, "km"),
            },
            SurfaceAnchor::Fraction { value: 1.5 },
        ];
        for a in cases {
            let mut s = reference_intent();
            s.stops[1].anchor = a.clone();
            assert!(compile(&s, &ctx()).is_err(), "{a:?}");
        }
    }

    #[test]
    fn duplicate_stop_ids_rejected() {
        let mut s = reference_intent();
        s.stops[1].id = Some("charger".into());
        assert!(
            compile(&s, &ctx())
                .unwrap_err()
                .iter()
                .any(|e| e.reason.contains("duplicate"))
        );
    }

    #[test]
    fn destination_only_trip_compiles() {
        let s: SurfaceIntent = serde_json::from_str(r#"{"destination":"Zeeduinweg 342"}"#).unwrap();
        let plan = compile(&s, &ctx()).unwrap();
        assert!(plan.stop_requests.is_empty());
    }

    #[test]
    fn missing_host_context_is_marked() {
        let s: SurfaceIntent = serde_json::from_str(r#"{"destination":"Utrecht"}"#).unwrap();
        let no_location = TripContext {
            current_location: None,
            ..ctx()
        };
        let errs = compile(&s, &no_location).unwrap_err();
        assert_eq!(errs[0].needs, Some(Need::CurrentLocation));

        let s: SurfaceIntent = serde_json::from_str(
            r#"{"stops":[{"what":{"category":"coffee"},"anchor":{"kind":"fraction","value":0.5}}]}"#,
        )
        .unwrap();
        let errs = compile(&s, &ctx()).unwrap_err();
        assert_eq!(errs[0].needs, Some(Need::Destination));

        let empty: SurfaceIntent = serde_json::from_str("{}").unwrap();
        assert_eq!(
            compile(&empty, &ctx()).unwrap_err()[0].needs,
            Some(Need::Request)
        );

        // Ordinary mistakes stay repairable.
        let s: SurfaceIntent =
            serde_json::from_str(r#"{"destination":"Utrecht","avoid":["lava"]}"#).unwrap();
        assert!(
            compile(&s, &ctx())
                .unwrap_err()
                .iter()
                .all(|e| e.needs.is_none())
        );
    }

    #[test]
    fn default_ids_assigned() {
        let mut s = reference_intent();
        s.stops[0].id = None;
        assert_eq!(compile(&s, &ctx()).unwrap().stop_requests[0].id, "stop_0");
    }

    #[test]
    fn surface_json_round_trip_and_unknown_fields_rejected() {
        let json = r#"{"destination":"Berlin","stops":[{"what":{"category":"coffee"},"anchor":{"kind":"fraction","value":0.5}}]}"#;
        let s: SurfaceIntent = serde_json::from_str(json).unwrap();
        assert!(compile(&s, &ctx()).is_ok());
        let bad = r#"{"destination":"Berlin","lat":1.0,"stops":[]}"#;
        assert!(serde_json::from_str::<SurfaceIntent>(bad).is_err());
    }
}
