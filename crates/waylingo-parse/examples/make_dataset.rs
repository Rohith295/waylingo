//! Generate the fine-tuning dataset for the Waylingo parser.
//!
//! ```text
//! cargo run -q -p waylingo-parse --example make_dataset -- [count] [out_dir]
//! # default: 40000 examples into finetune/data
//! # WAYLINGO_EXCLUDE=a.jsonl,b.jsonl  never put names from these test files in training
//! ```
//!
//! Every example is (compact system prompt, user message in the exact format the
//! engine sends, canonical SurfaceIntent JSON). Labels are built from the same
//! template choices that produced the sentence, so they are correct by
//! construction, and every positive example must compile with the real plan
//! compiler or it is dropped. Places cover Europe, the US and India. English only.
//! No map provider data is involved.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

use serde_json::json;
use waylingo_parse::{COMPACT_SYSTEM_PROMPT, ParseInput};
use waylingo_plan::{
    Coords, Quantity, SavedPlace, SurfaceAnchor, SurfaceIntent, SurfaceStop, SurfaceWhat,
    TripContext, compile,
};

// ---------- tiny deterministic RNG ----------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len())]
    }
    fn chance(&mut self, p: f64) -> bool {
        (self.next() % 10_000) as f64 / 10_000.0 < p
    }
}

// ---------- held-out pools (overfitting check) ----------

thread_local! {
    /// When true, generate only from held-out places and brands (the unseen test set).
    static HOLDOUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn holdout() -> bool {
    HOLDOUT.with(std::cell::Cell::get)
}

/// Every fifth place is held out of training and appears only in `test_unseen`.
fn pool_places(r: Region) -> Vec<&'static str> {
    places(r)
        .iter()
        .enumerate()
        .filter(|(i, _)| (i % 5 == 4) == holdout())
        .map(|(_, p)| *p)
        .collect()
}

/// Every fourth brand (in lists of three or more) is held out of training. A brand
/// that is in any region's training pool (e.g. Shell, Holiday Inn) is never held out.
fn pool_brands(r: Region, cat: &str) -> Vec<&'static str> {
    let train_pool = |r2: Region, c: &str| -> Vec<&'static str> {
        let all = brands(r2, c);
        all.iter()
            .enumerate()
            .filter(|(i, _)| all.len() < 3 || i % 4 != 3)
            .map(|(_, b)| *b)
            .collect()
    };
    if !holdout() {
        return train_pool(r, cat);
    }
    let in_training: HashSet<&str> = [Region::Europe, Region::Us, Region::India]
        .into_iter()
        .flat_map(|r2| train_pool(r2, cat))
        .collect();
    brands(r, cat)
        .iter()
        .copied()
        .filter(|b| !in_training.contains(b))
        .collect()
}

// ---------- regions ----------

#[derive(Clone, Copy, PartialEq)]
enum Region {
    Europe,
    Us,
    India,
}

fn region_name(r: Region) -> &'static str {
    match r {
        Region::Europe => "europe",
        Region::Us => "us",
        Region::India => "india",
    }
}

fn places(r: Region) -> &'static [&'static str] {
    match r {
        Region::Europe => &[
            "Amsterdam",
            "Paris",
            "Berlin",
            "Madrid",
            "Barcelona",
            "Lisbon",
            "Porto",
            "Rome",
            "Milan",
            "Florence",
            "Naples",
            "Venice",
            "Vienna",
            "Prague",
            "Warsaw",
            "Krakow",
            "Budapest",
            "Brussels",
            "Antwerp",
            "Bruges",
            "Copenhagen",
            "Stockholm",
            "Gothenburg",
            "Oslo",
            "Bergen",
            "Helsinki",
            "Dublin",
            "Cork",
            "Edinburgh",
            "Glasgow",
            "London",
            "Manchester",
            "Bristol",
            "Cardiff",
            "Zurich",
            "Geneva",
            "Lyon",
            "Marseille",
            "Nice",
            "Bordeaux",
            "Toulouse",
            "Seville",
            "Valencia",
            "Granada",
            "Bilbao",
            "Athens",
            "Thessaloniki",
            "Bucharest",
            "Sofia",
            "Zagreb",
            "Split",
            "Ljubljana",
            "Bratislava",
            "Luxembourg",
            "Rotterdam",
            "Utrecht",
            "Munich",
            "Hamburg",
            "Salzburg",
            "Tallinn",
            "Riga",
            "Vilnius",
            "Tower Bridge, London",
            "Rue de Rivoli, Paris",
            "Gran Via, Madrid",
            "Via del Corso, Rome",
            "Nyhavn, Copenhagen",
            "Damrak, Amsterdam",
            "Avenida da Liberdade, Lisbon",
            "Princes Street, Edinburgh",
            "Grafton Street, Dublin",
            "La Rambla, Barcelona",
            "Gare du Nord, Paris",
            "King's Cross Station",
            "Schiphol Airport",
            "Heathrow Airport",
            "Charles de Gaulle Airport",
            "Fiumicino Airport",
            "Sagrada Familia",
            "the Colosseum",
            "the Eiffel Tower",
            "the Acropolis",
            "Charles Bridge, Prague",
            "Old Town Square, Prague",
            "Piazza San Marco, Venice",
            "Montmartre",
            "Camden Market",
        ],
        Region::Us => &[
            "New York",
            "Boston",
            "Philadelphia",
            "Washington DC",
            "Baltimore",
            "Chicago",
            "Detroit",
            "Cleveland",
            "Columbus",
            "Nashville",
            "Memphis",
            "Atlanta",
            "Miami",
            "Orlando",
            "Tampa",
            "Dallas",
            "Houston",
            "Austin",
            "San Antonio",
            "Denver",
            "Boulder",
            "Phoenix",
            "Tucson",
            "Las Vegas",
            "Los Angeles",
            "San Diego",
            "San Francisco",
            "San Jose",
            "Sacramento",
            "Portland",
            "Seattle",
            "Salt Lake City",
            "Minneapolis",
            "St. Louis",
            "Kansas City",
            "New Orleans",
            "Charlotte",
            "Raleigh",
            "Pittsburgh",
            "Albuquerque",
            "Santa Fe",
            "Savannah",
            "Charleston",
            "Buffalo",
            "Indianapolis",
            "Milwaukee",
            "Omaha",
            "Boise",
            "Anchorage",
            "Lincoln Memorial, Washington DC",
            "Terminal 3, New York",
            "Times Square",
            "the Golden Gate Bridge",
            "Hollywood Boulevard, Los Angeles",
            "Pike Place Market, Seattle",
            "Navy Pier, Chicago",
            "Grand Canyon Village",
            "JFK Airport",
            "LAX",
            "O'Hare Airport",
            "Yosemite Valley",
            "Bourbon Street, New Orleans",
            "Springfield",
            "the Space Needle",
            "Mount Rushmore",
            "Lake Tahoe",
            "Yellowstone",
            "Napa Valley",
            "Santa Monica Pier",
            "Central Park",
            "the Las Vegas Strip",
            "Disney World",
        ],
        Region::India => &[
            "Mumbai",
            "Pune",
            "Nashik",
            "Delhi",
            "New Delhi",
            "Gurugram",
            "Noida",
            "Jaipur",
            "Agra",
            "Bengaluru",
            "Bangalore",
            "Mysuru",
            "Mangaluru",
            "Chennai",
            "Pondicherry",
            "Madurai",
            "Hyderabad",
            "Warangal",
            "Kolkata",
            "Ahmedabad",
            "Surat",
            "Vadodara",
            "Lucknow",
            "Kanpur",
            "Varanasi",
            "Kochi",
            "Thiruvananthapuram",
            "Kozhikode",
            "Goa",
            "Panaji",
            "Chandigarh",
            "Amritsar",
            "Ludhiana",
            "Indore",
            "Bhopal",
            "Nagpur",
            "Visakhapatnam",
            "Vijayawada",
            "Coimbatore",
            "Ooty",
            "Udaipur",
            "Jodhpur",
            "Shimla",
            "Manali",
            "Rishikesh",
            "Dehradun",
            "Guwahati",
            "Bhubaneswar",
            "Raipur",
            "Patna",
            "MG Road, Bengaluru",
            "Marine Drive, Mumbai",
            "Connaught Place, New Delhi",
            "Bandra Kurla Complex, Mumbai",
            "Koramangala, Bengaluru",
            "Indiranagar, Bengaluru",
            "Hitech City, Hyderabad",
            "Park Street, Kolkata",
            "Anna Salai, Chennai",
            "Baga Beach, Goa",
            "India Gate",
            "the Taj Mahal",
            "the Gateway of India",
            "Charminar",
            "Hawa Mahal, Jaipur",
            "Chhatrapati Shivaji Maharaj International Airport",
            "Kempegowda International Airport",
            "Indira Gandhi International Airport",
            "Rajiv Gandhi International Airport",
            "Electronic City, Bengaluru",
            "Juhu Beach, Mumbai",
            "Cyber City, Gurugram",
        ],
    }
}

