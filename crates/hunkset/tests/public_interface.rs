use hunkset::{evaluate, OccurrenceKey, SelectableUnit};
use std::collections::HashSet;

fn fixture() -> Vec<SelectableUnit> {
    vec![
        SelectableUnit::text(
            "src/server.rs",
            "src/server.rs",
            "timeout = 10",
            "timeout = 20",
        )
        .unwrap(),
        SelectableUnit::text(
            "tests/server.rs",
            "tests/server.rs",
            "retry = false",
            "retry = true",
        )
        .unwrap(),
        SelectableUnit::creation("src/new.rs", "timeout helper").unwrap(),
        SelectableUnit::creation("src/empty.rs", "").unwrap(),
        SelectableUnit::deletion("old/removed.rs", "legacy timeout").unwrap(),
        SelectableUnit::rename("src/client.rs", "archive/client.rs").unwrap(),
        SelectableUnit::mode("script.sh", "script.sh").unwrap(),
        SelectableUnit::binary_existing("image.png", "image.png").unwrap(),
        SelectableUnit::text(
            "src/duplicate.rs",
            "src/duplicate.rs",
            "timeout = 10",
            "timeout = 20",
        )
        .unwrap(),
        SelectableUnit::text(
            "src/nested/naive.rs",
            "src/nested/naive.rs",
            "naive",
            "naive",
        )
        .unwrap(),
        SelectableUnit::text("src/café.rs", "src/café.rs", "cafe", "café").unwrap(),
        SelectableUnit::deletion("old/empty.rs", "").unwrap(),
    ]
}

fn indices(query: &str) -> HashSet<usize> {
    evaluate(query, &fixture())
        .unwrap()
        .into_iter()
        .map(OccurrenceKey::index)
        .collect()
}

#[test]
fn evaluates_all_none_and_valid_non_matches() {
    assert_eq!(indices("all()"), (0..fixture().len()).collect());
    assert!(indices("none()").is_empty());
    assert!(indices("content(\"absent\")").is_empty());
}

#[test]
fn evaluates_files_against_each_path_independently() {
    assert_eq!(
        indices("files(\"src/**\")"),
        HashSet::from([0, 2, 3, 5, 8, 9, 10])
    );
    assert_eq!(indices("files(\"old/**\")"), HashSet::from([4, 11]));
    assert_eq!(indices("files(\"*.sh\")"), HashSet::from([6]));
    assert_eq!(
        indices("files((\"src/**\" | \"tests/**\") ~ \"**/new.rs\")"),
        HashSet::from([0, 1, 3, 5, 8, 9, 10])
    );
    assert!(indices("files(~\"archive/**\")").contains(&5));
    assert!(!indices("all() ~ files(\"archive/**\")").contains(&5));
    assert_eq!(
        indices("files(\"src/**/*.rs\")"),
        HashSet::from([0, 2, 3, 5, 8, 9, 10])
    );
    assert_eq!(indices("files(\"src/caf?.rs\")"), HashSet::from([10]));
    assert!(indices("files(\"**/new.rs\")").contains(&2));

    let midsegment = [SelectableUnit::mode("renew.rs", "renew.rs").unwrap()];
    assert!(evaluate("files(\"**/new.rs\")", &midsegment)
        .unwrap()
        .is_empty());
}

#[test]
fn literal_content_is_case_sensitive_and_uses_only_changed_sides() {
    assert_eq!(indices("content(\"timeout\")"), HashSet::from([0, 2, 4, 8]));
    assert!(indices("content(\"Timeout\")").is_empty());
    assert_eq!(indices("content(\"timeout = 10\")"), HashSet::from([0, 8]));
    assert_eq!(indices("content(\"timeout = 20\")"), HashSet::from([0, 8]));
}

#[test]
fn text_predicates_do_not_match_non_text_units() {
    assert!(indices("content(\"name.rs\")").is_empty());
    assert!(indices("content(\"script.sh\")").is_empty());
    assert!(indices("content(\"image.png\")").is_empty());
    assert!(indices("all() ~ content(\"timeout\")").is_superset(&HashSet::from([3, 5, 6, 7, 11])));
}

#[test]
fn evaluates_operators_with_the_agreed_precedence_and_associativity() {
    assert_eq!(
        indices("files(\"src/**\") & content(\"timeout\") | files(\"*.sh\")"),
        HashSet::from([0, 2, 6, 8])
    );
    assert_eq!(
        indices("all() ~ files(\"src/**\") & files(\"old/**\")"),
        HashSet::from([4, 11])
    );
    assert_eq!(
        indices("~files(\"src/**\") & files(\"old/**\")"),
        HashSet::from([4, 11])
    );
    assert_eq!(
        indices("files(\"src/**\") & (content(\"retry\") | content(\"timeout\"))"),
        HashSet::from([0, 2, 8])
    );
}

#[test]
fn conjunction_requires_the_same_occurrence() {
    assert!(indices("files(\"src/**\") & content(\"retry\")").is_empty());
    assert_eq!(
        indices("content(\"timeout = 10\") & content(\"timeout = 20\")"),
        HashSet::from([0, 8])
    );
}

#[test]
fn duplicate_looking_occurrences_have_distinct_local_keys() {
    assert_eq!(indices("content(\"timeout = 20\")"), HashSet::from([0, 8]));
}

#[test]
fn reports_invalid_syntax_unknown_functions_and_wrong_arguments() {
    let syntax = evaluate("all() |", &fixture()).unwrap_err();
    assert!(syntax.message().contains("expected a function"));
    assert!(syntax.offset() > 0);

    let unknown = evaluate("regex(\"timeout\")", &fixture()).unwrap_err();
    assert_eq!(unknown.message(), "unknown function `regex`");

    let wrong_arguments = evaluate("content()", &fixture()).unwrap_err();
    assert_eq!(
        wrong_arguments.message(),
        "content() expects one string argument"
    );

    let wrong_argument_type = evaluate("files(all())", &fixture()).unwrap_err();
    assert!(wrong_argument_type
        .message()
        .contains("expected a string argument"));
}

#[test]
fn rejects_empty_paths_at_the_public_boundary() {
    assert_eq!(
        SelectableUnit::creation("", "content")
            .unwrap_err()
            .to_string(),
        "a selectable unit path cannot be empty"
    );
    assert!(SelectableUnit::text("old.rs", "", "old", "new").is_err());
    assert!(SelectableUnit::rename("", "new.rs").is_err());
    assert!(SelectableUnit::binary_deletion("").is_err());
}

#[test]
fn selects_file_unit_kinds_independently() {
    assert_eq!(indices("creations()"), HashSet::from([2, 3]));
    assert_eq!(indices("deletions()"), HashSet::from([4, 11]));
    assert_eq!(indices("renames()"), HashSet::from([5]));
    assert_eq!(indices("modes()"), HashSet::from([6]));
    assert_eq!(indices("binaries()"), HashSet::from([7]));
    assert!(indices("content(\"timeout\") & renames()").is_empty());
    assert_eq!(
        indices("~content(\"timeout\") & (renames() | modes() | binaries())"),
        HashSet::from([5, 6, 7])
    );
}
