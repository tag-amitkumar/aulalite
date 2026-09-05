// crates/backend/src/services/plagiarism.rs
//! Pure, dependency-free text-similarity primitives for the plagiarism check.
//!
//! The pipeline is deliberately simple and self-contained (no external crates):
//!
//!   1. NORMALIZE — lowercase, strip punctuation to spaces, collapse runs of
//!      whitespace to a single space. This makes the comparison robust to
//!      capitalization, punctuation, and reformatting.
//!   2. SHINGLE — split the normalized text into words and build overlapping
//!      word k-grams (k=5). A k-gram captures local word order, so reordered or
//!      lightly edited passages still match on their shared spans.
//!   3. FINGERPRINT — hash each shingle with the std `DefaultHasher` into a
//!      `u64`, dedup, and sort. The resulting `Vec<u64>` is a compact,
//!      order-independent set we can persist (as a `BIGINT[]`) and compare
//!      cheaply with a merge walk.
//!
//! Similarity between two fingerprints is reported as both Jaccard
//! (|A∩B| / |A∪B|) and containment (|A∩B| / min(|A|,|B|)). Containment is the
//! better signal for catching a short copied passage embedded in a longer
//! submission; the handler uses the max of the two so neither a length mismatch
//! nor a partial copy hides a match.
//!
//! This compares one submission's text against the OTHER submissions of the
//! SAME assignment — never across assignments or tenants.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Number of words per shingle (word k-gram). 5 balances catching short copied
/// spans against being noisy on common short phrases.
pub const SHINGLE_K: usize = 5;

/// Normalize text for comparison: lowercase, replace every non-alphanumeric
/// char with a space, then collapse whitespace runs to single spaces and trim.
/// ASCII-fast but Unicode-correct (uses `char::is_alphanumeric`).
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_space = true; // leading: suppress a leading space
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lc in ch.to_lowercase() {
                out.push(lc);
            }
            prev_space = false;
        } else if !prev_space {
            out.push(' ');
            prev_space = true;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Split normalized text into its words. Returns an empty Vec for empty input.
fn words(normalized: &str) -> Vec<&str> {
    if normalized.is_empty() {
        return Vec::new();
    }
    normalized.split(' ').filter(|w| !w.is_empty()).collect()
}

/// Hash one shingle (a slice of words) into a u64 with the std hasher. Words are
/// hashed with a unit separator between them so `["ab","c"]` and `["a","bc"]`
/// don't collide.
fn hash_shingle(shingle: &[&str]) -> u64 {
    let mut h = DefaultHasher::new();
    for w in shingle {
        w.hash(&mut h);
        0x1fu8.hash(&mut h); // separator
    }
    h.finish()
}

/// Build the sorted, deduped fingerprint (`Vec<u64>`) for a piece of text.
///
/// When the text has fewer than `SHINGLE_K` words we fall back to a single
/// shingle of all the words, so short answers still get a (degenerate but
/// stable) fingerprint instead of an empty one. Empty/whitespace-only text
/// yields an empty fingerprint, which is treated as 0.0 similarity to anything.
pub fn fingerprint(text: &str) -> Vec<u64> {
    let normalized = normalize(text);
    let ws = words(&normalized);
    if ws.is_empty() {
        return Vec::new();
    }
    let mut hashes: Vec<u64> = if ws.len() < SHINGLE_K {
        vec![hash_shingle(&ws)]
    } else {
        ws.windows(SHINGLE_K).map(hash_shingle).collect()
    };
    hashes.sort_unstable();
    hashes.dedup();
    hashes
}

/// Size of the intersection of two SORTED, deduped fingerprints via a merge
/// walk. O(|a| + |b|), no allocation.
fn intersection_size(a: &[u64], b: &[u64]) -> usize {
    let (mut i, mut j, mut n) = (0usize, 0usize, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                n += 1;
                i += 1;
                j += 1;
            }
        }
    }
    n
}

/// Jaccard similarity |A∩B| / |A∪B| of two sorted fingerprints. Two empty
/// fingerprints are defined as 0.0 (no evidence of overlap, not "identical").
pub fn jaccard(a: &[u64], b: &[u64]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let inter = intersection_size(a, b);
    let union = a.len() + b.len() - inter;
    if union == 0 {
        0.0
    } else {
        inter as f64 / union as f64
    }
}

