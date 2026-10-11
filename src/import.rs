use crate::state::{Band, Curve, FilterKind, MAX_BANDS, MAX_GAIN_DB};
use std::path::Path;

/// A curve read from an Equalizer APO configuration, the format AutoEq publishes its
/// headphone corrections in (`ParametricEQ.txt`).
#[derive(Debug, PartialEq)]
pub struct Imported {
    pub curve: Curve,
    /// Filters left out: types this equalizer does not have, or more than it has room for.
    pub skipped: usize,
}

fn kind(name: &str) -> Option<FilterKind> {
    match name.to_ascii_uppercase().as_str() {
        "PK" | "PEQ" => Some(FilterKind::Peak),
        "LS" | "LSC" | "LSQ" => Some(FilterKind::LowShelf),
        "HS" | "HSC" | "HSQ" => Some(FilterKind::HighShelf),
        "LP" | "LPQ" => Some(FilterKind::LowPass),
        "HP" | "HPQ" => Some(FilterKind::HighPass),
        _ => None,
    }
}

/// The number after `key`, as in `Fc 105 Hz` or `Gain -2.5 dB`.
fn value(tokens: &[&str], key: &str) -> Option<f32> {
    let at = tokens.iter().position(|t| t.eq_ignore_ascii_case(key))?;
    tokens.get(at + 1)?.parse().ok().filter(|v: &f32| v.is_finite())
}

/// Parses `ON PK Fc 105 Hz Gain 5.5 dB Q 0.70`, the part of a filter line after the colon.
fn filter(tokens: &[&str]) -> Option<Band> {
    let mut band = Band::new(kind(tokens.get(1)?)?, value(tokens, "Fc")?);
    band.enabled = tokens[0].eq_ignore_ascii_case("ON");
    if band.kind.has_gain() {
        band.gain_db = value(tokens, "Gain")?;
    }
    if let Some(q) = value(tokens, "Q") {
        band.q = q;
    }
    band.clamp();
    Some(band)
}

pub fn parse(text: &str) -> Result<Imported, String> {
    let mut curve = Curve {
        bands: Vec::new(),
        preamp_db: 0.0,
    };
    let mut skipped = 0;
    let mut recognized = false;
    for line in text.lines() {
        let Some((head, rest)) = line.split_once(':') else {
            continue;
        };
        let head = head.trim().to_ascii_lowercase();
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        if head == "preamp" {
            if let Some(db) = tokens.first().and_then(|t| t.parse::<f32>().ok()) {
                curve.preamp_db = db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB);
                recognized = true;
            }
        } else if head.starts_with("filter") {
            match filter(&tokens) {
                Some(band) if curve.bands.len() < MAX_BANDS => curve.bands.push(band),
                _ => skipped += 1,
            }
            recognized = true;
        }
    }
    if recognized {
        Ok(Imported { curve, skipped })
    } else {
        Err("No Equalizer APO filters found in that file".into())
    }
}

/// Reads a configuration file. Returns a preset name taken from the file name with it.
pub fn read(path: &Path) -> Result<(String, Imported), String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("Could not read file: {error}"))?;
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported");
    // AutoEq names its files "Sennheiser HD 650 ParametricEQ.txt".
    let name = stem.strip_suffix("ParametricEQ").unwrap_or(stem).trim();
    let name = if name.is_empty() { "Imported" } else { name };
    Ok((name.to_owned(), parse(&text)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTOEQ: &str = "\
Preamp: -6.2 dB
Filter 1: ON LSC Fc 105 Hz Gain 5.5 dB Q 0.70
Filter 2: ON PK Fc 180 Hz Gain -2.3 dB Q 1.20
Filter 3: ON PK Fc 3250 Hz Gain 4.1 dB Q 2.85
Filter 4: ON HSC Fc 10000 Hz Gain -3.0 dB Q 0.70
";

    #[test]
    fn reads_an_autoeq_file() {
        let imported = parse(AUTOEQ).unwrap();
        assert_eq!(imported.skipped, 0);
        assert_eq!(imported.curve.preamp_db, -6.2);
        let bands = &imported.curve.bands;
        assert_eq!(bands.len(), 4);
        assert_eq!(bands[0].kind, FilterKind::LowShelf);
        assert_eq!((bands[1].freq, bands[1].gain_db, bands[1].q), (180.0, -2.3, 1.2));
        assert_eq!(bands[3].kind, FilterKind::HighShelf);
        assert!(bands.iter().all(|b| b.enabled));
    }

    #[test]
    fn handles_off_filters_pass_filters_and_missing_q() {
        let imported = parse(
            "Filter 1: OFF PK Fc 1000 Hz Gain 3 dB Q 1\r\n\
             Filter 2: ON HP Fc 30 Hz\r\n\
             filter: on lpq fc 18000 hz q 0.5\r\n",
        )
        .unwrap();
        let bands = &imported.curve.bands;
        assert!(!bands[0].enabled);
        assert_eq!(bands[1].kind, FilterKind::HighPass);
        assert_eq!(bands[1].q, std::f32::consts::FRAC_1_SQRT_2);
        assert_eq!((bands[2].kind, bands[2].q), (FilterKind::LowPass, 0.5));
        assert_eq!(imported.curve.preamp_db, 0.0);
    }

    #[test]
    fn counts_what_it_cannot_use() {
        let mut text = String::from("Filter 1: ON NO Fc 50 Hz\nFilter 2: ON PK Fc 1000 Hz\n");
        for _ in 0..MAX_BANDS + 2 {
            text.push_str("Filter: ON PK Fc 1000 Hz Gain 1 dB Q 1\n");
        }
        let imported = parse(&text).unwrap();
        assert_eq!(imported.curve.bands.len(), MAX_BANDS);
        // The notch, the peak with no gain, and the two that did not fit.
        assert_eq!(imported.skipped, 4);
    }

    #[test]
    fn clamps_values_out_of_range() {
        let imported = parse("Preamp: -90 dB\nFilter 1: ON PK Fc 5 Hz Gain 40 dB Q 100\n").unwrap();
        assert_eq!(imported.curve.preamp_db, -MAX_GAIN_DB);
        let band = imported.curve.bands[0];
        assert_eq!((band.freq, band.gain_db, band.q), (20.0, MAX_GAIN_DB, 18.0));
    }

    #[test]
    fn names_the_preset_after_the_file() {
        let path = std::env::temp_dir().join("eq-test Sennheiser HD 650 ParametricEQ.txt");
        std::fs::write(&path, AUTOEQ).unwrap();
        let (name, imported) = read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(name, "eq-test Sennheiser HD 650");
        assert_eq!(imported.curve.bands.len(), 4);
        assert!(read(&path).is_err());
    }

    #[test]
    fn rejects_files_that_are_something_else() {
        assert!(parse("").is_err());
        assert!(parse("Dear diary: today I boosted the bass\n").is_err());
    }
}