fn brands(r: Region, cat: &str) -> &'static [&'static str] {
    match (r, cat) {
        (Region::Europe, "fast_food") => {
            &["McDonald's", "Burger King", "KFC", "Subway", "Five Guys"]
        }
        (Region::Us, "fast_food") => &[
            "McDonald's",
            "Wendy's",
            "Taco Bell",
            "Chick-fil-A",
            "In-N-Out",
            "Burger King",
            "Chipotle",
        ],
        (Region::India, "fast_food") => &[
            "McDonald's",
            "KFC",
            "Domino's",
            "Haldiram's",
            "Burger King",
            "Wow! Momo",
        ],
        (Region::Europe, "coffee") => &["Starbucks", "Costa Coffee", "Pret A Manger", "Caffè Nero"],
        (Region::Us, "coffee") => &["Starbucks", "Dunkin'", "Peet's Coffee", "Tim Hortons"],
        (Region::India, "coffee") => &[
            "Starbucks",
            "Café Coffee Day",
            "Third Wave Coffee",
            "Blue Tokai",
        ],
        (Region::Europe, "fuel") => &["Shell", "BP", "TotalEnergies", "Esso", "Repsol", "Eni"],
        (Region::Us, "fuel") => &[
            "Shell",
            "Chevron",
            "Exxon",
            "Mobil",
            "Costco Gas",
            "Buc-ee's",
        ],
        (Region::India, "fuel") => &["Indian Oil", "Bharat Petroleum", "HP", "Shell", "Reliance"],
        (Region::Europe, "ev_charging") => &["Ionity", "Fastned", "Tesla Supercharger", "Allego"],
        (Region::Us, "ev_charging") => &[
            "Tesla Supercharger",
            "Electrify America",
            "ChargePoint",
            "EVgo",
        ],
        (Region::India, "ev_charging") => &[
            "Tata Power EZ Charge",
            "Statiq",
            "ChargeZone",
            "Jio-bp pulse",
        ],
        (Region::Europe, "supermarket") => &[
            "Lidl",
            "Aldi",
            "Carrefour",
            "Tesco",
            "Albert Heijn",
            "Mercadona",
        ],
        (Region::Us, "supermarket") => {
            &["Walmart", "Target", "Whole Foods", "Trader Joe's", "Costco"]
        }
        (Region::India, "supermarket") => &["Reliance Fresh", "DMart", "More", "Spencer's"],
        (Region::Europe, "hotel") => &["Ibis", "Novotel", "Premier Inn", "Holiday Inn"],
        (Region::Us, "hotel") => &["Holiday Inn", "Marriott", "Hampton Inn", "Motel 6"],
        (Region::India, "hotel") => &["Taj", "OYO", "Lemon Tree", "ITC"],
        (Region::Europe, "pharmacy") => &["Boots"],
        (Region::Us, "pharmacy") => &["CVS", "Walgreens"],
        (Region::India, "pharmacy") => &["Apollo Pharmacy", "MedPlus"],
        _ => &[],
    }
}

/// Free-text places (no category fits): (phrase with article, label text).
fn text_places(r: Region) -> &'static [(&'static str, &'static str)] {
    match r {
        Region::Europe => &[
            ("a bakery", "bakery"),
            ("a bike shop", "bike shop"),
            ("a car wash", "car wash"),
            ("a playground", "playground"),
            ("an ice cream shop", "ice cream shop"),
            ("a flower shop", "flower shop"),
            ("a bookshop", "bookshop"),
            ("a laundrette", "laundrette"),
            ("a tyre shop", "tyre shop"),
        ],
        Region::Us => &[
            ("a car wash", "car wash"),
            ("a dog park", "dog park"),
            ("a bakery", "bakery"),
            ("a laundromat", "laundromat"),
            ("a bookstore", "bookstore"),
            ("a tire shop", "tire shop"),
            ("an ice cream shop", "ice cream shop"),
            ("a playground", "playground"),
        ],
        Region::India => &[
            ("a dhaba", "dhaba"),
            ("a chai stall", "chai stall"),
            ("an ATM", "ATM"),
            ("a mechanic", "mechanic"),
            ("a sweet shop", "sweet shop"),
            ("a puncture repair shop", "puncture repair shop"),
            ("a bakery", "bakery"),
            ("a car wash", "car wash"),
        ],
    }
}

// ---------- invented names (teach copying, not recall) ----------
//
// Real place and brand lists are small, so a model can memorise them. Invented towns,
// street addresses, landmarks and brands give thousands of names the model can only
// get right by copying them from the request.

thread_local! {
    /// Lowercased places and brands used in training; the unseen set avoids them.
    static TRAIN_NAMES: std::cell::RefCell<HashSet<String>> =
        std::cell::RefCell::new(HashSet::new());
}

fn remember(name: &str) {
    TRAIN_NAMES.with(|s| s.borrow_mut().insert(name.to_lowercase()));
}

thread_local! {
    /// Lowercased names from earlier test files (`WAYLINGO_EXCLUDE`): never trained on.
    static EXCLUDED: std::cell::RefCell<HashSet<String>> =
        std::cell::RefCell::new(HashSet::new());
}

/// Load place and brand names from the labels of earlier test files, so a new
/// training set never contains a name those tests rely on being unseen.
fn load_excluded(paths: &str) {
    for path in paths.split(',').filter(|p| !p.is_empty()) {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        for line in text.lines() {
            let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(label) = row["messages"][2]["content"].as_str() else {
                continue;
            };
            // Places and brands only: generic free text ("bakery", "mechanic") is a small
            // fixed vocabulary the model must still learn.
            if let Ok(i) = serde_json::from_str::<SurfaceIntent>(label) {
                let names = i
                    .origin
                    .iter()
                    .chain(i.destination.iter())
                    .chain(i.waypoints.iter())
                    .filter(|p| !p.starts_with("saved:"))
                    .chain(i.stops.iter().filter_map(|s| s.what.brand.as_ref()))
                    .map(|n| n.to_lowercase());
                EXCLUDED.with(|s| s.borrow_mut().extend(names));
            }
        }
    }
}

