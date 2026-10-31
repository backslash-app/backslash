//! Conversions typed in the search bar: `5 km in miles`, `72f to c`,
//! `2 GiB en MB`, `20 € en $`.
//!
//! Units are converted here; currencies need exchange rates, which the
//! caller provides (see [`Rates`]).

use std::collections::HashMap;

/// What the user asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Unit { value: f64, from: &'static Unit, to: &'static Unit },
    Currency { amount: f64, from: String, to: String },
}

#[derive(Debug, PartialEq)]
pub struct Unit {
    pub symbol: &'static str,
    names: &'static [&'static str],
    dimension: Dimension,
    /// Value in the dimension's base unit for 1 of this unit.
    factor: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dimension {
    Length,
    Mass,
    Temperature,
    Data,
    Time,
    Volume,
    Speed,
    Area,
}

macro_rules! unit {
    ($symbol:expr, $dim:ident, $factor:expr, [$($name:expr),*]) => {
        Unit { symbol: $symbol, names: &[$symbol $(, $name)*], dimension: Dimension::$dim, factor: $factor }
    };
}

static UNITS: &[Unit] = &[
    // Length (base: metre)
    unit!("mm", Length, 0.001, ["millimeter", "millimeters", "millimetre", "millimètre", "millimètres"]),
    unit!("cm", Length, 0.01, ["centimeter", "centimeters", "centimètre", "centimètres"]),
    unit!("m", Length, 1.0, ["meter", "meters", "metre", "mètre", "mètres"]),
    unit!("km", Length, 1000.0, ["kilometer", "kilometers", "kilomètre", "kilomètres"]),
    unit!("in", Length, 0.0254, ["inch", "inches", "pouce", "pouces", "\""]),
    unit!("ft", Length, 0.3048, ["foot", "feet", "pied", "pieds", "'"]),
    unit!("yd", Length, 0.9144, ["yard", "yards"]),
    unit!("mi", Length, 1609.344, ["mile", "miles"]),
    unit!("nmi", Length, 1852.0, ["nautical mile", "nautical miles", "mille marin"]),
    // Mass (base: kilogram)
    unit!("mg", Mass, 1e-6, ["milligram", "milligrams", "milligramme", "milligrammes"]),
    unit!("g", Mass, 0.001, ["gram", "grams", "gramme", "grammes"]),
    unit!("kg", Mass, 1.0, ["kilogram", "kilograms", "kilogramme", "kilogrammes", "kilo", "kilos"]),
    unit!("t", Mass, 1000.0, ["tonne", "tonnes", "ton", "tons"]),
    unit!("oz", Mass, 0.028349523125, ["ounce", "ounces", "once", "onces"]),
    unit!("lb", Mass, 0.45359237, ["lbs", "pound", "pounds", "livre", "livres"]),
    unit!("st", Mass, 6.35029318, ["stone", "stones"]),
    // Temperature (handled specially; factor unused)
    unit!("°C", Temperature, 1.0, ["c", "°c", "celsius", "degc"]),
    unit!("°F", Temperature, 1.0, ["f", "°f", "fahrenheit", "degf"]),
    unit!("K", Temperature, 1.0, ["k", "kelvin", "kelvins"]),
    // Data (base: byte)
    unit!("bit", Data, 0.125, ["bits"]),
    unit!("B", Data, 1.0, ["b", "byte", "bytes", "octet", "octets", "o"]),
    unit!("kB", Data, 1e3, ["kb", "ko", "kilobyte", "kilobytes", "kilooctet", "kilooctets"]),
    unit!("MB", Data, 1e6, ["mb", "mo", "megabyte", "megabytes", "mégaoctet", "mégaoctets"]),
    unit!("GB", Data, 1e9, ["gb", "go", "gigabyte", "gigabytes", "gigaoctet", "gigaoctets"]),
    unit!("TB", Data, 1e12, ["tb", "to", "terabyte", "terabytes", "téraoctet", "téraoctets"]),
    unit!("KiB", Data, 1024.0, ["kib", "kio", "kibibyte", "kibibytes"]),
    unit!("MiB", Data, 1048576.0, ["mib", "mio", "mebibyte", "mebibytes"]),
    unit!("GiB", Data, 1073741824.0, ["gib", "gio", "gibibyte", "gibibytes"]),
    unit!("TiB", Data, 1099511627776.0, ["tib", "tio", "tebibyte", "tebibytes"]),
    // Time (base: second)
    unit!("ms", Time, 0.001, ["millisecond", "milliseconds", "milliseconde", "millisecondes"]),
    unit!("s", Time, 1.0, ["sec", "secs", "second", "seconds", "seconde", "secondes"]),
    unit!("min", Time, 60.0, ["mins", "minute", "minutes"]),
    unit!("h", Time, 3600.0, ["hr", "hrs", "hour", "hours", "heure", "heures"]),
    unit!("d", Time, 86400.0, ["day", "days", "jour", "jours", "j"]),
    unit!("wk", Time, 604800.0, ["week", "weeks", "semaine", "semaines"]),
    unit!("yr", Time, 31557600.0, ["year", "years", "an", "ans", "année", "années"]),
    // Volume (base: litre)
    unit!("ml", Volume, 0.001, ["millilitre", "millilitres", "milliliter", "milliliters"]),
    unit!("cl", Volume, 0.01, ["centilitre", "centilitres"]),
    unit!("l", Volume, 1.0, ["litre", "litres", "liter", "liters"]),
    unit!("gal", Volume, 3.785411784, ["gallon", "gallons"]),
    unit!("qt", Volume, 0.946352946, ["quart", "quarts"]),
    unit!("cup", Volume, 0.2365882365, ["cups", "tasse", "tasses"]),
    unit!("fl oz", Volume, 0.0295735295625, ["floz", "fluid ounce", "fluid ounces"]),
    // Speed (base: metre per second)
    unit!("m/s", Speed, 1.0, ["mps"]),
    unit!("km/h", Speed, 1.0 / 3.6, ["kmh", "kph"]),
    unit!("mph", Speed, 0.44704, ["mi/h"]),
    unit!("kn", Speed, 1852.0 / 3600.0, ["knot", "knots", "noeud", "noeuds", "nœud", "nœuds"]),
    // Area (base: square metre)
    unit!("m²", Area, 1.0, ["m2", "sqm"]),
    unit!("km²", Area, 1e6, ["km2"]),
    unit!("ha", Area, 1e4, ["hectare", "hectares"]),
    unit!("acre", Area, 4046.8564224, ["acres"]),
    unit!("ft²", Area, 0.09290304, ["ft2", "sqft"]),
];

