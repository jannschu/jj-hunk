use hunkset::{
    evaluate, evaluate_with_aliases, AliasDefinition, AliasEnvironment, OccurrenceId,
    OccurrenceKey, SelectableUnit,
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
    assert!(indices("changed(content:\"absent\")").is_empty());
}

#[test]
fn evaluates_globs_against_each_path_independently() {
    assert_eq!(
        indices("changed(path:\"src/**\")"),
        HashSet::from([0, 2, 3, 5, 8, 9, 10])
    );
    assert_eq!(indices("changed(path:\"old/**\")"), HashSet::from([4, 11]));
    assert_eq!(indices("changed(path:\"*.sh\")"), HashSet::from([6]));
    assert_eq!(
        indices("changed(path:(\"src/**\" | \"tests/**\") ~ \"**/new.rs\")"),
        HashSet::from([0, 1, 3, 5, 8, 9, 10])
    );
    assert!(indices("changed(path:~\"archive/**\")").contains(&5));
    assert!(!indices("all() ~ changed(path:\"archive/**\")").contains(&5));
    assert_eq!(
        indices("changed(path:\"src/**/*.rs\")"),
        HashSet::from([0, 2, 3, 5, 8, 9, 10])
    );
    assert_eq!(
        indices("changed(path:\"src/caf?.rs\")"),
        HashSet::from([10])
    );
    assert!(indices("changed(path:\"**/new.rs\")").contains(&2));
    assert!(indices("changed(path:glob:\"**/**/new.rs\")").contains(&2));
    assert!(indices("changed(path:glob:\"src/**/naive.rs\")").contains(&9));
    assert!(indices("changed(path:glob:\"src/caf?.rs\")").contains(&10));

    let midsegment = [SelectableUnit::mode("renew.rs", "renew.rs").unwrap()];
    assert!(evaluate("changed(path:\"**/new.rs\")", &midsegment)
        .unwrap()
        .is_empty());
}

#[test]
fn literal_content_is_case_sensitive_and_uses_only_changed_sides() {
    assert_eq!(
        indices("changed(content:\"timeout\")"),
        HashSet::from([0, 2, 4, 8])
    );
    assert!(indices("changed(content:\"Timeout\")").is_empty());
    assert_eq!(
        indices("changed(content:\"timeout = 10\")"),
        HashSet::from([0, 8])
    );
    assert_eq!(
        indices("changed(content:\"timeout = 20\")"),
        HashSet::from([0, 8])
    );
}

#[test]
fn explicit_text_and_path_sides_are_independent() {
    assert_eq!(
        indices("added(content:\"timeout\")"),
        HashSet::from([0, 2, 8])
    );
    assert_eq!(
        indices("removed(content:\"timeout\")"),
        HashSet::from([0, 4, 8])
    );
    assert!(indices("changed(before_path:\"archive/**\")").is_empty());
    assert_eq!(
        indices("changed(after_path:\"archive/**\")"),
        HashSet::from([5])
    );
    assert_eq!(
        indices("changed(before_path:\"src/client.rs\")"),
        HashSet::from([5])
    );
    assert!(indices("changed(after_path:\"src/client.rs\")").is_empty());
}