/// Containment |A∩B| / min(|A|,|B|) of two sorted fingerprints. Catches a short
/// copied passage embedded in a much longer submission, where Jaccard would be
/// diluted by the surrounding original text.
pub fn containment(a: &[u64], b: &[u64]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = intersection_size(a, b);
    let denom = a.len().min(b.len());
    inter as f64 / denom as f64
}

/// The similarity score the handler reports: the MAX of Jaccard and
/// containment, so neither a length mismatch (handled by containment) nor a
/// partial copy (handled by containment) nor a near-identical pair (handled by
/// Jaccard) is missed. Range `0.0..=1.0`.
pub fn similarity(a: &[u64], b: &[u64]) -> f64 {
    jaccard(a, b).max(containment(a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_lowercases_strips_punct_collapses_ws() {
        assert_eq!(
            normalize("  Hello,   WORLD!!!  This\tis\n\na Test.  "),
            "hello world this is a test"
        );
        assert_eq!(normalize("a---b___c"), "a b c");
        assert_eq!(normalize("   "), "");
        assert_eq!(normalize(""), "");
    }

    #[test]
    fn identical_text_scores_one() {
        let t = "the quick brown fox jumps over the lazy dog again and again";
        let a = fingerprint(t);
        let b = fingerprint(t);
        assert_eq!(jaccard(&a, &b), 1.0);
        assert_eq!(containment(&a, &b), 1.0);
        assert_eq!(similarity(&a, &b), 1.0);
    }

    #[test]
    fn identical_ignoring_case_and_punctuation_scores_one() {
        let a = fingerprint("The Quick, Brown Fox JUMPS over the lazy dog!");
        let b = fingerprint("the quick brown fox jumps over the lazy dog");
        assert_eq!(similarity(&a, &b), 1.0);
    }

    #[test]
    fn disjoint_text_scores_zero() {
        let a = fingerprint("alpha beta gamma delta epsilon zeta eta theta");
        let b = fingerprint("one two three four five six seven eight nine");
        assert_eq!(jaccard(&a, &b), 0.0);
        assert_eq!(containment(&a, &b), 0.0);
        assert_eq!(similarity(&a, &b), 0.0);
    }

    #[test]
    fn partial_overlap_scores_between_zero_and_one() {
        // Shared 6-word span "the quick brown fox jumps over" yields 2 shared
        // 5-grams; the rest diverges.
        let a = fingerprint("the quick brown fox jumps over a sleepy cat today");
        let b = fingerprint("the quick brown fox jumps over the lazy dog now");
        let j = jaccard(&a, &b);
        let c = containment(&a, &b);
        assert!(j > 0.0 && j < 1.0, "jaccard out of range: {j}");
        assert!(c > 0.0 && c < 1.0, "containment out of range: {c}");
        assert!(similarity(&a, &b) >= j);
    }

    #[test]
    fn containment_catches_short_passage_in_long_text() {
        let short = "plagiarism detection compares submissions for shared spans";
        let long = format!(
            "introduction paragraph with lots of original framing here {short} \
             and then a long conclusion with much more unrelated original content \
             padding padding padding to dilute the jaccard score substantially"
        );
        let a = fingerprint(short);
        let b = fingerprint(&long);
        let c = containment(&a, &b);
        let j = jaccard(&a, &b);
        // The short passage is fully contained, so containment is high while
        // Jaccard is dragged down by the long doc's unique shingles.
        assert!(c > j, "expected containment {c} > jaccard {j}");
        assert!(c > 0.8, "containment should be high: {c}");
    }

    #[test]
    fn empty_fingerprints_are_zero_not_one() {
        let e = fingerprint("   ...  ");
        assert!(e.is_empty());
        assert_eq!(jaccard(&e, &e), 0.0);
        assert_eq!(containment(&e, &e), 0.0);
        assert_eq!(similarity(&e, &e), 0.0);
    }

    #[test]
    fn short_text_below_k_words_still_fingerprints() {
        let a = fingerprint("hello world");
        let b = fingerprint("hello world");
        assert_eq!(a.len(), 1);
        assert_eq!(similarity(&a, &b), 1.0);
        let c = fingerprint("goodbye moon");
        assert_eq!(similarity(&a, &c), 0.0);
    }

    #[test]
    fn fingerprint_is_sorted_and_deduped() {
        let fp = fingerprint("a a a b b c the quick brown fox the quick brown fox");
        for w in fp.windows(2) {
            assert!(w[0] < w[1], "fingerprint not strictly sorted/deduped");
        }
    }
}
