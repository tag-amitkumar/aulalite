// crates/backend/src/services/slugger.rs
//! Pure-function slug generator. No IO.
//!
//! `slugify(title)` converts a free-form title to a lowercase ASCII slug:
//! whitespace → `-`, non-`[a-z0-9-]` characters dropped, leading/trailing
//! `-` trimmed, repeated `-` collapsed.
//!
//! `dedup(base, existing)` returns `base`, or `base-2`, `base-3`, ... if the
//! base is already in `existing`. The first available form is returned.

pub fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = true; // pretend a dash precedes the start so leading dashes get trimmed
    for ch in input.chars() {
        let folded = ascii_fold(ch);
        for c in folded.chars() {
            let lower = c.to_ascii_lowercase();
            if lower.is_ascii_alphanumeric() {
                out.push(lower);
                prev_dash = false;
            } else if !prev_dash {
                out.push('-');
                prev_dash = true;
            }
            // else: skip — collapse repeated separators
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn ascii_fold(ch: char) -> String {
    // Minimal Latin diacritic fold.
    match ch {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => "a".into(),
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => "A".into(),
        'è' | 'é' | 'ê' | 'ë' => "e".into(),
        'È' | 'É' | 'Ê' | 'Ë' => "E".into(),
        'ì' | 'í' | 'î' | 'ï' => "i".into(),
        'Ì' | 'Í' | 'Î' | 'Ï' => "I".into(),
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' => "o".into(),
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' => "O".into(),
        'ù' | 'ú' | 'û' | 'ü' => "u".into(),
        'Ù' | 'Ú' | 'Û' | 'Ü' => "U".into(),
        'ñ' => "n".into(),
        'Ñ' => "N".into(),
        'ç' => "c".into(),
        'Ç' => "C".into(),
        c if c.is_ascii() => c.to_string(),
        _ => String::new(), // drop non-ASCII we don't fold
    }
}

pub fn dedup(base: &str, existing: &[String]) -> String {
    if !existing.iter().any(|s| s == base) {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !existing.iter().any(|s| s == &candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_lowercases_and_dashes() {
        assert_eq!(slugify("Intro to Calculus"), "intro-to-calculus");
    }

    #[test]
    fn slugify_drops_punctuation() {
        assert_eq!(slugify("Math 101: Functions!"), "math-101-functions");
    }

    #[test]
    fn slugify_collapses_repeats() {
        assert_eq!(slugify("a   b---c"), "a-b-c");
    }

    #[test]
    fn slugify_trims_edges() {
        assert_eq!(slugify("---hello---"), "hello");
    }

    #[test]
    fn slugify_handles_empty() {
        assert_eq!(slugify(""), "");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn slugify_ascii_folds_simple_diacritics() {
        assert_eq!(slugify("Café au lait"), "cafe-au-lait");
        assert_eq!(slugify("Niño"), "nino");
    }

    #[test]
    fn dedup_returns_base_when_unused() {
        let used: Vec<String> = vec![];
        assert_eq!(dedup("intro", &used), "intro");
    }

    #[test]
    fn dedup_appends_2_then_3() {
        assert_eq!(dedup("intro", &["intro".into()]), "intro-2");
        assert_eq!(
            dedup("intro", &["intro".into(), "intro-2".into()]),
            "intro-3"
        );
    }
}
