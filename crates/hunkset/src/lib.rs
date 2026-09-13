//! Pure query evaluation over caller-supplied selectable change units.

mod parser;

use std::collections::HashSet;

pub use parser::QueryError;

/// A key that identifies one unit only for the duration of an evaluation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OccurrenceKey(usize);

impl OccurrenceKey {
    pub fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Change {
    Text { removed: String, added: String },
    Creation { added: String },
    Deletion { removed: String },
    Rename,
    Mode,
    Binary,
}

/// One distinct, valid occurrence in the caller's comparison scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectableUnit {
    old_path: Option<String>,
    new_path: Option<String>,
    change: Change,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidPath;

impl std::fmt::Display for InvalidPath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a selectable unit path cannot be empty")
    }
}

impl std::error::Error for InvalidPath {}

impl SelectableUnit {
    /// Construct one text block without unchanged context.
    pub fn text(
        old_path: impl Into<String>,
        new_path: impl Into<String>,
        removed: impl Into<String>,
        added: impl Into<String>,
    ) -> Result<Self, InvalidPath> {
        Self::existing_paths(
            old_path,
            new_path,
            Change::Text {
                removed: removed.into(),
                added: added.into(),
            },
        )
    }

    /// Construct one indivisible text-file creation, including empty files.
    pub fn creation(
        new_path: impl Into<String>,
        added: impl Into<String>,
    ) -> Result<Self, InvalidPath> {
        Ok(Self {
            old_path: None,
            new_path: Some(nonempty_path(new_path)?),
            change: Change::Creation {
                added: added.into(),
            },
        })
    }

    /// Construct one indivisible text-file deletion, including empty files.
    pub fn deletion(
        old_path: impl Into<String>,
        removed: impl Into<String>,
    ) -> Result<Self, InvalidPath> {
        Ok(Self {
            old_path: Some(nonempty_path(old_path)?),
            new_path: None,
            change: Change::Deletion {
                removed: removed.into(),
            },
        })
    }

    pub fn rename(
        old_path: impl Into<String>,
        new_path: impl Into<String>,
    ) -> Result<Self, InvalidPath> {
        Self::existing_paths(old_path, new_path, Change::Rename)
    }

    pub fn mode(
        old_path: impl Into<String>,
        new_path: impl Into<String>,
    ) -> Result<Self, InvalidPath> {
        Self::existing_paths(old_path, new_path, Change::Mode)
    }

    pub fn binary_existing(
        old_path: impl Into<String>,
        new_path: impl Into<String>,
    ) -> Result<Self, InvalidPath> {
        Self::existing_paths(old_path, new_path, Change::Binary)
    }

    pub fn binary_creation(new_path: impl Into<String>) -> Result<Self, InvalidPath> {
        Ok(Self {
            old_path: None,
            new_path: Some(nonempty_path(new_path)?),
            change: Change::Binary,
        })
    }

    pub fn binary_deletion(old_path: impl Into<String>) -> Result<Self, InvalidPath> {
        Ok(Self {
            old_path: Some(nonempty_path(old_path)?),
            new_path: None,
            change: Change::Binary,
        })
    }

    fn existing_paths(
        old_path: impl Into<String>,
        new_path: impl Into<String>,
        change: Change,
    ) -> Result<Self, InvalidPath> {
        Ok(Self {
            old_path: Some(nonempty_path(old_path)?),
            new_path: Some(nonempty_path(new_path)?),
            change,
        })
    }
}

fn nonempty_path(path: impl Into<String>) -> Result<String, InvalidPath> {
    let path = path.into();
    if path.is_empty() {
        Err(InvalidPath)
    } else {
        Ok(path)
    }
}

/// Evaluate `query` and return evaluation-local keys for selected occurrences.
pub fn evaluate(
    query: &str,
    units: &[SelectableUnit],
) -> Result<HashSet<OccurrenceKey>, QueryError> {
    let expression = parser::parse(query)?;
    let universe = (0..units.len()).map(OccurrenceKey).collect::<HashSet<_>>();
    Ok(evaluate_expression(&expression, units, &universe))
}