#[test]
fn regex_matching_is_explicit_and_stays_within_one_changed_side() {
    assert_eq!(
        indices(r#"changed(content:regex:"timeout\\s*=\\s*2\\d")"#),
        HashSet::from([0, 8])
    );
    assert_eq!(
        indices(r#"added(content:regex:"timeout\\s*=\\s*2\\d")"#),
        HashSet::from([0, 8])
    );
    assert_eq!(
        indices(r#"removed(content:regex:"timeout\\s*=\\s*1\\d")"#),
        HashSet::from([0, 8])
    );
    assert!(indices(r#"changed(content:regex:"TIMEOUT")"#).is_empty());
    assert_eq!(
        indices(r#"changed(content:regex:"(?i)TIMEOUT")"#),
        HashSet::from([0, 2, 4, 8])
    );
    assert!(indices("changed(content:\"timeout.*20\")").is_empty());

    let multiline =
        [SelectableUnit::text("notes.txt", "notes.txt", "alpha\nbeta", "gamma\ndelta").unwrap()];
    assert_eq!(
        evaluate(r#"removed(content:regex:"alpha\nbeta")"#, &multiline)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        evaluate(r#"added(content:regex:"gamma\ndelta")"#, &multiline)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        evaluate("added(content:\"gamma\\ndelta\")", &multiline)
            .unwrap()
            .len(),
        1
    );
    assert!(evaluate("changed(content:\"beta\\ngamma\")", &multiline)
        .unwrap()
        .is_empty());
    assert!(
        evaluate(r#"changed(content:regex:"(?s)beta.*gamma")"#, &multiline)
            .unwrap()
            .is_empty()
    );

    let metacharacters =
        [SelectableUnit::creation("expression.txt", "literal a+b and (group)").unwrap()];
    assert_eq!(
        evaluate(r#"changed(content:regex:"a\\+b")"#, &metacharacters)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn text_predicates_do_not_match_non_text_units() {
    assert!(indices("changed(content:\"name.rs\")").is_empty());
    assert!(indices("changed(content:\"script.sh\")").is_empty());
    assert!(indices("changed(content:\"image.png\")").is_empty());
    assert!(indices("all() ~ changed(content:\"timeout\")")
        .is_superset(&HashSet::from([3, 5, 6, 7, 11])));
}

#[test]
fn evaluates_operators_with_the_agreed_precedence_and_associativity() {
    assert_eq!(
        indices("changed(path:\"src/**\") & changed(content:\"timeout\") | changed(path:\"*.sh\")"),
        HashSet::from([0, 2, 6, 8])
    );
    assert_eq!(
        indices("all() ~ changed(path:\"src/**\") & changed(path:\"old/**\")"),
        HashSet::from([4, 11])
    );
    assert_eq!(
        indices("~changed(path:\"src/**\") & changed(path:\"old/**\")"),
        HashSet::from([4, 11])
    );
    assert_eq!(
        indices("changed(path:\"src/**\") & (changed(content:\"retry\") | changed(content:\"timeout\"))"),
        HashSet::from([0, 2, 8])
    );
}

#[test]
fn conjunction_requires_the_same_occurrence() {
    assert!(indices("changed(path:\"src/**\") & changed(content:\"retry\")").is_empty());
    assert_eq!(
        indices("changed(content:\"timeout = 10\") & changed(content:\"timeout = 20\")"),
        HashSet::from([0, 8])
    );
}

#[test]
fn duplicate_looking_occurrences_have_distinct_local_keys() {
    assert_eq!(
        indices("changed(content:\"timeout = 20\")"),
        HashSet::from([0, 8])
    );
}

#[test]
fn occurrence_ids_match_exactly_and_duplicates_are_rejected() {
    let first = OccurrenceId::parse(&format!("hunk-{}", "1".repeat(64))).unwrap();
    let second = OccurrenceId::parse(&format!("sha256:{}", "2".repeat(64))).unwrap();
    let units = vec![
        SelectableUnit::text("a.rs", "a.rs", "old", "new")
            .unwrap()
            .with_occurrence_id(first.clone()),
        SelectableUnit::mode("b.rs", "b.rs")
            .unwrap()
            .with_occurrence_id(second),
    ];

    let selected = evaluate(&format!("id(\"{first}\")"), &units).unwrap();
    assert_eq!(selected.iter().next().unwrap().index(), 0);
    assert_eq!(selected.len(), 1);
    assert!(evaluate(&format!("id(\"hunk-{}\")", "1".repeat(63)), &units).is_err());

    let duplicate = vec![
        SelectableUnit::creation("a.rs", "a")
            .unwrap()
            .with_occurrence_id(first.clone()),
        SelectableUnit::deletion("b.rs", "b")
            .unwrap()
            .with_occurrence_id(first),
    ];
    let error = evaluate("all()", &duplicate).unwrap_err();
    assert!(error.message().contains("duplicate occurrence id"));
}

#[test]
fn reports_invalid_syntax_unknown_functions_and_wrong_arguments() {
    let syntax = evaluate("all() |", &fixture()).unwrap_err();
    assert!(syntax.message().contains("expected a function"));
    assert!(syntax.offset() > 0);

    let invalid_regex = evaluate("all() | changed(content:regex:\"[\")", &fixture()).unwrap_err();
    assert!(invalid_regex.message().contains("invalid regex"));
    assert_eq!(invalid_regex.offset(), 30);

    let unknown = evaluate("semantic()", &fixture()).unwrap_err();
    assert_eq!(unknown.message(), "unknown function or alias `semantic`");

    let wrong_arguments = evaluate("changed(content:)", &fixture()).unwrap_err();
    assert!(
        wrong_arguments.message().contains("expected a function")
            || wrong_arguments.message().contains("expected a string")
    );

    let wrong_argument_type = evaluate("changed(path:all())", &fixture()).unwrap_err();
    assert!(wrong_argument_type.message().contains("expected `:`"));
}

#[test]
fn rejects_replaced_predicate_names() {
    for query in [
        "creations()",
        "deletions()",
        "files(\"src/**\")",
        "before_files(\"src/**\")",
        "after_files(\"src/**\")",
    ] {
        let error = evaluate(query, &fixture()).unwrap_err();
        assert!(
            error.message().contains("unknown function or alias")
                || error.message().contains("expected a function"),
            "{query}: {error}"
        );
    }

    assert!(evaluate("added(\"text\")", &fixture())
        .unwrap_err()
        .message()
        .contains("expected a function"));
    let removed = evaluate("removed(\"text\")", &fixture()).unwrap_err();
    assert!(
        removed.message().contains("unknown function or alias")
            || removed.message().contains("expected a function")
    );
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
    assert_eq!(indices("added(file:glob:\"**\")"), HashSet::from([2, 3]));
    assert_eq!(indices("removed(file:glob:\"**\")"), HashSet::from([4, 11]));
    assert_eq!(indices("renamed()"), HashSet::from([5]));
    assert_eq!(indices("mode_changed()"), HashSet::from([6]));
    assert_eq!(indices("binary_changed()"), HashSet::from([7]));
    assert!(indices("changed(content:\"timeout\") & renamed()").is_empty());
    assert_eq!(
        indices("~changed(content:\"timeout\") & (renamed() | mode_changed() | binary_changed())"),
        HashSet::from([5, 6, 7])
    );
}

#[test]
fn operation_first_added_removed_matrix_selects_whole_units() {
    assert_eq!(indices("changed()"), (0..fixture().len()).collect());
    assert_eq!(indices("added()"), HashSet::from([0, 1, 2, 3, 8, 9, 10]));
    assert_eq!(indices("removed()"), HashSet::from([0, 1, 4, 8, 9, 10, 11]));
    assert_eq!(
        indices("added(content:\"timeout\")"),
        HashSet::from([0, 2, 8])
    );
    assert_eq!(
        indices("removed(content:\"timeout\")"),
        HashSet::from([0, 4, 8])
    );
    assert_eq!(
        indices("added(file:glob:\"src/**\")"),
        HashSet::from([2, 3])
    );
    assert_eq!(
        indices("removed(file:glob:\"old/**\")"),
        HashSet::from([4, 11])
    );
    assert_eq!(
        indices("added(path:glob:\"src/**\", content:\"timeout\")"),
        HashSet::from([0, 2, 8])
    );
    assert!(indices("added(file:glob:\"src/empty.rs\", content:\"timeout\")").is_empty());
    assert_eq!(
        indices("added(content:\"timeout\") & removed(content:\"timeout\")"),
        HashSet::from([0, 8])
    );
}

#[test]
fn field_patterns_boolean_sides_and_modifiers_are_typed() {
    assert_eq!(
        indices("added(path:glob:\"src/**\" | glob:\"tests/**\", content:regex:\"timeout|retry\")"),
        HashSet::from([0, 1, 2, 8])
    );
    assert_eq!(
        indices("renamed(from:glob:\"src/**\", to:exact:\"archive/client.rs\")"),
        HashSet::from([5])
    );
    assert!(indices("renamed(from:glob:\"archive/**\")").is_empty());
    assert_eq!(
        indices("changed(path:regex:\"^src/.*\\\\.rs$\")"),
        HashSet::from([0, 2, 3, 5, 8, 9, 10])
    );
    assert_eq!(
        indices("changed(content:exact:\"timeout helper\")"),
        HashSet::from([2])
    );
    assert_eq!(
        indices("added(content:substring:\"timeout\")"),
        HashSet::from([0, 2, 8])
    );
    assert!(indices("added(file:glob:\"src/**\", before_path:~glob:\"old/**\")").is_empty());
    assert!(
        indices("removed(after_path:~glob:\"src/**\") & removed(file:glob:\"old/**\")").is_empty()
    );
}

#[test]
fn empty_file_content_is_present_but_missing_text_and_binary_sides_are_not() {
    let units = [
        SelectableUnit::creation("empty.txt", "").unwrap(),
        SelectableUnit::deletion("old-empty.txt", "").unwrap(),
        SelectableUnit::text("insert.txt", "insert.txt", "", "new").unwrap(),
        SelectableUnit::text("delete.txt", "delete.txt", "old", "").unwrap(),
        SelectableUnit::binary_creation("binary.bin").unwrap(),
        SelectableUnit::binary_deletion("old-binary.bin").unwrap(),
    ];
    let selected = |query: &str| {
        evaluate(query, &units)
            .unwrap()
            .into_iter()
            .map(OccurrenceKey::index)
            .collect::<HashSet<_>>()
    };
    assert_eq!(
        selected("added(file:glob:\"*.txt\", content:exact:\"\")"),
        HashSet::from([0])
    );
    assert_eq!(
        selected("removed(file:glob:\"*.txt\", content:regex:\"^$\")"),
        HashSet::from([1])
    );
    assert_eq!(
        selected("added(content:~exact:\"old\")"),
        HashSet::from([0, 2])
    );
    assert_eq!(
        selected("removed(content:~exact:\"new\")"),
        HashSet::from([1, 3])
    );
    assert_eq!(selected("added(file:glob:\"*.bin\")"), HashSet::from([4]));
    assert_eq!(selected("removed(file:glob:\"*.bin\")"), HashSet::from([5]));
    assert!(selected("added(file:glob:\"*.bin\", content:~exact:\"\")").is_empty());
}

#[test]
fn long_content_glob_does_not_recurse_or_allocate_a_matrix() {
    let long = "a".repeat(80_000);
    let units = [SelectableUnit::creation("long.txt", long).unwrap()];
    assert_eq!(
        evaluate("added(content:glob:\"*\")", &units).unwrap().len(),
        1
    );
    assert_eq!(
        evaluate("added(content:glob:\"**\")", &units)
            .unwrap()
            .len(),
        1
    );
    assert!(evaluate("added(content:glob:\"?\")", &units)
        .unwrap()
        .is_empty());
}

#[test]
fn strict_operation_fields_fail_with_offsets() {
    for query in [
        "changed(kind:\"text\")",
        "changed(content:\"timeout\", content:\"retry\")",
        "renamed(content:\"timeout\")",
        "mode_changed(file:\"src/**\")",
        "binary_changed(from:\"src/**\")",
        "added(to:\"src/new.rs\")",
        "removed(file:unknown:\"old/**\")",
        "changed(path:regex:\"[\")",
        "changed(content:all())",
    ] {
        let error = evaluate(query, &fixture()).unwrap_err();
        assert!(error.offset() > 0, "{query}: {error}");
    }
    for replaced in [
        "content(\"timeout\")",
        "glob(\"src/**\")",
        "creations()",
        "renames()",
    ] {
        assert!(evaluate(replaced, &fixture()).is_err(), "{replaced}");
    }
}

#[test]
fn aliases_expand_structurally_with_named_and_set_parameters() {
    let aliases = AliasEnvironment::new([
        AliasDefinition::new(
            "generated",
            Vec::<String>::new(),
            "changed(path:\"src/new.rs\")",
        )
        .unwrap(),
        AliasDefinition::new(
            "handwritten",
            ["selection"],
            "(all() ~ generated()) & selection()",
        )
        .unwrap(),
        AliasDefinition::new(
            "under_src",
            ["selection"],
            "changed(path:\"src/**\") & selection()",
        )
        .unwrap(),
    ])
    .unwrap();

    let aliased = evaluate_with_aliases(
        "handwritten(changed(content:\"timeout\"))",
        &fixture(),
        &aliases,
    )
    .unwrap();
    let expanded = evaluate(
        "(all() ~ changed(path:\"src/new.rs\")) & changed(content:\"timeout\")",
        &fixture(),
    )
    .unwrap();
    assert_eq!(aliased, expanded);

    let precedence = evaluate_with_aliases(
        "under_src(changed(content:\"timeout\") | mode_changed())",
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
        "changed",
        "added",
        "removed",
        "renamed",
        "mode_changed",
        "binary_changed",
        "id",
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
    let compound_glob_expression = (0..16)
        .map(|index| format!("\"path-{index}\""))
        .collect::<Vec<_>>()
        .join(" | ");
    let mut exponential = format!("changed(path:({compound_glob_expression}))");
    for _ in 0..9 {
        exponential = format!("duplicate({exponential})");
    }
    let size_limit = evaluate_with_aliases(&exponential, &fixture(), &duplicating).unwrap_err();
    assert!(size_limit.message().contains("expanded-node limit"));

    let paths = AliasEnvironment::new([AliasDefinition::new(
        "paths",
        Vec::<String>::new(),
        &format!("changed(path:({compound_glob_expression}))"),
    )
    .unwrap()])
    .unwrap();
    let repeated_named_alias = std::iter::repeat_n("paths()", 350)
        .collect::<Vec<_>>()
        .join(" | ");
    let named_limit = evaluate_with_aliases(&repeated_named_alias, &fixture(), &paths).unwrap_err();
    assert!(named_limit.message().contains("expanded-node limit"));
}
