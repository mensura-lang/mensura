//! Small edit-distance "did you mean" suggestions for diagnostics.

/// Levenshtein edit distance.
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// The closest candidate within edit distance 2, if any.
pub(crate) fn did_you_mean(
    name: &str,
    candidates: impl IntoIterator<Item = String>,
) -> Option<String> {
    candidates
        .into_iter()
        .map(|c| (distance(name, &c), c))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

/// `"; did you mean `x`?"` or `""`.
pub(crate) fn suffix(name: &str, candidates: impl IntoIterator<Item = String>) -> String {
    match did_you_mean(name, candidates) {
        Some(c) => format!("; did you mean `{c}`?"),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_a_close_name() {
        let cands = ["readings".to_string(), "machines".to_string()];
        assert_eq!(did_you_mean("readngs", cands), Some("readings".to_string()));
    }

    #[test]
    fn no_suggestion_when_far() {
        let cands = ["readings".to_string()];
        assert_eq!(did_you_mean("xyz", cands), None);
    }

    #[test]
    fn suffix_formats_a_hint() {
        let cands = ["bind".to_string(), "split".to_string()];
        assert_eq!(suffix("bnid", cands), "; did you mean `bind`?".to_string());
    }
}