/// ISO codes accepted for currencies, and the symbols mapped to them.
const CURRENCIES: &[&str] = &[
    "EUR", "USD", "GBP", "JPY", "CHF", "CAD", "AUD", "NZD", "CNY", "HKD", "SGD", "SEK", "NOK", "DKK", "PLN", "CZK",
    "HUF", "RON", "BGN", "TRY", "ILS", "INR", "IDR", "KRW", "MXN", "BRL", "ZAR", "THB", "PHP", "MYR", "ISK",
];
const SYMBOLS: &[(&str, &str)] =
    &[("€", "EUR"), ("$", "USD"), ("£", "GBP"), ("¥", "JPY"), ("₹", "INR"), ("₩", "KRW"), ("₺", "TRY"), ("₪", "ILS")];

const SEPARATORS: &[&str] = &[" to ", " in ", " en ", " as ", " vers ", "->", "=>", " → ", "→", " = "];

fn find_unit(name: &str) -> Option<&'static Unit> {
    let name = name.trim();
    // Case matters for data units (mb vs Mb are rare enough to fold); try exact first.
    UNITS
        .iter()
        .find(|u| u.names.contains(&name))
        .or_else(|| UNITS.iter().find(|u| u.names.iter().any(|n| n.eq_ignore_ascii_case(name))))
}

fn find_currency(name: &str) -> Option<String> {
    let name = name.trim();
    if let Some((_, code)) = SYMBOLS.iter().find(|(s, _)| *s == name) {
        return Some(code.to_string());
    }
    let upper = name.to_ascii_uppercase();
    CURRENCIES.contains(&upper.as_str()).then_some(upper)
}

