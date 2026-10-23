use super::*;
fn generate(text: &str, terms: &[String]) -> String {
    SnippetGenerator::new(terms).generate(text)
}
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
fn uses_first_match_instead_of_scoring_windows() {
    let text = format!(
        "{}rust {} search{}",
        "x ".repeat(200),
        "x ".repeat(100),
        " x".repeat(200)
    );
    let snippet = generate(&text, &["rust".into(), "search".into()]);
    assert!(snippet.contains("[rust]"));
    assert!(!snippet.contains("[search]"));
}
#[test]
fn matches_whole_normalized_tokens() {
    assert_eq!(
        generate("RUST rusty rust!", &["rust".into()]),
        "[RUST] rusty [rust]!"
    );
    assert_eq!(generate("", &[]), "");
}

#[test]
fn unicode_case_changes_and_overlapping_patterns_preserve_whole_tokens() {
    assert_eq!(
        generate(
            "CAFÉ İ RUSTY rust",
            &[
                "café".into(),
                "i\u{307}".into(),
                "rust".into(),
                "rusty".into()
            ]
        ),
        "[CAFÉ] [İ] [RUSTY] [rust]"
    );
    assert_eq!(
        generate("Rust\nsearch\tfast", &["rust".into(), "search".into()]),
        "[Rust] [search] fast"
    );
}

#[test]
fn absent_terms_and_empty_query_use_a_utf8_safe_prefix() {
    let text = "é".repeat(300);
    let expected = format!("{} ...", "é".repeat(240));
    assert_eq!(generate(&text, &["missing".into()]), expected);
    assert_eq!(generate(&text, &[]), expected);
}

#[test]
fn first_match_near_end_keeps_at_most_sixty_preceding_characters() {
    let text = format!("{}rust", "é".repeat(300));
    let snippet = generate(&text, &["rust".into()]);
    // No word boundary exists before rust, so use a separator for the actual match.
    assert!(!snippet.contains("[rust]"));
    let text = format!("{} rust", "é".repeat(300));
    assert_eq!(
        generate(&text, &["rust".into()]),
        format!("... {} [rust]", "é".repeat(59))
    );
}
