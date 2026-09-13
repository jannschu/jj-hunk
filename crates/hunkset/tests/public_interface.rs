use hunkset::{
    evaluate, evaluate_with_aliases, AliasDefinition, AliasEnvironment, OccurrenceKey,
    SelectableUnit,
};
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
fn explicit_text_and_path_sides_are_independent() {
    assert_eq!(indices("added(\"timeout\")"), HashSet::from([0, 2, 8]));
    assert_eq!(indices("removed(\"timeout\")"), HashSet::from([0, 4, 8]));
    assert!(indices("before_files(\"archive/**\")").is_empty());
    assert_eq!(indices("after_files(\"archive/**\")"), HashSet::from([5]));
    assert_eq!(
        indices("before_files(\"src/client.rs\")"),
        HashSet::from([5])
    );
    assert!(indices("after_files(\"src/client.rs\")").is_empty());
}

#[test]
fn regex_matching_is_explicit_and_stays_within_one_changed_side() {
    assert_eq!(
        indices(r#"regex("timeout\\s*=\\s*2\\d")"#),
        HashSet::from([0, 8])
    );
    assert_eq!(
        indices(r#"added_regex("timeout\\s*=\\s*2\\d")"#),
        HashSet::from([0, 8])
    );
    assert_eq!(
        indices(r#"removed_regex("timeout\\s*=\\s*1\\d")"#),
        HashSet::from([0, 8])
    );
    assert!(indices(r#"regex("TIMEOUT")"#).is_empty());
    assert_eq!(
        indices(r#"regex("(?i)TIMEOUT")"#),
        HashSet::from([0, 2, 4, 8])
    );
    assert!(indices("content(\"timeout.*20\")").is_empty());

    let multiline =
        [SelectableUnit::text("notes.txt", "notes.txt", "alpha\nbeta", "gamma\ndelta").unwrap()];
    assert_eq!(
        evaluate(r#"removed_regex("alpha\nbeta")"#, &multiline)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        evaluate(r#"added_regex("gamma\ndelta")"#, &multiline)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        evaluate("added(\"gamma\\ndelta\")", &multiline)
            .unwrap()
            .len(),
        1
    );
    assert!(evaluate("content(\"beta\\ngamma\")", &multiline)
        .unwrap()
        .is_empty());
    assert!(evaluate(r#"regex("(?s)beta.*gamma")"#, &multiline)
        .unwrap()
        .is_empty());

    let metacharacters =
        [SelectableUnit::creation("expression.txt", "literal a+b and (group)").unwrap()];
    assert_eq!(
        evaluate(r#"regex("a\\+b")"#, &metacharacters)
            .unwrap()
            .len(),
        1
    );
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

    let invalid_regex = evaluate("all() | regex(\"[\")", &fixture()).unwrap_err();
    assert!(invalid_regex.message().contains("invalid regex"));
    assert_eq!(invalid_regex.offset(), 14);

    let unknown = evaluate("semantic()", &fixture()).unwrap_err();
    assert_eq!(unknown.message(), "unknown function or alias `semantic`");

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

#[test]
fn aliases_expand_structurally_with_named_and_set_parameters() {
    let aliases = AliasEnvironment::new([
        AliasDefinition::new("generated", Vec::<String>::new(), "files(\"src/new.rs\")").unwrap(),
        AliasDefinition::new(
            "handwritten",
            ["selection"],
            "(all() ~ generated()) & selection()",
        )
        .unwrap(),
        AliasDefinition::new(
            "under_src",
            ["selection"],
            "files(\"src/**\") & selection()",
        )
        .unwrap(),
    ])
    .unwrap();

    let aliased =
        evaluate_with_aliases("handwritten(content(\"timeout\"))", &fixture(), &aliases).unwrap();
    let expanded = evaluate(
        "(all() ~ files(\"src/new.rs\")) & content(\"timeout\")",
        &fixture(),
    )
    .unwrap();
    assert_eq!(aliased, expanded);

    let precedence = evaluate_with_aliases(
        "under_src(content(\"timeout\") | modes())",
        &fixture(),
        &aliases,
    )
    .unwrap()
    .into_iter()
    .map(OccurrenceKey::index)
    .collect::<HashSet<_>>();
    assert_eq!(precedence, HashSet::from([0, 2, 8]));
}

#[test]
fn alias_configuration_and_expansion_errors_are_actionable() {
    for builtin in [
        "all",
        "none",
        "files",
        "before_files",
        "after_files",
        "content",
        "added",
        "removed",
        "regex",
        "added_regex",
        "removed_regex",
        "renames",
        "modes",
        "binaries",
        "creations",
        "deletions",
    ] {
        let collision =
            AliasEnvironment::new([
                AliasDefinition::new(builtin, Vec::<String>::new(), "all()").unwrap()
            ])
            .unwrap_err();
        assert!(collision.message().contains("collides with a builtin"));
    }
    let duplicate_parameter =
        AliasDefinition::new("bad", ["selection", "selection"], "all()").unwrap_err();
    assert!(duplicate_parameter
        .message()
        .contains("duplicate alias parameter"));
    let builtin_parameter = AliasDefinition::new("bad", ["all"], "all()").unwrap_err();
    assert!(builtin_parameter
        .message()
        .contains("parameter `all` collides with a builtin"));

    let duplicate = AliasEnvironment::new([
        AliasDefinition::new("same", Vec::<String>::new(), "all()").unwrap(),
        AliasDefinition::new("same", Vec::<String>::new(), "none()").unwrap(),
    ])
    .unwrap_err();
    assert!(duplicate.message().contains("duplicate alias `same`"));

    let aliases = AliasEnvironment::new([
        AliasDefinition::new("one", ["selection"], "selection()").unwrap(),
        AliasDefinition::new("cycle_a", Vec::<String>::new(), "cycle_b()").unwrap(),
        AliasDefinition::new("cycle_b", Vec::<String>::new(), "cycle_a()").unwrap(),
    ])
    .unwrap();
    let unknown = evaluate_with_aliases("missing()", &fixture(), &aliases).unwrap_err();
    assert_eq!(unknown.message(), "unknown function or alias `missing`");
    let arity = evaluate_with_aliases("one()", &fixture(), &aliases).unwrap_err();
    assert!(arity.message().contains("expects 1 arguments, got 0"));
    let cycle = evaluate_with_aliases("cycle_a()", &fixture(), &aliases).unwrap_err();
    assert!(cycle.message().contains("cycle_a -> cycle_b -> cycle_a"));

    let definitions = (0..34)
        .map(|index| {
            let expression = if index == 33 {
                "all()".to_owned()
            } else {
                format!("alias_{}()", index + 1)
            };
            AliasDefinition::new(format!("alias_{index}"), Vec::<String>::new(), &expression)
                .unwrap()
        })
        .collect::<Vec<_>>();
    let deep = AliasEnvironment::new(definitions).unwrap();
    let limit = evaluate_with_aliases("alias_0()", &fixture(), &deep).unwrap_err();
    assert!(limit.message().contains("expansion limit"));

    let duplicating = AliasEnvironment::new([AliasDefinition::new(
        "duplicate",
        ["selection"],
        "selection() | selection()",
    )
    .unwrap()])
    .unwrap();
    let compound_fileset = (0..16)
        .map(|index| format!("\"path-{index}\""))
        .collect::<Vec<_>>()
        .join(" | ");
    let mut exponential = format!("files(({compound_fileset}))");
    for _ in 0..9 {
        exponential = format!("duplicate({exponential})");
    }
    let size_limit = evaluate_with_aliases(&exponential, &fixture(), &duplicating).unwrap_err();
    assert!(size_limit.message().contains("expanded-node limit"));

    let paths = AliasEnvironment::new([AliasDefinition::new(
        "paths",
        Vec::<String>::new(),
        &format!("files(({compound_fileset}))"),
    )
    .unwrap()])
    .unwrap();
    let repeated_named_alias = std::iter::repeat_n("paths()", 350)
        .collect::<Vec<_>>()
        .join(" | ");
    let named_limit = evaluate_with_aliases(&repeated_named_alias, &fixture(), &paths).unwrap_err();
    assert!(named_limit.message().contains("expanded-node limit"));
}