/// Splits `"5.5km"` / `"5,5 km"` / `"-40 °F"` into number and rest.
fn split_number(s: &str) -> Option<(f64, &str)> {
    let s = s.trim();
    // Currency symbols may come first: "$20 to €".
    for (symbol, _) in SYMBOLS {
        if let Some(rest) = s.strip_prefix(symbol) {
            let (n, tail) = split_number(rest)?;
            if tail.trim().is_empty() {
                return Some((n, symbol));
            }
        }
    }
    let end = s
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || c == ',' || (i == 0 && c == '-') || c == ' ' && i == 0))
        .map_or(s.len(), |(i, _)| i);
    let number = s[..end].trim().replace(',', ".");
    let value: f64 = number.parse().ok()?;
    Some((value, s[end..].trim()))
}

/// Recognizes a conversion. Returns `None` for anything else.
pub fn parse(query: &str) -> Option<Request> {
    let query = query.trim();
    if query.len() > 80 {
        return None;
    }
    let lower = query.to_lowercase();
    let (left, right) = SEPARATORS.iter().find_map(|sep| {
        let i = lower.find(sep)?;
        Some((&query[..i], &query[i + sep.len()..]))
    })?;
    let (value, from) = split_number(left)?;
    let to = right.trim();
    if from.is_empty() || to.is_empty() {
        return None;
    }
    if let (Some(from), Some(to)) = (find_unit(from), find_unit(to)) {
        if from.dimension == to.dimension {
            return Some(Request::Unit { value, from, to });
        }
        return None;
    }
    let (from, to) = (find_currency(from)?, find_currency(to)?);
    Some(Request::Currency { amount: value, from, to })
}

/// Converts a unit request.
pub fn convert(value: f64, from: &Unit, to: &Unit) -> f64 {
    if from.dimension == Dimension::Temperature {
        let kelvin = match from.symbol {
            "°C" => value + 273.15,
            "°F" => (value - 32.0) * 5.0 / 9.0 + 273.15,
            _ => value,
        };
        return match to.symbol {
            "°C" => kelvin - 273.15,
            "°F" => (kelvin - 273.15) * 9.0 / 5.0 + 32.0,
            _ => kelvin,
        };
    }
    value * from.factor / to.factor
}

/// Formats with at most 6 significant decimals and no float noise.
pub fn format(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    let magnitude = value.abs().log10().floor() as i32;
    let decimals = (5 - magnitude).clamp(0, 10) as usize;
    let s = format!("{value:.decimals$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    // Thousands separators make big results readable: 1 609 344.
    let (int, frac) = s.split_once('.').map_or((s.as_str(), None), |(i, f)| (i, Some(f)));
    let (sign, digits) = int.strip_prefix('-').map_or(("", int), |d| ("-", d));
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push('\u{202f}');
        }
        grouped.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

/// Exchange rates relative to one base currency, as published by a central
/// bank (the ECB's `{base, date, rates}` answer deserializes directly).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Rates {
    pub base: String,
    /// Date of the rates, `YYYY-MM-DD`.
    pub date: String,
    pub rates: HashMap<String, f64>,
    /// Unix time when they were fetched.
    #[serde(default)]
    pub fetched: u64,
    /// Who published them; empty means the European Central Bank.
    #[serde(default)]
    pub source: String,
}

/// A central bank's table as served by allratestoday.com
/// (`/api/open/central-bank/{bank}`): each row is `1 base = value quote`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BankTable {
    pub rate_date: String,
    pub rates: Vec<BankQuote>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct BankQuote {
    pub base: String,
    pub quote: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub value: f64,
}

impl Rates {
    /// Rates relative to `home` from a bank's table, whichever way the bank
    /// quotes its pairs. Only `reference` rows count where the bank also
    /// publishes buying and selling rates; `None` if nothing usable is left.
    pub fn from_bank_table(table: &BankTable, home: &str, source: &str) -> Option<Rates> {
        let mut rates = HashMap::new();
        for q in &table.rates {
            if q.value <= 0.0 || !["reference", "middle", "mid", "official"].contains(&q.kind.as_str()) {
                continue;
            }
            if q.base == home && q.quote != home {
                rates.insert(q.quote.clone(), q.value);
            } else if q.quote == home && q.base != home {
                rates.insert(q.base.clone(), 1.0 / q.value);
            }
        }
        (!rates.is_empty()).then(|| Rates {
            base: home.into(),
            date: table.rate_date.clone(),
            rates,
            fetched: 0,
            source: source.into(),
        })
    }