fn excluded(name: &str) -> bool {
    EXCLUDED.with(|s| s.borrow().contains(&name.to_lowercase()))
}

fn seen_in_training(name: &str) -> bool {
    TRAIN_NAMES.with(|s| s.borrow().contains(&name.to_lowercase()))
}

/// An invented one-word name in the style of the region, e.g. "Dornholm", "Rampur".
fn coined(r: Region, rng: &mut Rng) -> String {
    let (pre, suf): (&[&str], &[&str]) = match r {
        Region::Europe => (
            &[
                "Alder", "Bran", "Carls", "Dorn", "Elm", "Fal", "Gren", "Hol", "Kirch", "Lund",
                "Mar", "Nor", "Ost", "Ros", "Sten", "Thal", "Vel", "Wester", "Zell", "Bel", "Cor",
                "Lam", "Mont", "Vila", "Bor", "Kra", "Lis", "Pol", "Tor", "Sal",
            ],
            &[
                "burg", "dorf", "heim", "holm", "by", "stad", "ford", "ton", "ville", "mont", "ac",
                "ano", "ella", "ova", "ice", "sund", "berg", "haven", "wick", "era",
            ],
        ),
        Region::Us => (
            &[
                "Spring", "Maple", "Cedar", "Lake", "River", "Oak", "Pine", "Fair", "Green",
                "Mill", "Clear", "Silver", "Rock", "Elk", "Bay", "Red", "Stone", "Brook", "Glen",
                "Ash", "Fox", "Haw", "Kings", "Bridge", "Win",
            ],
            &[
                "field", "ville", "ton", "wood", "dale", "port", "view", "ridge", "land", "burg",
                "ford", "mont", "haven", " City", " Falls", " Springs", " Heights",
            ],
        ),
        Region::India => (
            &[
                "Ram", "Shiv", "Kris", "Har", "Sur", "Chandra", "Vij", "Ganga", "Laksh", "Bhav",
                "Dev", "Nav", "Raj", "Sita", "Ana", "Mad", "Kal", "Pad", "Sar", "Tir", "Bel",
                "Kum", "Nan", "Ud", "Jag",
            ],
            &[
                "pur", "abad", "nagar", "garh", "ganj", "wadi", "palli", "kota", "halli", "pet",
                "ner", "giri", "puram", "ur", "gaon", "kund",
            ],
        ),
    };
    format!("{}{}", rng.pick(pre), rng.pick(suf))
}

/// An invented town, sometimes with a state or country.
fn invented_town(r: Region, rng: &mut Rng) -> String {
    let t = coined(r, rng);
    match r {
        Region::Us if rng.chance(0.3) => format!(
            "{t}, {}",
            rng.pick(&[
                "OH", "TX", "CA", "NY", "PA", "GA", "OR", "MI", "NC", "AZ", "CO", "WA"
            ])
        ),
        Region::India if rng.chance(0.2) => format!(
            "{t}, {}",
            rng.pick(&[
                "Maharashtra",
                "Karnataka",
                "Tamil Nadu",
                "Rajasthan",
                "Gujarat",
                "Kerala",
                "Punjab",
                "Odisha",
                "Telangana",
                "Uttarakhand",
            ])
        ),
        Region::Europe if rng.chance(0.15) => format!(
            "{t}, {}",
            rng.pick(&[
                "France", "Germany", "Italy", "Spain", "Poland", "Sweden", "Norway", "Austria",
                "Portugal", "Ireland",
            ])
        ),
        _ => t,
    }
}

/// A short town name for addresses and landmarks: a real one-or-two-word place or an
/// invented one.
fn town(r: Region, rng: &mut Rng) -> String {
    if rng.chance(0.5) {
        let short: Vec<&str> = pool_places(r)
            .into_iter()
            .filter(|p| {
                const LANDMARK: [&str; 12] = [
                    "Airport", "Station", "Pier", "Market", "Beach", "Park", "Bridge", "Gate",
                    "Fort", "Palace", "Tower", "Square",
                ];
                !p.contains(',')
                    && p.split_whitespace().count() <= 2
                    && !p.split_whitespace().any(|w| LANDMARK.contains(&w))
            })
            .collect();
        if !short.is_empty() {
            return rng.pick(&short).to_string();
        }
    }
    coined(r, rng)
}

fn street_name(r: Region, rng: &mut Rng) -> &'static str {
    rng.pick(match r {
        Region::Europe => &[
            "Victoria",
            "Albert",
            "Kings",
            "Queens",
            "Mozart",
            "Garibaldi",
            "Voltaire",
            "Nelson",
            "Lindholm",
            "Kowalski",
            "Novak",
            "Dvorak",
            "Andersen",
            "Moreau",
            "Rossi",
            "Jansen",
            "Larsen",
            "Mill",
            "Church",
            "Station",
        ],
        Region::Us => &[
            "Washington",
            "Lincoln",
            "Jefferson",
            "Franklin",
            "Madison",
            "Elm",
            "Main",
            "Park",
            "Sunset",
            "Peachtree",
            "Market",
            "Broad",
            "Hudson",
            "Spruce",
            "Magnolia",
            "Willow",
            "Highland",
            "Cherry",
            "Ocean",
            "Canyon",
        ],
        Region::India => &[
            "Gandhi",
            "Nehru",
            "Tilak",
            "Patel",
            "Ambedkar",
            "Tagore",
            "Shastri",
            "Bose",
            "Netaji",
            "Rajaji",
            "Ashoka",
            "Lajpat",
            "Sardar",
            "Vivekananda",
            "Kamaraj",
            "Shivaji",
            "Malviya",
            "MG",
            "Station",
            "Temple",
        ],
    })
}

/// An invented street address, e.g. "Maplewood".
fn address(r: Region, rng: &mut Rng) -> String {
    let n = 1 + rng.below(400);
    let s = street_name(r, rng);
    let t = town(r, rng);
    let street = match r {
        Region::Europe => match rng.below(12) {
            0 => format!("{n} {s} Road"),
            1 => format!("{n} {s} Street"),
            2 => format!("Via {s} {n}"),
            3 => format!("Rue {s} {n}"),
            4 => format!("{s}straat {n}"),
            5 => format!("{s}weg {n}"),
            6 => format!("{}weg {n}", coined(r, rng)),
            7 => format!("{}straat {n}", coined(r, rng)),
            8 => format!("{}gracht {n}", coined(r, rng)),
            9 => format!("{}laan {n}", coined(r, rng)),
            10 => format!("{}strasse {n}", coined(r, rng)),
            _ => format!("{s}gatan {n}"),
        },
        Region::Us => format!(
            "{n} {s} {}",
            rng.pick(&[
                "St", "Street", "Ave", "Avenue", "Blvd", "Road", "Drive", "Lane"
            ])
        ),
        Region::India => match rng.below(5) {
            0 => format!("{n} {s} Road"),
            1 => format!("{s} Marg"),
            2 => format!("Sector {}", 1 + rng.below(80)),
            3 => format!("{s} Nagar"),
            _ => format!("Plot {n}, {s} Road"),
        },
    };
    // People often leave the town out ("drive me to branmont"). Half of all
    // addresses do, so the model does not learn that a town always follows.
    if rng.chance(0.5) && !street.starts_with("Sector") {
        street
    } else {
        format!("{street}, {t}")
    }
}

