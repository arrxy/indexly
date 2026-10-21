/* Query-dependent snippets: prefer windows containing more distinct query
terms, then more matches. Highlight whole tokens using the index tokenizer's
alphanumeric splitting and Unicode lowercase rules. Byte slicing always
uses character boundaries, and only top-ranked passages are processed. */
use std::collections::HashMap;

const WINDOW: usize = 240;
pub fn generate(text: &str, terms: &[String]) -> String {
    let text: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut boundaries: Vec<_> = text.char_indices().map(|(i, _)| i).collect();
    boundaries.push(text.len());
    let chars = boundaries.len() - 1;
    let term_ids: HashMap<&str, usize> = terms
        .iter()
        .enumerate()
        .map(|(i, t)| (t.as_str(), i))
        .collect();
    let mut matches = Vec::new();
    let mut start = None;
    for (offset, ch) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if ch.is_alphanumeric() {
            start.get_or_insert(offset);
        } else if let Some(begin) = start.take() {
            if let Some(&id) = term_ids.get(text[begin..offset].to_lowercase().as_str()) {
                matches.push((begin, offset, id));
            }
        }
    }
    let mut best = (0, 0, 0); // distinct terms, total hits, character start
    for &(begin, end, _) in &matches {
        let begin_char = boundaries.binary_search(&begin).unwrap();
        let end_char = boundaries.binary_search(&end).unwrap();
        // A match can sit at either edge of the best window. Considering
        // those edges as well as its center avoids missing separated terms.
        for candidate in [
            begin_char,
            end_char.saturating_sub(WINDOW),
            ((begin_char + end_char) / 2).saturating_sub(WINDOW / 2),
        ] {
            let window_start = candidate.min(chars.saturating_sub(WINDOW));
            let low = boundaries[window_start];
            let high = boundaries[(window_start + WINDOW).min(chars)];
            let mut present = vec![false; terms.len()];
            let mut hits = 0;
            for &(a, b, id) in &matches {
                if a >= low && b <= high {
                    present[id] = true;
                    hits += 1;
                }
            }
            let distinct = present.iter().filter(|&&v| v).count();
            if (distinct, hits) > (best.0, best.1)
                || ((distinct, hits) == (best.0, best.1) && window_start < best.2)
            {
                best = (distinct, hits, window_start);
            }
        }
    }
    let low = boundaries[best.2];
    let high = boundaries[(best.2 + WINDOW).min(chars)];
    let mut result = String::new();
    if low > 0 {
        result.push_str("... ");
    }
    let mut position = low;
    for &(begin, end, _) in &matches {
        if begin >= low && end <= high {
            result.push_str(&text[position..begin]);
            result.push('[');
            result.push_str(&text[begin..end]);
            result.push(']');
            position = end;
        }
    }
    result.push_str(&text[position..high]);
    if high < text.len() {
        result.push_str(" ...");
    }
    result.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selects_query_region_and_preserves_unicode() {
        let text = format!(
            "{} Café naïve 東京 rust search. {}",
            "unrelated ".repeat(100),
            "trailing ".repeat(100)
        );
        let terms = vec!["café".into(), "東京".into(), "rust".into()];
        let s = generate(&text, &terms);
        assert!(s.contains("[Café]"));
        assert!(s.contains("[東京]"));
        assert!(s.contains("[rust]"));
        assert!(s.starts_with("..."));
        assert!(s.chars().count() < 270);
    }
    #[test]
    fn covers_separated_query_terms_in_one_window() {
        let text = format!(
            "{}rust {} search{}",
            "x ".repeat(200),
            "x ".repeat(100),
            " x".repeat(200)
        );
        let snippet = generate(&text, &["rust".into(), "search".into()]);
        assert!(snippet.contains("[rust]"));
        assert!(snippet.contains("[search]"));
    }
    #[test]
    fn matches_whole_normalized_tokens() {
        assert_eq!(
            generate("RUST rusty rust!", &["rust".into()]),
            "[RUST] rusty [rust]!"
        );
        assert_eq!(generate("", &[]), "");
    }
}