    fn rate(&self, code: &str) -> Option<f64> {
        if code == self.base {
            Some(1.0)
        } else {
            self.rates.get(code).copied()
        }
    }

    pub fn convert(&self, amount: f64, from: &str, to: &str) -> Option<f64> {
        Some(amount / self.rate(from)? * self.rate(to)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(q: &str) -> Option<String> {
        match parse(q)? {
            Request::Unit { value, from, to } => Some(format!("{} {}", format(convert(value, from, to)), to.symbol)),
            Request::Currency { amount, from, to } => Some(format!("{amount} {from}->{to}")),
        }
    }

    #[test]
    fn units() {
        assert_eq!(conv("5 km in miles").as_deref(), Some("3.10686 mi"));
        assert_eq!(conv("5km to mi").as_deref(), Some("3.10686 mi"));
        assert_eq!(conv("100 °F en c").as_deref(), Some("37.7778 °C"));
        assert_eq!(conv("-40 c to f").as_deref(), Some("-40 °F"));
        assert_eq!(conv("2 GiB en MB").as_deref(), Some("2\u{202f}147.48 MB"));
        assert_eq!(conv("1,5 h en min").as_deref(), Some("90 min"));
        assert_eq!(conv("3 mi -> m").as_deref(), Some("4\u{202f}828.03 m"));
        assert_eq!(conv("90 km/h in mph").as_deref(), Some("55.9234 mph"));
    }

    #[test]
    fn currencies_and_rejections() {
        assert_eq!(conv("20 € en $").as_deref(), Some("20 EUR->USD"));
        assert_eq!(conv("$15 to eur").as_deref(), Some("15 USD->EUR"));
        assert_eq!(conv("100 chf in jpy").as_deref(), Some("100 CHF->JPY"));
        for q in ["5 km in kg", "firefox", "fly to the moon", "5 km", "km to mi", "20 abc en def"] {
            assert_eq!(conv(q), None, "{q}");
        }
    }

    #[test]
    fn rates_cross_convert() {
        let rates = Rates {
            base: "EUR".into(),
            date: "2026-10-27".into(),
            rates: [("USD".to_string(), 1.1), ("GBP".to_string(), 0.85)].into(),
            fetched: 0,
            source: String::new(),
        };
        assert!((rates.convert(20.0, "EUR", "USD").unwrap() - 22.0).abs() < 1e-9);
        assert!((rates.convert(11.0, "USD", "EUR").unwrap() - 10.0).abs() < 1e-9);
        assert!((rates.convert(1.1, "USD", "GBP").unwrap() - 0.85).abs() < 1e-9);
        assert_eq!(rates.convert(1.0, "EUR", "XXX"), None);
    }

    #[test]
    fn rates_from_bank_table() {
        // Pairs quoted both ways, a buying rate and a nonsense value are
        // brought to one base.
        let table: BankTable = serde_json::from_str(
            r#"{"bank":"boe","rate_date":"2026-10-29","rates":[
                {"base":"GBP","quote":"USD","type":"reference","value":1.25},
                {"base":"EUR","quote":"GBP","type":"reference","value":0.8},
                {"base":"GBP","quote":"JPY","type":"buy","value":190.0},
                {"base":"GBP","quote":"CHF","type":"reference","value":0.0}]}"#,
        )
        .unwrap();
        let rates = Rates::from_bank_table(&table, "GBP", "Bank of England").unwrap();
        assert_eq!(rates.base, "GBP");
        assert_eq!(rates.date, "2026-10-29");
        assert_eq!(rates.source, "Bank of England");
        assert!((rates.convert(10.0, "EUR", "USD").unwrap() - 10.0).abs() < 1e-9);
        assert_eq!(rates.rates.len(), 2);
        assert!(Rates::from_bank_table(&table, "XXX", "").is_none());
    }
}