/// An invented landmark, e.g. "Rampur Railway Station", "Sunset Plaza".
fn landmark(r: Region, rng: &mut Rng) -> String {
    if rng.chance(0.3) {
        return format!(
            "{} {}",
            street_name(r, rng),
            rng.pick(&["Park", "Square", "Market", "Plaza", "Gardens"])
        );
    }
    let kinds: &[&str] = match r {
        Region::India => &[
            "Railway Station",
            "Bus Stand",
            "Airport",
            "Fort",
            "Lake",
            "Mall",
            "University",
            "Temple",
            "Beach",
            "Hospital",
        ],
        Region::Us => &[
            "Airport",
            "Mall",
            "Stadium",
            "University",
            "Convention Center",
            "State Park",
            "Beach",
            "Union Station",
            "Hospital",
            "Outlets",
        ],
        Region::Europe => &[
            "Central Station",
            "Airport",
            "Old Town",
            "Harbour",
            "Cathedral",
            "University",
            "Castle",
            "Beach",
            "Stadium",
            "Hospital",
        ],
    };
    format!("{} {}", town(r, rng), rng.pick(kinds))
}

/// A trip place: a real one from the pool, or an invented town, address or landmark.
/// In the unseen set, never a place used in training.
fn pick_place(r: Region, rng: &mut Rng) -> String {
    for _ in 0..50 {
        let p = match rng.below(100) {
            0..=44 => {
                let all = pool_places(r);
                rng.pick(&all).to_string()
            }
            45..=64 => invented_town(r, rng),
            65..=89 => address(r, rng),
            _ => landmark(r, rng),
        };
        if !holdout() || !seen_in_training(&p) {
            return p;
        }
    }
    invented_town(r, rng)
}

/// An invented brand whose name says what it is, e.g. "Dornholm Coffee", "Kaveri's
/// Kitchen". `None` for categories without chains.
fn invented_brand(r: Region, rng: &mut Rng, cat: &str) -> Option<String> {
    let nouns: &[&str] = match cat {
        "fuel" => &["Fuel", "Petroleum", "Gas", "Energy"],
        "ev_charging" => &["Charge", "EV", "Volt", "Charging"],
        "fast_food" => &["Burgers", "Chicken", "Pizza", "Tacos", "Wraps"],
        "coffee" => &["Coffee", "Café", "Roasters", "Espresso"],
        "supermarket" => &["Market", "Foods", "Grocers", "Mart"],
        "hotel" => &["Inn", "Hotel", "Suites", "Lodge"],
        "pharmacy" => &["Pharmacy", "Chemist", "Drugstore"],
        "restaurant" => &["Kitchen", "Grill", "Bistro", "Diner"],
        _ => return None,
    };
    for _ in 0..30 {
        let stem = coined(r, rng);
        let b = if rng.chance(0.3) {
            format!("{stem}'s {}", rng.pick(nouns))
        } else {
            format!("{stem} {}", rng.pick(nouns))
        };
        if !holdout() || !seen_in_training(&b) {
            return Some(b);
        }
    }
    None
}

fn locale(r: Region, rng: &mut Rng) -> &'static str {
    match r {
        Region::Europe => rng.pick(&["en", "en-GB", "en-IE"]),
        Region::Us => rng.pick(&["en-US", "en"]),
        Region::India => rng.pick(&["en-IN", "en"]),
    }
}

// ---------- labels ----------

fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}
fn offset(from: &str, dir: &str, value: f64, unit: &str) -> SurfaceAnchor {
    SurfaceAnchor::Offset {
        from: from.into(),
        direction: dir.into(),
        amount: q(value, unit),
    }
}
fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

const CATEGORIES: [&str; 10] = [
    "ev_charging",
    "fuel",
    "restaurant",
    "fast_food",
    "coffee",
    "rest_area",
    "hotel",
    "parking",
    "supermarket",
    "pharmacy",
];

/// A "what" phrase (with article) and its label.
fn what(r: Region, rng: &mut Rng, cat: &str) -> (String, SurfaceWhat) {
    let mut w = SurfaceWhat {
        category: Some(cat.into()),
        ..SurfaceWhat::default()
    };
    let chains = pool_brands(r, cat);
    if rng.chance(0.3) {
        // Half real chains, half invented ones the model can only copy.
        let invented = if chains.is_empty() || rng.chance(0.5) {
            invented_brand(r, rng, cat)
        } else {
            None
        };
        let b = invented.or_else(|| (!chains.is_empty()).then(|| rng.pick(&chains).to_string()));
        if let Some(b) = b {
            // A name is searched as a name; the model never guesses its category. The
            // category is kept only when the user also says the kind of place.
            let (phrase, kind_said) = match (cat, rng.below(10)) {
                ("fuel", 0..=1) => (
                    format!(
                        "a {b} {}",
                        rng.pick(&["petrol station", "gas station", "pump"])
                    ),
                    true,
                ),
                ("ev_charging", 0..=1) => (
                    format!("a {b} {}", rng.pick(&["charger", "charging station"])),
                    true,
                ),
                ("hotel", 0) => (format!("a {b} hotel"), true),
                ("supermarket", 0) => (format!("a {b} supermarket"), true),
                (_, 0..=5) => (format!("a {b}"), false),
                _ => (b.clone(), false),
            };
            if !kind_said {
                w.category = None;
            }
            w.brand = Some(b);
            return (phrase, w);
        }
    }
    let conns = [
        ("CCS2", "ccs2"),
        ("CCS", "ccs2"),
        ("Type 2", "type2"),
        ("CHAdeMO", "chademo"),
        ("Tesla", "nacs"),
        ("NACS", "nacs"),
        ("CCS1", "ccs1"),
    ];
    let phrase: String = match cat {
        "ev_charging" => match rng.below(9) {
            0 => {
                w.min_power_kw = Some(150.0);
                "a fast charger".into()
            }
            1 => {
                w.min_power_kw = Some(50.0);
                "a rapid charger".into()
            }
            2 | 3 => {
                let kw = *rng.pick(&[50.0, 100.0, 150.0, 250.0, 350.0, 22.0, 60.0]);
                w.min_power_kw = Some(kw);
                if rng.chance(0.5) {
                    format!("a {} kW charger", fmt_num(kw))
                } else {
                    format!("a {}kW charger", fmt_num(kw))
                }
            }
            4 => {
                let (n, c) = *rng.pick(&conns);
                w.connector_types = vec![c.into()];
                format!("a {n} charger")
            }
            5 => {
                let kw = *rng.pick(&[50.0, 150.0, 350.0]);
                let (n, c) = *rng.pick(&conns[..3]);
                w.min_power_kw = Some(kw);
                w.connector_types = vec![c.into()];
                format!("a {} kW {n} charger", fmt_num(kw))
            }
            _ => rng
                .pick(&[
                    "a charger",
                    "a charging station",
                    "an EV charger",
                    "a place to charge",
                    "a charging point",
                ])
                .to_string(),
        },
        "fuel" => match r {
            Region::Us => rng
                .pick(&["a gas station", "gas", "a place to fill up", "a fuel stop"])
                .to_string(),
            Region::India => rng
                .pick(&[
                    "a petrol pump",
                    "a petrol station",
                    "fuel",
                    "a fuel station",
                    "a CNG station",
                ])
                .to_string(),
            Region::Europe => rng
                .pick(&[
                    "a petrol station",
                    "fuel",
                    "a fuel stop",
                    "a place to fill up",
                    "a filling station",
                ])
                .to_string(),
        },
        "restaurant" => rng
            .pick(&[
                "a restaurant",
                "lunch",
                "dinner",
                "somewhere to eat",
                "a place for lunch",
                "breakfast",
                "a meal",
            ])
            .to_string(),
        "fast_food" => rng
            .pick(&[
                "fast food",
                "a quick bite",
                "a burger place",
                "a drive-thru",
            ])
            .to_string(),
        "coffee" => rng
            .pick(&[
                "coffee",
                "a coffee break",
                "a café",
                "a coffee shop",
                "a coffee stop",
            ])
            .to_string(),
        "rest_area" => rng
            .pick(&[
                "a rest area",
                "a rest stop",
                "a place to stretch our legs",
                "a service area",
                "a toilet break",
            ])
            .to_string(),
        "hotel" => rng
            .pick(&[
                "a hotel",
                "a place to stay overnight",
                "somewhere to sleep",
                "a motel",
            ])
            .to_string(),
        "parking" => rng
            .pick(&["parking", "a place to park", "a parking spot", "a car park"])
            .to_string(),
        "supermarket" => rng
            .pick(&["a supermarket", "a grocery store", "groceries"])
            .to_string(),
        "pharmacy" => rng
            .pick(&["a pharmacy", "a chemist", "a drugstore", "a medical store"])
            .to_string(),
        _ => unreachable!("unknown category {cat}"),
    };
    (phrase, w)
}