fn evaluate_expression(
    expression: &parser::Expression,
    units: &[SelectableUnit],
    universe: &HashSet<OccurrenceKey>,
) -> HashSet<OccurrenceKey> {
    use parser::Expression;

    match expression {
        Expression::All => universe.clone(),
        Expression::None => HashSet::new(),
        Expression::Files(fileset) => matching_keys(units, |unit| {
            unit.old_path
                .iter()
                .chain(unit.new_path.iter())
                .any(|path| evaluate_fileset(fileset, path))
        }),
        Expression::BeforeFiles(fileset) => matching_keys(units, |unit| {
            unit.old_path
                .as_deref()
                .is_some_and(|path| evaluate_fileset(fileset, path))
        }),
        Expression::AfterFiles(fileset) => matching_keys(units, |unit| {
            unit.new_path
                .as_deref()
                .is_some_and(|path| evaluate_fileset(fileset, path))
        }),
        Expression::Content(literal) => matching_keys(units, |unit| {
            changed_sides(&unit.change)
                .into_iter()
                .any(|side| side.contains(literal))
        }),
        Expression::Added(literal) => matching_keys(units, |unit| {
            added_side(&unit.change).is_some_and(|side| side.contains(literal))
        }),
        Expression::Removed(literal) => matching_keys(units, |unit| {
            removed_side(&unit.change).is_some_and(|side| side.contains(literal))
        }),
        Expression::Regex(regex) => matching_keys(units, |unit| {
            changed_sides(&unit.change)
                .into_iter()
                .any(|side| regex.is_match(side))
        }),
        Expression::AddedRegex(regex) => matching_keys(units, |unit| {
            added_side(&unit.change).is_some_and(|side| regex.is_match(side))
        }),
        Expression::RemovedRegex(regex) => matching_keys(units, |unit| {
            removed_side(&unit.change).is_some_and(|side| regex.is_match(side))
        }),
        Expression::Renames => matching_keys(units, |unit| matches!(unit.change, Change::Rename)),
        Expression::Modes => matching_keys(units, |unit| matches!(unit.change, Change::Mode)),
        Expression::Binaries => matching_keys(units, |unit| matches!(unit.change, Change::Binary)),
        Expression::Creations => {
            matching_keys(units, |unit| matches!(unit.change, Change::Creation { .. }))
        }
        Expression::Deletions => {
            matching_keys(units, |unit| matches!(unit.change, Change::Deletion { .. }))
        }
        Expression::Union(left, right) => {
            let mut selected = evaluate_expression(left, units, universe);
            selected.extend(evaluate_expression(right, units, universe));
            selected
        }
        Expression::Intersection(left, right) => {
            let left = evaluate_expression(left, units, universe);
            let right = evaluate_expression(right, units, universe);
            left.intersection(&right).copied().collect()
        }
        Expression::Difference(left, right) => {
            let left = evaluate_expression(left, units, universe);
            let right = evaluate_expression(right, units, universe);
            left.difference(&right).copied().collect()
        }
        Expression::Complement(inner) => universe
            .difference(&evaluate_expression(inner, units, universe))
            .copied()
            .collect(),
    }
}

fn evaluate_fileset(expression: &parser::FilesetExpression, path: &str) -> bool {
    use parser::FilesetExpression;

    match expression {
        FilesetExpression::Pattern(pattern) => fileset_matches(pattern, path),
        FilesetExpression::Union(left, right) => {
            evaluate_fileset(left, path) || evaluate_fileset(right, path)
        }
        FilesetExpression::Intersection(left, right) => {
            evaluate_fileset(left, path) && evaluate_fileset(right, path)
        }
        FilesetExpression::Difference(left, right) => {
            evaluate_fileset(left, path) && !evaluate_fileset(right, path)
        }
        FilesetExpression::Complement(inner) => !evaluate_fileset(inner, path),
    }
}

fn matching_keys(
    units: &[SelectableUnit],
    predicate: impl Fn(&SelectableUnit) -> bool,
) -> HashSet<OccurrenceKey> {
    units
        .iter()
        .enumerate()
        .filter_map(|(index, unit)| predicate(unit).then_some(OccurrenceKey(index)))
        .collect()
}

fn changed_sides(change: &Change) -> Vec<&str> {
    match change {
        Change::Text { removed, added } => vec![removed, added],
        Change::Creation { added } => vec![added],
        Change::Deletion { removed } => vec![removed],
        Change::Rename | Change::Mode | Change::Binary => Vec::new(),
    }
}

fn added_side(change: &Change) -> Option<&str> {
    match change {
        Change::Text { added, .. } | Change::Creation { added } => Some(added),
        Change::Deletion { .. } | Change::Rename | Change::Mode | Change::Binary => None,
    }
}

fn removed_side(change: &Change) -> Option<&str> {
    match change {
        Change::Text { removed, .. } | Change::Deletion { removed } => Some(removed),
        Change::Creation { .. } | Change::Rename | Change::Mode | Change::Binary => None,
    }
}

fn fileset_matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.chars().collect::<Vec<_>>();
    let path = path.chars().collect::<Vec<_>>();
    let mut answers = vec![vec![None; path.len() + 1]; pattern.len() + 1];
    fileset_matches_from(&pattern, &path, 0, 0, &mut answers)
}

fn fileset_matches_from(
    pattern: &[char],
    path: &[char],
    pattern_index: usize,
    path_index: usize,
    answers: &mut [Vec<Option<bool>>],
) -> bool {
    if let Some(answer) = answers[pattern_index][path_index] {
        return answer;
    }

    let answer = match pattern.get(pattern_index) {
        None => path_index == path.len(),
        Some('*') if pattern.get(pattern_index + 1) == Some(&'*') => {
            let suffix_index = if pattern.get(pattern_index + 2) == Some(&'/') {
                pattern_index + 3
            } else {
                pattern_index + 2
            };
            let can_end_directories = pattern.get(pattern_index + 2) != Some(&'/')
                || path_index == 0
                || path.get(path_index - 1) == Some(&'/');
            (can_end_directories
                && fileset_matches_from(pattern, path, suffix_index, path_index, answers))
                || (path_index < path.len()
                    && fileset_matches_from(pattern, path, pattern_index, path_index + 1, answers))
        }
        Some('*') => {
            fileset_matches_from(pattern, path, pattern_index + 1, path_index, answers)
                || (path
                    .get(path_index)
                    .is_some_and(|character| *character != '/')
                    && fileset_matches_from(pattern, path, pattern_index, path_index + 1, answers))
        }
        Some('?') => {
            path.get(path_index)
                .is_some_and(|character| *character != '/')
                && fileset_matches_from(pattern, path, pattern_index + 1, path_index + 1, answers)
        }
        Some(literal) => {
            path.get(path_index) == Some(literal)
                && fileset_matches_from(pattern, path, pattern_index + 1, path_index + 1, answers)
        }
    };
    answers[pattern_index][path_index] = Some(answer);
    answer
}
