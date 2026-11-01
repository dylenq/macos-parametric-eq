use crate::state::{Band, Curve, FilterKind, MAX_BANDS, MAX_GAIN_DB};
use std::io::Read;
use std::path::Path;

/// Largest file read as a configuration. AutoEq's run to a kilobyte; this leaves room for
/// long hand-written ones while keeping a stray video dropped on the window from being
/// read into memory.
const MAX_FILE_BYTES: u64 = 1 << 20;

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

/// A number as the file writes it. "nan" and "inf" parse as floats, but are not settings.
fn number(token: &str) -> Option<f32> {
    token.parse().ok().filter(|v: &f32| v.is_finite())
}

/// The number after `key`, as in `Fc 105 Hz` or `Gain -2.5 dB`.
fn value(tokens: &[&str], key: &str) -> Option<f32> {
    let at = tokens.iter().position(|t| t.eq_ignore_ascii_case(key))?;
    number(tokens.get(at + 1)?)
}

/// The Q of a bandwidth given in octaves, as in `BW Oct 1.0`.
fn bandwidth_q(tokens: &[&str]) -> Option<f32> {
    let at = tokens.iter().position(|t| t.eq_ignore_ascii_case("BW"))?;
    if !tokens.get(at + 1)?.eq_ignore_ascii_case("Oct") {
        return None;
    }
    let octaves = number(tokens.get(at + 2)?).filter(|&n| n > 0.0)?;
    let ratio = 2f32.powf(octaves);
    Some(ratio.sqrt() / (ratio - 1.0)).filter(|q| q.is_finite())
}

/// Parses `ON PK Fc 105 Hz Gain 5.5 dB Q 0.70`, the part of a filter line after the colon.
fn filter(tokens: &[&str]) -> Option<Band> {
    let mut band = Band::new(kind(tokens.get(1)?)?, value(tokens, "Fc")?);
    band.enabled = tokens[0].eq_ignore_ascii_case("ON");
    if band.kind.has_gain() {
        band.gain_db = value(tokens, "Gain")?;
    }
    if let Some(q) = value(tokens, "Q").or_else(|| bandwidth_q(tokens)) {
        band.q = q;
    }
    band.clamp();
    Some(band)
}

pub fn parse(text: &str) -> Result<Imported, String> {
    let mut curve = Curve::default();
    let mut skipped = 0;
    let mut recognized = false;
    for line in text.lines() {
        let Some((head, rest)) = line.split_once(':') else {
            continue;
        };
        let head = head.trim().to_ascii_lowercase();
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        if head == "preamp" {
            // Equalizer APO applies each preamp in turn, so they add up.
            if let Some(db) = tokens.first().and_then(|t| number(t)) {
                curve.preamp_db += db;
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
    curve.preamp_db = curve.preamp_db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB);
    if recognized {
        Ok(Imported { curve, skipped })
    } else {
        Err("No Equalizer APO filters found in that file".into())
    }
}

/// Text from a file in whatever encoding Windows editors save Equalizer APO files in:
/// UTF-8 with or without a byte order mark, or UTF-16 with one. Anything else is read as
/// UTF-8 with the odd character replaced, which leaves the ASCII settings intact.
fn decode(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        let units = rest.as_chunks::<2>().0.iter().map(|&pair| unit(pair));
        char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
    };
    match bytes {
        [0xef, 0xbb, 0xbf, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [0xff, 0xfe, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xfe, 0xff, rest @ ..] => utf16(rest, u16::from_be_bytes),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Reads a configuration file. Returns a preset name taken from the file name with it.
pub fn read(path: &Path) -> Result<(String, Imported), String> {
    let error = |error: std::io::Error| format!("Could not read file: {error}");
    let too_large = || Err("That file is too large to be an equalizer configuration".into());
    // A pipe or device would block or never end, and a big file is not a configuration.
    let meta = std::fs::metadata(path).map_err(error)?;
    if !meta.is_file() {
        return Err("Could not read file: not a regular file".into());
    }
    if meta.len() > MAX_FILE_BYTES {
        return too_large();
    }
    // Bounded too, in case the file grew since.
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(error)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return too_large();
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported");
    // AutoEq names its files "Sennheiser HD 650 ParametricEQ.txt".
    let name = stem.strip_suffix("ParametricEQ").unwrap_or(stem).trim();
    let name = if name.is_empty() { "Imported" } else { name };
    Ok((name.to_owned(), parse(&decode(&bytes))?))
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

    #[test]
    fn a_preamp_that_is_not_a_number_is_left_out() {
        for text in ["Preamp: nan dB\n", "Preamp: NaN dB\n", "Preamp: inf dB\n", "Preamp: -infinity dB\n"] {
            let imported = parse(&format!("{text}Filter 1: ON PK Fc 1000 Hz Gain 3 dB Q 1\n")).unwrap();
            assert_eq!(imported.curve.preamp_db, 0.0, "{text}");
        }
    }

    #[test]
    fn reads_files_saved_with_a_byte_order_mark_or_as_utf16() {
        let mut bom = "\u{feff}".to_owned();
        bom.push_str(AUTOEQ);
        assert_eq!(decode(bom.as_bytes()), AUTOEQ);
        assert_eq!(parse(&decode(bom.as_bytes())).unwrap().curve.preamp_db, -6.2);

        let le: Vec<u8> = [0xff, 0xfe].into_iter().chain(AUTOEQ.encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let be: Vec<u8> = [0xfe, 0xff].into_iter().chain(AUTOEQ.encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(decode(&le), AUTOEQ);
        assert_eq!(decode(&be), AUTOEQ);
        // Latin-1 from an old editor still reads, with the odd character replaced.
        assert!(decode(b"Preamp: -3 dB \xb0\n").starts_with("Preamp: -3 dB"));
    }

    #[test]
    fn preamps_add_up_and_bandwidth_becomes_q() {
        let imported = parse(
            "Preamp: -3 dB\nPreamp: -2.5 dB\n\
             Filter 1: ON PK Fc 1000 Hz Gain 3 dB BW Oct 1\n\
             Filter 2: ON PK Fc 2000 Hz Gain 3 dB Q 2 BW Oct 1\n",
        )
        .unwrap();
        assert_eq!(imported.curve.preamp_db, -5.5);
        let bands = &imported.curve.bands;
        assert!((bands[0].q - std::f32::consts::SQRT_2).abs() < 1e-4, "{}", bands[0].q);
        // Q wins when a line gives both.
        assert_eq!(bands[1].q, 2.0);
    }

    #[test]
    fn refuses_files_too_large_to_be_a_configuration() {
        let path = std::env::temp_dir().join("eq-test-large.txt");
        std::fs::write(&path, vec![b'#'; MAX_FILE_BYTES as usize + 1]).unwrap();
        let result = read(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(result.unwrap_err().contains("too large"));
        assert!(read(&std::env::temp_dir()).unwrap_err().contains("not a regular file"));
    }
}