/// An anchor phrase and its label. `dest` names the destination when the user named
/// one; `wp` names the first waypoint when there is one.
fn anchor(
    r: Region,
    rng: &mut Rng,
    dest: Option<&str>,
    wp: Option<&str>,
) -> (String, SurfaceAnchor) {
    let miles = r == Region::Us && rng.chance(0.85);
    loop {
        match rng.below(14) {
            0..=2 => {
                let n = *rng.pick(&[10.0, 15.0, 20.0, 25.0, 30.0, 40.0, 45.0, 50.0, 75.0, 90.0]);
                let s = fmt_num(n);
                let p = rng
                    .pick(&[
                        format!("{s} minutes in"),
                        format!("after {s} minutes"),
                        format!("in {s} minutes"),
                        format!("in about {s} min"),
                        format!("{s} mins into the drive"),
                        format!("after about {s} minutes of driving"),
                        format!("{s} minutes from now"),
                    ])
                    .clone();
                return (p, offset("origin", "after", n, "min"));
            }
            3 => {
                let (p, v): (&str, f64) = *rng.pick(&[
                    ("an hour in", 1.0),
                    ("after an hour", 1.0),
                    ("in an hour", 1.0),
                    ("after two hours", 2.0),
                    ("an hour and a half in", 1.5),
                    ("after 3 hours", 3.0),
                    ("in 2 hours", 2.0),
                    ("after 2.5 hours", 2.5),
                ]);
                return (p.into(), offset("origin", "after", v, "h"));
            }
            4 | 5 => {
                let (n, unit) = if miles {
                    (
                        *rng.pick(&[10.0, 20.0, 25.0, 30.0, 50.0, 60.0, 75.0, 100.0, 150.0]),
                        "mi",
                    )
                } else {
                    (
                        *rng.pick(&[20.0, 30.0, 50.0, 75.0, 100.0, 120.0, 150.0, 200.0, 250.0]),
                        "km",
                    )
                };
                let s = fmt_num(n);
                let u = if unit == "mi" {
                    *rng.pick(&["miles", "mi"])
                } else {
                    *rng.pick(&["km", "kilometres", "kilometers"])
                };
                let p = rng
                    .pick(&[
                        format!("after {s} {u}"),
                        format!("{s} {u} in"),
                        format!("in {s} {u}"),
                        format!("{s} {u} from here"),
                    ])
                    .clone();
                return (p, offset("origin", "after", n, unit));
            }
            6 | 7 => {
                let n = *rng.pick(&[10.0, 15.0, 20.0, 30.0, 45.0]);
                let s = fmt_num(n);
                let mut opts = vec![
                    format!("{s} minutes before we arrive"),
                    format!("{s} min before arriving"),
                    format!("{s} minutes before we get there"),
                    format!("{s} minutes before the end of the trip"),
                ];
                if let Some(d) = dest {
                    opts.push(format!("{s} minutes before {d}"));
                    opts.push(format!("{s} minutes before reaching {d}"));
                    opts.push(format!("{s} minutes before we get to {d}"));
                }
                return (
                    rng.pick(&opts).clone(),
                    offset("destination", "before", n, "min"),
                );
            }
            8 => {
                let (n, unit) = if miles {
                    (*rng.pick(&[5.0, 10.0, 20.0, 30.0, 50.0]), "mi")
                } else {
                    (*rng.pick(&[5.0, 10.0, 20.0, 30.0, 50.0]), "km")
                };
                let s = fmt_num(n);
                let u = if unit == "mi" {
                    "miles"
                } else {
                    *rng.pick(&["km", "kilometres"])
                };
                let mut opts = vec![
                    format!("{s} {u} before we arrive"),
                    format!("{s} {u} before the destination"),
                ];
                if let Some(d) = dest {
                    opts.push(format!("{s} {u} before {d}"));
                    opts.push(format!("{s} {u} out from {d}"));
                    opts.push(format!("{s} {u} before we get to {d}"));
                }
                return (
                    rng.pick(&opts).clone(),
                    offset("destination", "before", n, unit),
                );
            }
            9 | 10 => {
                return match (rng.below(4), miles) {
                    (0 | 1, true) => {
                        let n = *rng.pick(&[50.0, 75.0, 100.0, 150.0, 200.0]);
                        (
                            format!("every {} miles", fmt_num(n)),
                            SurfaceAnchor::Every { amount: q(n, "mi") },
                        )
                    }
                    (0 | 1, false) => {
                        let n = *rng.pick(&[50.0, 100.0, 150.0, 200.0, 250.0, 300.0]);
                        let u = *rng.pick(&["km", "kilometres", "kilometers"]);
                        (
                            format!("every {} {u}", fmt_num(n)),
                            SurfaceAnchor::Every { amount: q(n, "km") },
                        )
                    }
                    (2, _) => {
                        let (p, v) = *rng.pick(&[
                            ("every hour", 1.0),
                            ("every 2 hours", 2.0),
                            ("every two hours", 2.0),
                            ("every 3 hours", 3.0),
                            ("every 1.5 hours", 1.5),
                            ("every 4 hours", 4.0),
                        ]);
                        (p.to_string(), SurfaceAnchor::Every { amount: q(v, "h") })
                    }
                    _ => {
                        let n = *rng.pick(&[45.0, 60.0, 90.0, 120.0]);
                        (
                            format!("every {} minutes", fmt_num(n)),
                            SurfaceAnchor::Every {
                                amount: q(n, "min"),
                            },
                        )
                    }
                };
            }
            11 | 12 => {
                let (p, v) = *rng.pick(&[
                    ("halfway", 0.5),
                    ("half way there", 0.5),
                    ("midway", 0.5),
                    ("in the middle of the trip", 0.5),
                    ("halfway through the drive", 0.5),
                    ("a third of the way", 0.333),
                    ("a quarter of the way", 0.25),
                    ("two thirds of the way", 0.667),
                    ("three quarters of the way", 0.75),
                ]);
                return (p.into(), SurfaceAnchor::Fraction { value: v });
            }
            _ => {
                let Some(w) = wp else { continue };
                if rng.chance(0.5) {
                    let n = *rng.pick(&[10.0, 15.0, 20.0, 30.0]);
                    return (
                        format!("{} minutes after {w}", fmt_num(n)),
                        offset("waypoint:0", "after", n, "min"),
                    );
                }
                let (n, unit, u) = if miles {
                    (*rng.pick(&[10.0, 20.0, 30.0]), "mi", "miles")
                } else {
                    (*rng.pick(&[10.0, 20.0, 30.0, 50.0]), "km", "km")
                };
                return (
                    format!("{} {u} past {w}", fmt_num(n)),
                    offset("waypoint:0", "after", n, unit),
                );
            }
        }
    }
}

