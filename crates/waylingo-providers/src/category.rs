use waylingo_plan::{Category, What};

/// A plain search phrase for a category, used as the free-text query when a
/// provider needs one.
pub(crate) fn label(c: Category) -> &'static str {
    match c {
        Category::EvCharging => "electric vehicle charging station",
        Category::Fuel => "petrol station",
        Category::Restaurant => "restaurant",
        Category::FastFood => "fast food",
        Category::Coffee => "coffee shop",
        Category::RestArea => "rest area",
        Category::Hotel => "hotel",
        Category::Parking => "parking",
        Category::Supermarket => "supermarket",
        Category::Pharmacy => "pharmacy",
    }
}

/// Names a provider's category list may use for each Waylingo category, lowercase.
/// Matched against the category name and its synonyms.
pub(crate) fn provider_names(c: Category) -> &'static [&'static str] {
    match c {
        Category::EvCharging => &[
            "electric vehicle station",
            "electric vehicle charging station",
            "ev charging station",
        ],
        Category::Fuel => &["petrol station", "gas station", "fuel station"],
        Category::Restaurant => &["restaurant"],
        Category::FastFood => &["fast food"],
        Category::Coffee => &["coffee shop", "café", "cafe", "café/pub"],
        Category::RestArea => &["rest area"],
        Category::Hotel => &["hotel", "hotel/motel"],
        Category::Parking => &["parking garage", "open parking area", "parking"],
        Category::Supermarket => &["supermarkets & hypermarkets", "supermarket"],
        Category::Pharmacy => &["pharmacy"],
    }
}

/// The free-text query for a stop: brand, else text, else the category label.
pub(crate) fn query_text(w: &What) -> String {
    w.brand
        .clone()
        .or_else(|| w.text.clone())
        .or_else(|| w.category.map(|c| label(c).to_string()))
        .unwrap_or_else(|| "point of interest".to_string())
}
