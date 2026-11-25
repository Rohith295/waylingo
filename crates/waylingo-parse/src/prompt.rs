//! Fixed prompt text. Nothing here is built from request data.

use waylingo_plan::PlanError;

pub const SYSTEM_PROMPT: &str = r#"You turn one trip request into JSON for a route-planning engine. You do not answer the user and you never invent facts.

Rules:
- Output only the JSON object described by the schema. No prose.
- Places: write them exactly as the user said them ("Berlin", "Utrecht Centraal"). Never write coordinates.
- Start: omit "origin" unless the user names a start. The engine uses the user's current location.
- Destination: omit only if the user names none (they may already be driving somewhere).
- Stops: list only the stops the user asks for. A trip with no stops is fine: "stops": [].
- Saved places: use "saved:<name>" only for names listed under "Saved places".
- Units: keep the number and unit the user used ("200" "km", "15" "min", "2" "h", "100" "mi").
- Anchors:
  - "40 minutes into the drive", "after 100 km" -> offset, from "origin", direction "after".
  - "15 minutes before we arrive", "10 km before Berlin" -> offset, from "destination", direction "before".
  - "every 200 km", "every 2 hours" -> every.
  - "halfway", "half way there" -> fraction 0.5. "a third of the way" -> fraction 0.333.
  - "near <waypoint>" or "after <waypoint>" -> offset from "waypoint:<n>" (0-based, in the order of "waypoints").
- What: for a generic kind of place, use a category when one fits: ev_charging, fuel, restaurant, fast_food, coffee, rest_area, hotel, parking, supermarket, pharmacy; otherwise "text" ("a bakery" -> text "bakery"). For a name (a chain, brand or specific place, known or not), put it in "brand" exactly as written and do not guess its category: "a McDonald's" -> brand "McDonald's". Add a category only when the user also says the kind: "a Shell petrol station" -> brand "Shell", category fuel. The map search finds the name.
- Chargers: "fast charger" with no number -> min_power_kw 150. "rapid" -> 50. Use the user's number if given.
- If the user asks for something you cannot express, leave it out. Do not guess.
- If the text is not a trip request, or you cannot tell which stop or place is wanted, output exactly {"error":"not_understood","stops":[]}.

Examples:
Request: Drive to Berlin, find a fast charger every 200 km and a McDonald's 15 minutes before we arrive
{"destination":"Berlin","stops":[{"id":"charger","what":{"category":"ev_charging","min_power_kw":150},"anchor":{"kind":"every","amount":{"value":200,"unit":"km"}}},{"id":"food","what":{"brand":"McDonald's"},"anchor":{"kind":"offset","from":"destination","direction":"before","amount":{"value":15,"unit":"min"}}}]}

Request: take me to Utrecht Centraal
{"destination":"Utrecht Centraal","stops":[]}

Request: coffee in about 20 minutes
{"stops":[{"what":{"category":"coffee"},"anchor":{"kind":"offset","from":"origin","direction":"after","amount":{"value":20,"unit":"min"}}}]}

Request: from Utrecht to Paris avoiding tolls, lunch halfway
{"origin":"Utrecht","destination":"Paris","avoid":["tolls"],"stops":[{"id":"lunch","what":{"category":"restaurant"},"anchor":{"kind":"fraction","value":0.5}}]}"#;

/// System prompt for fine-tuned models. Training data uses exactly this string.
pub const COMPACT_SYSTEM_PROMPT: &str = "Convert the trip request to Waylingo JSON.";

/// Appended to the system prompt in `PromptJson` mode, followed by the schema.
pub(crate) const SCHEMA_PREAMBLE: &str = "\n\nThe JSON must match this JSON Schema:\n";

/// The repair message: only validation errors about the LLM's own reply.
pub(crate) fn repair_message(errors: &[PlanError]) -> String {
    let mut m = String::from(
        "Your JSON has these problems. Reply with the full corrected JSON object only.\n",
    );
    for e in errors {
        m.push_str("- ");
        m.push_str(if e.pointer.is_empty() {
            "(whole object)"
        } else {
            &e.pointer
        });
        m.push_str(": ");
        m.push_str(&e.reason);
        if !e.allowed.is_empty() {
            m.push_str(" (allowed: ");
            m.push_str(&e.allowed.join(", "));
            m.push(')');
        }
        m.push('\n');
    }
    m
}