const VERBS: [&str; 8] = [
    "find", "I need", "stop at", "look for", "add", "show me", "I want", "get me",
];

/// One stop: phrase, label, and whether the phrase is a question.
fn stop(
    r: Region,
    rng: &mut Rng,
    cat: Option<&str>,
    dest: Option<&str>,
    wp: Option<&str>,
) -> (String, SurfaceStop, bool) {
    let (what_phrase, w) = match cat {
        Some(c) => what(r, rng, c),
        None => {
            let (p, t) = *rng.pick(text_places(r));
            (
                p.to_string(),
                SurfaceWhat {
                    text: Some(t.into()),
                    ..SurfaceWhat::default()
                },
            )
        }
    };
    let (anchor_phrase, a) = anchor(r, rng, dest, wp);
    let verb = *rng.pick(&VERBS);
    let bare = what_phrase
        .strip_prefix("a ")
        .or_else(|| what_phrase.strip_prefix("an "))
        .unwrap_or(&what_phrase)
        .to_string();
    let mut question = false;
    let (phrase, top_n) = match rng.below(100) {
        0..=7 => {
            let k = 2 + rng.below(4) as u8;
            let lead = rng.pick(&["give me", "show me", "list"]);
            (
                format!("{lead} {k} options for {bare} {anchor_phrase}"),
                Some(k),
            )
        }
        8..=24 => (format!("{anchor_phrase}, {verb} {what_phrase}"), None),
        25..=36 => {
            question = true;
            let q = match rng.below(6) {
                0 => format!("is there {what_phrase} {anchor_phrase}"),
                1 => format!("where can I get {what_phrase} {anchor_phrase}"),
                2 => format!("can you find {what_phrase} {anchor_phrase}"),
                3 => format!("could you add {what_phrase} {anchor_phrase}"),
                4 => format!("any {bare} {anchor_phrase}"),
                _ => format!("can we stop for {what_phrase} {anchor_phrase}"),
            };
            (q, None)
        }
        // Fragments, as people type them: "coffee in 20 min".
        37..=44 => (format!("{bare} {anchor_phrase}"), None),
        _ => (format!("{verb} {what_phrase} {anchor_phrase}"), None),
    };
    (
        phrase,
        SurfaceStop {
            id: None,
            what: w,
            anchor: a,
            top_n,
        },
        question,
    )
}

const SAVED: [&str; 6] = ["home", "work", "office", "mum's", "gym", "school"];

fn base_ctx(r: Region, rng: &mut Rng) -> TripContext {
    let saved = SAVED
        .iter()
        .filter(|_| rng.chance(0.35))
        .map(|n| SavedPlace {
            name: (*n).into(),
            location: Coords {
                lat: 40.0,
                lng: 5.0,
            },
        })
        .collect();
    TripContext {
        current_location: Some(Coords {
            lat: 40.0,
            lng: 5.0,
        }),
        locale: Some(locale(r, rng).into()),
        saved_places: saved,
        ..TripContext::default()
    }
}

fn saved_lead(rng: &mut Rng, name: &str) -> String {
    if rng.chance(0.3) {
        // Just the name: "Office. 2 parking options after 100 km".
        let mut c = name.chars();
        return c
            .next()
            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
            .unwrap_or_default();
    }
    match name {
        "home" => rng
            .pick(&[
                "Drive home",
                "Take me home",
                "Head home",
                "Going home",
                "Navigate home",
            ])
            .to_string(),
        "mum's" => rng
            .pick(&["Drive to mum's", "Take me to mum's", "Head to mum's place"])
            .to_string(),
        "office" | "gym" => format!(
            "{} the {name}",
            rng.pick(&["Drive to", "Take me to", "Head to", "Go to"])
        ),
        _ => format!(
            "{} {name}",
            rng.pick(&["Drive to", "Take me to", "Head to", "Go to"])
        ),
    }
}

/// Returns the noised text and whether it was lowercased (labels must then copy
/// places, brands and free text in lowercase too, exactly as written).
fn noise(rng: &mut Rng, s: &str, question: bool, names: &[String]) -> (String, bool) {
    let lower = rng.chance(0.2);
    let starts_with_name = names.iter().any(|n| s.starts_with(n.as_str()));
    let mut s = if lower {
        s.to_lowercase()
    } else if starts_with_name {
        // Capitalising would change a name the label copies verbatim.
        s.to_string()
    } else {
        let mut c = s.chars();
        c.next()
            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
            .unwrap_or_default()
    };
    if rng.chance(0.1) {
        s.push_str(rng.pick(&[" please", " pls", " plz", " thanks", " thx", " cheers"]));
    }
    if question && rng.chance(0.85) {
        s.push('?');
    } else {
        match rng.below(5) {
            0 => s.push('.'),
            1 => s.push('!'),
            _ => {}
        }
    }
    (s, lower)
}

/// Places, brands and free text in the label: the words that must stay verbatim.
fn label_names(i: &SurfaceIntent) -> Vec<String> {
    i.origin
        .iter()
        .chain(i.destination.iter())
        .chain(i.waypoints.iter())
        .filter(|p| !p.starts_with("saved:"))
        .cloned()
        .chain(
            i.stops
                .iter()
                .flat_map(|s| s.what.brand.iter().chain(s.what.text.iter()).cloned()),
        )
        .collect()
}

/// Chat shorthand and typos, as people really type. Words inside a place, brand or
/// free-text name are never touched, so the label still copies them verbatim.
fn roughen(rng: &mut Rng, text: &str, names: &[String]) -> String {
    let shorthand = rng.chance(0.12);
    let mut typos = if rng.chance(0.12) {
        1 + rng.below(2)
    } else {
        0
    };
    if !shorthand && typos == 0 {
        return text.to_string();
    }
    let protected: Vec<(usize, usize)> = names
        .iter()
        .flat_map(|n| {
            text.match_indices(n.as_str())
                .map(|(a, m)| (a, a + m.len()))
        })
        .collect();
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    for (k, tok) in text.split(' ').enumerate() {
        if k > 0 {
            out.push(' ');
        }
        let (start, end) = (pos, pos + tok.len());
        pos = end + 1;
        if protected.iter().any(|&(a, b)| a < end && start < b) {
            out.push_str(tok);
            continue;
        }
        let core = tok.trim_end_matches(|c: char| !c.is_alphanumeric());
        let tail = &tok[core.len()..];
        if shorthand && rng.chance(0.7) {
            let short = match core.to_lowercase().as_str() {
                "you" => Some("u"),
                "please" => Some(*rng.pick(&["pls", "plz"])),
                "minutes" => Some("mins"),
                "minute" => Some("min"),
                "hours" => Some("hrs"),
                "hour" => Some("hr"),
                "kilometres" | "kilometers" => Some("kms"),
                "and" => Some(*rng.pick(&["&", "n"])),
                "with" => Some("w/"),
                "thanks" => Some("thx"),
                "before" => Some("b4"),
                "want" => Some("wanna"),
                _ => None,
            };
            if let Some(s) = short {
                out.push_str(s);
                out.push_str(tail);
                continue;
            }
        }
        if typos > 0 && core.len() >= 4 && core.is_ascii() && rng.chance(0.25) {
            typos -= 1;
            let mut b: Vec<u8> = core.bytes().collect();
            let i = 1 + rng.below(b.len() - 2);
            match rng.below(3) {
                0 => b.swap(i, i + 1),
                1 => {
                    b.remove(i);
                }
                _ => b.insert(i, b[i]),
            }
            out.push_str(&String::from_utf8(b).unwrap_or_else(|_| core.to_string()));
            out.push_str(tail);
            continue;
        }
        out.push_str(tok);
    }
    out
}

fn lowercase_labels(i: &mut SurfaceIntent) {
    let low = |v: &mut Option<String>| {
        if let Some(x) = v
            && !x.starts_with("saved:")
        {
            *x = x.to_lowercase();
        }
    };
    low(&mut i.origin);
    low(&mut i.destination);
    for w in &mut i.waypoints {
        *w = w.to_lowercase();
    }
    for s in &mut i.stops {
        low(&mut s.what.brand);
        low(&mut s.what.text);
    }
}

struct Example {
    user: String,
    intent: SurfaceIntent,
    ctx: TripContext,
    negative: bool,
    region: &'static str,
}

fn trip(rng: &mut Rng) -> Example {
    let r = *rng.pick(&[Region::Europe, Region::Us, Region::India]);
    let mut ctx = base_ctx(r, rng);
    let mut intent = SurfaceIntent::default();
    let roll = rng.below(100);

    // Destination: named place (most), saved place, or none (already driving).
    let mut dest: Option<String> = None;
    let mut wp: Option<String> = None;
    let mut simple_dest = false;
    let lead = if roll < 12 && !ctx.saved_places.is_empty() {
        let s = ctx.saved_places[rng.below(ctx.saved_places.len())]
            .name
            .clone();
        intent.destination = Some(format!("saved:{s}"));
        saved_lead(rng, &s)
    } else if roll < 22 {
        ctx.active_destination = Some(Coords {
            lat: 41.0,
            lng: 6.0,
        });
        String::new()
    } else {
        let d = pick_place(r, rng);
        let o = loop {
            let o = pick_place(r, rng);
            if o != d {
                break o;
            }
        };
        let mut lead = if rng.chance(0.3) {
            intent.origin = Some(o.clone());
            rng.pick(&[
                format!("From {o} to {d}"),
                format!("Drive from {o} to {d}"),
                format!("Plan a route from {o} to {d}"),
                format!("Road trip from {o} to {d}"),
                format!("Starting from {o}, heading to {d}"),
                format!("Leaving {o} for {d}"),
                format!("{o} to {d}"),
                format!("From {o}, going to {d}"),
                format!("Starting in {o}, drive to {d}"),
            ])
            .clone()
        } else {
            simple_dest = true;
            rng.pick(&[
                format!("Drive to {d}"),
                format!("Take me to {d}"),
                format!("Navigate to {d}"),
                format!("I'm driving to {d}"),
                format!("We're heading to {d}"),
                format!("Route to {d}"),
                format!("Plan a trip to {d}"),
                format!("Going to {d}"),
                format!("On my way to {d}"),
                format!("Get me to {d}"),
                format!("Road trip to {d}"),
            ])
            .clone()
        };
        if rng.chance(0.15) {
            let w = loop {
                let w = pick_place(r, rng);
                if w != d && w != o {
                    break w;
                }
            };
            lead = format!(
                "{lead} {} {w}",
                rng.pick(&["via", "through", "passing through"])
            );
            intent.waypoints = vec![w.clone()];
            wp = Some(w);
            simple_dest = false;
        }
        intent.destination = Some(d.clone());
        dest = Some(d);
        lead
    };

    // 1-3 stops with different categories (or free text).
    let n = *rng.pick(&[1, 1, 1, 1, 2, 2, 2, 3]);
    let mut used = HashSet::new();
    let mut phrases = Vec::new();
    let mut last_question = false;
    for _ in 0..n {
        let cat = if rng.chance(0.08) {
            None
        } else {
            let c = *rng.pick(&CATEGORIES);
            if !used.insert(c) {
                continue;
            }
            Some(c)
        };
        let (p, s, question) = stop(r, rng, cat, dest.as_deref(), wp.as_deref());
        last_question = question;
        phrases.push(p);
        intent.stops.push(s);
    }

    let mut extras = Vec::new();
    if rng.chance(0.12) {
        let (p, a): (&str, &[&str]) = *rng.pick(&[
            ("avoiding tolls", &["tolls"][..]),
            ("with no toll roads", &["tolls"][..]),
            ("avoid highways", &["highways"][..]),
            ("avoid motorways", &["highways"][..]),
            ("no ferries", &["ferries"][..]),
            ("avoiding tolls and ferries", &["tolls", "ferries"][..]),
            ("skip the highways", &["highways"][..]),
            ("no toll roads", &["tolls"][..]),
            ("avoid the expressway", &["highways"][..]),
        ]);
        extras.push(p.to_string());
        intent.avoid = a.iter().map(|x| (*x).to_string()).collect();
    }
    if rng.chance(0.1) {
        let m = *rng.pick(&[2.0, 3.0, 5.0, 10.0, 15.0]);
        let s = fmt_num(m);
        extras.push(
            rng.pick(&[
                format!("no more than {s} minutes off the route"),
                format!("max {s} minute detour"),
                format!("detours under {s} minutes"),
                format!("keep detours within {s} min"),
            ])
            .clone(),
        );
        intent.max_detour = Some(q(m, "min"));
    }

    let joined = match phrases.len() {
        1 => phrases[0].clone(),
        2 => format!(
            "{}{}{}",
            phrases[0],
            rng.pick(&[" and ", ", and ", ", then ", ", also "]),
            phrases[1]
        ),
        _ => format!("{}, {} and {}", phrases[0], phrases[1], phrases[2]),
    };
    let roll = rng.below(100);
    let mut text = match (&dest, lead.is_empty()) {
        (_, true) => joined,
        (Some(d), false) if simple_dest && roll < 20 => format!("{joined} on the way to {d}"),
        // Destination last, as people often say it.
        (Some(d), false) if simple_dest && roll < 35 => rng
            .pick(&[
                format!("{joined}, then take me to {d}"),
                format!("{joined} and then drive to {d}"),
                format!("{joined}. Destination is {d}"),
                format!("{joined}, heading to {d}"),
                format!("{joined}, final stop {d}"),
                format!("{joined} - going to {d}"),
            ])
            .clone(),
        // A bare destination first: "Barcelona, coffee in 1 hour".
        (Some(d), false) if simple_dest && roll < 48 => rng
            .pick(&[
                format!("{d}, {joined}"),
                format!("{d}. {joined}"),
                format!("{d} - {joined}"),
                format!("to {d}, {joined}"),
                format!("{d} please, {joined}"),
            ])
            .clone(),
        _ => format!(
            "{lead}{}{joined}",
            rng.pick(&[", ", " and ", ". ", ": ", " - "])
        ),
    };
    let question = last_question && extras.is_empty();
    for e in extras {
        text = format!("{text}, {e}");
    }
    let names = label_names(&intent);
    if rng.chance(0.08) && !names.iter().any(|n| text.starts_with(n.as_str())) {
        let lead = *rng.pick(&[
            "hey, ",
            "ok so ",
            "quick one: ",
            "alright, ",
            "so ",
            "hi, ",
            "we're leaving now, ",
            "running late, ",
            "hey can you help, ",
        ]);
        let mut c = text.chars();
        let first = c
            .next()
            .map(|f| f.to_lowercase().collect::<String>())
            .unwrap_or_default();
        text = format!("{lead}{first}{}", c.as_str());
    }
    if !question && rng.chance(0.05) {
        text.push_str(rng.pick(&[
            " - we're in a hurry",
            " if possible",
            ", nothing fancy",
            " - kids are with us",
        ]));
    }
    let text = roughen(rng, &text, &names);
    let (user, lower) = noise(rng, &text, question, &names);
    if lower {
        lowercase_labels(&mut intent);
    }
    Example {
        user,
        intent,
        ctx,
        negative: false,
        region: region_name(r),
    }
}

fn negative(rng: &mut Rng) -> Example {
    let r = *rng.pick(&[Region::Europe, Region::Us, Region::India]);
    let city = rng.pick(places(r)).to_string();
    let text = rng
        .pick(&[
            "hello".to_string(),
            "hi there".into(),
            "what's the weather like?".into(),
            format!("what's the weather in {city}"),
            "tell me a joke".into(),
            "asdf qwerty".into(),
            "how far is the moon".into(),
            "translate good morning to French".into(),
            "who won the match yesterday".into(),
            "set an alarm for 7am".into(),
            "play some music".into(),
            format!("what time is it in {city}"),
            "thanks".into(),
            "ok".into(),
            "what can you do?".into(),
            format!("tell me about {city}"),
            "call mum".into(),
            format!("how many people live in {city}"),
            "turn on the AC".into(),
            "read my messages".into(),
            "find something".into(),
            "stop somewhere".into(),
            "take me there".into(),
            "navigate".into(),
            "I want to go".into(),
            "find it on the way".into(),
            "stop at the place".into(),
            "um".into(),
            "show me stuff on the route".into(),
            "the usual please".into(),
            "same as last time".into(),
            format!("is {city} nice?"),
            "how much fuel do I have".into(),
            "what's my ETA".into(),
            "?".into(),
            "qwfp arst zxcv".into(),
            "is there traffic on the way?".into(),
            "how long will it take".into(),
            "where am I".into(),
            "can u help me".into(),
            "r u there".into(),
            format!("how far is {city}"),
            "cancel the route".into(),
            "mute navigation".into(),
            "any good spots around".into(),
            "stop".into(),
        ])
        .clone();
    let ctx = base_ctx(r, rng);
    let (user, _) = noise(rng, &text, false, &[]);
    Example {
        user,
        intent: SurfaceIntent {
            error: Some("not_understood".into()),
            ..SurfaceIntent::default()
        },
        ctx,
        negative: true,
        region: "none",
    }
}

struct Gen {
    rng: Rng,
    seen: HashSet<String>,
    stats: BTreeMap<String, usize>,
    samples: Vec<(String, String)>,
    dropped: usize,
    excluded: usize,
}

impl Gen {
    fn rows(&mut self, count: usize, negative_rate: f64) -> Vec<serde_json::Value> {
        let mut rows = Vec::new();
        let mut attempts = 0;
        while rows.len() < count && attempts < count * 50 {
            attempts += 1;
            let ex = if self.rng.chance(negative_rate) {
                negative(&mut self.rng)
            } else {
                trip(&mut self.rng)
            };
            if !self.seen.insert(ex.user.to_lowercase()) {
                continue;
            }
            if label_names(&ex.intent).iter().any(|n| excluded(n)) {
                self.excluded += 1;
                continue;
            }
            // Labels must agree with the real compiler: positives compile, negatives do not.
            if compile(&ex.intent, &ex.ctx).is_ok() == ex.negative {
                self.dropped += 1;
                continue;
            }
            let Ok(input) = ParseInput::new(&ex.user, &ex.ctx) else {
                self.dropped += 1;
                continue;
            };
            let target = serde_json::to_string(&ex.intent).expect("serialize");
            if !holdout() {
                for n in label_names(&ex.intent) {
                    remember(&n);
                }
                self.count_stats(&ex);
                if self.samples.len() < 12 && self.rng.chance(0.01) {
                    self.samples.push((ex.user.clone(), target.clone()));
                }
            }
            rows.push(json!({
                "messages": [
                    { "role": "system", "content": COMPACT_SYSTEM_PROMPT },
                    { "role": "user", "content": input.user_message() },
                    { "role": "assistant", "content": target },
                ],
                "meta": { "region": ex.region, "active_route": ex.ctx.active_destination.is_some(), "unseen": holdout() },
                "saved": ex.ctx.saved_places.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
            }));
        }
        rows
    }

    fn count_stats(&mut self, ex: &Example) {
        for s in &ex.intent.stops {
            let k = match &s.anchor {
                SurfaceAnchor::Offset { from, .. } => {
                    format!("anchor offset:{}", from.split(':').next().unwrap_or(""))
                }
                SurfaceAnchor::Every { .. } => "anchor every".into(),
                SurfaceAnchor::Fraction { .. } => "anchor fraction".into(),
            };
            *self.stats.entry(k).or_default() += 1;
            let w = s
                .what
                .category
                .clone()
                .unwrap_or_else(|| "free_text".into());
            *self.stats.entry(format!("what {w}")).or_default() += 1;
            if s.what.brand.is_some() {
                *self.stats.entry("what (with brand)".into()).or_default() += 1;
            }
        }
        *self
            .stats
            .entry(format!("region {}", ex.region))
            .or_default() += 1;
        *self
            .stats
            .entry(format!("stops per query {}", ex.intent.stops.len()))
            .or_default() += 1;
        if ex.ctx.active_destination.is_some() {
            *self
                .stats
                .entry("context active route (no destination)".into())
                .or_default() += 1;
        }
        if ex.negative {
            *self.stats.entry("not_understood".into()).or_default() += 1;
        }
    }
}

fn write_jsonl(path: &str, rows: &[serde_json::Value]) {
    let mut s = String::new();
    for r in rows {
        let _ = writeln!(s, "{r}");
    }
    std::fs::write(path, s).expect("write jsonl");
}

fn main() {
    let mut args = std::env::args().skip(1);
    let count: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(40_000);
    let out_dir = args.next().unwrap_or_else(|| "finetune/data".into());
    std::fs::create_dir_all(&out_dir).expect("create output dir");

    let mut g = Gen {
        rng: Rng(0x9E37_79B9_7F4A_7C15),
        seen: HashSet::new(),
        stats: BTreeMap::new(),
        samples: vec![],
        dropped: 0,
        excluded: 0,
    };
    if let Ok(paths) = std::env::var("WAYLINGO_EXCLUDE") {
        load_excluded(&paths);
    }
    // Main pool: held-out places and brands never appear here.
    let mut rows = g.rows(count, 0.03);
    for i in (1..rows.len()).rev() {
        let j = g.rng.below(i + 1);
        rows.swap(i, j);
    }
    let n = rows.len();
    let (n_valid, n_test) = (n / 20, n / 20);
    write_jsonl(&format!("{out_dir}/test.jsonl"), &rows[..n_test]);
    write_jsonl(
        &format!("{out_dir}/valid.jsonl"),
        &rows[n_test..n_test + n_valid],
    );
    write_jsonl(&format!("{out_dir}/train.jsonl"), &rows[n_test + n_valid..]);

    // Unseen set: only held-out places and brands, to detect memorisation.
    HOLDOUT.with(|h| h.set(true));
    let unseen = g.rows(1_000, 0.0);
    write_jsonl(&format!("{out_dir}/test_unseen.jsonl"), &unseen);

    println!(
        "examples: {n} (train {}, valid {n_valid}, test {n_test}) + test_unseen {}; dropped {}, excluded {}",
        n - n_valid - n_test,
        unseen.len(),
        g.dropped,
        g.excluded
    );
    for (k, v) in &g.stats {
        println!("  {k}: {v}");
    }
    println!("samples:");
    for (u, t) in &g.samples {
        println!("  {u}\n    -> {t}");
    }
}
