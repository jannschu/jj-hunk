//! Pure query evaluation over caller-supplied selectable change units.

mod parser;

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{HashMap, HashSet};

pub use parser::QueryError;

pub const OCCURRENCE_ID_PREFIX: &str = "hunk-";

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct OccurrenceId(String);

impl OccurrenceId {
    pub fn parse(value: &str) -> Result<Self, InvalidOccurrenceId> {
        let value = value.trim();
        let hex = value
            .strip_prefix(OCCURRENCE_ID_PREFIX)
            .or_else(|| value.strip_prefix("id:"))
            .or_else(|| value.strip_prefix("sha:"))
            .or_else(|| value.strip_prefix("sha256:"))
            .ok_or(InvalidOccurrenceId)?;
        if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(InvalidOccurrenceId);
        }
        Ok(Self(format!(
            "{OCCURRENCE_ID_PREFIX}{}",
            hex.to_ascii_lowercase()
        )))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_sha256(digest: [u8; 32]) -> Self {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut value = String::with_capacity(OCCURRENCE_ID_PREFIX.len() + 64);
        value.push_str(OCCURRENCE_ID_PREFIX);
        for byte in digest {
            value.push(HEX[(byte >> 4) as usize] as char);
            value.push(HEX[(byte & 0x0f) as usize] as char);
        }
        Self(value)
    }
}

impl<'de> Deserialize<'de> for OccurrenceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Display for OccurrenceId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::ops::Deref for OccurrenceId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidOccurrenceId;

impl std::fmt::Display for InvalidOccurrenceId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .write_str("occurrence id must be hunk- followed by exactly 64 hexadecimal characters")
    }
}

impl std::error::Error for InvalidOccurrenceId {}

const MAX_ALIAS_EXPANSION_DEPTH: usize = 32;
const MAX_ALIAS_EXPANDED_NODES: usize = 10_000;

/// One parsed alias definition supplied by the caller.
#[derive(Clone, Debug)]
pub struct AliasDefinition {
    name: String,
    parameters: Vec<String>,
    expression: parser::Expression,
}

impl AliasDefinition {
    pub fn new(
        name: impl Into<String>,
        parameters: impl IntoIterator<Item = impl Into<String>>,
        expression: &str,
    ) -> Result<Self, QueryError> {
        let name = name.into();
        validate_alias_name(&name)?;
        let parameters = parameters.into_iter().map(Into::into).collect::<Vec<_>>();
        let mut seen = HashSet::new();
        for parameter in &parameters {
            validate_alias_name(parameter)?;
            if is_builtin(parameter) {
                return Err(QueryError::new(
                    0,
                    format!("alias parameter `{parameter}` collides with a builtin"),
                ));
            }
            if !seen.insert(parameter.clone()) {
                return Err(QueryError::new(
                    0,
                    format!("duplicate alias parameter `{parameter}`"),
                ));
            }
        }
        Ok(Self {
            name,
            parameters,
            expression: parser::parse(expression)?,
        })
    }
}

/// An explicit alias environment. The evaluator never reads configuration.
#[derive(Clone, Debug, Default)]
pub struct AliasEnvironment {
    definitions: HashMap<String, AliasDefinition>,
}

impl AliasEnvironment {
    pub fn new(definitions: impl IntoIterator<Item = AliasDefinition>) -> Result<Self, QueryError> {
        let mut aliases = Self::default();
        for definition in definitions {
            if is_builtin(&definition.name) {
                return Err(QueryError::new(
                    0,
                    format!("alias name `{}` collides with a builtin", definition.name),
                ));
            }
            let name = definition.name.clone();
            if aliases
                .definitions
                .insert(name.clone(), definition)
                .is_some()
            {
                return Err(QueryError::new(0, format!("duplicate alias `{name}`")));
            }
        }
        Ok(aliases)
    }
}

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
    occurrence_id: Option<OccurrenceId>,
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
            occurrence_id: None,
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
            occurrence_id: None,
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
            occurrence_id: None,
            old_path: None,
            new_path: Some(nonempty_path(new_path)?),
            change: Change::Binary,
        })
    }

    pub fn binary_deletion(old_path: impl Into<String>) -> Result<Self, InvalidPath> {
        Ok(Self {
            occurrence_id: None,
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
            occurrence_id: None,
            old_path: Some(nonempty_path(old_path)?),
            new_path: Some(nonempty_path(new_path)?),
            change,
        })
    }

    pub fn with_occurrence_id(mut self, occurrence_id: OccurrenceId) -> Self {
        self.occurrence_id = Some(occurrence_id);
        self
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
    evaluate_with_aliases(query, units, &AliasEnvironment::default())
}

pub fn evaluate_with_aliases(
    query: &str,
    units: &[SelectableUnit],
    aliases: &AliasEnvironment,
) -> Result<HashSet<OccurrenceKey>, QueryError> {
    let mut occurrence_ids = HashSet::new();
    for unit in units {
        if let Some(id) = &unit.occurrence_id {
            if !occurrence_ids.insert(id) {
                return Err(QueryError::new(
                    0,
                    format!("duplicate occurrence id `{id}`"),
                ));
            }
        }
    }
    let expression = parser::parse(query)?;
    let mut remaining_nodes = MAX_ALIAS_EXPANDED_NODES;
    let expression = expand_aliases(
        &expression,
        aliases,
        &HashMap::new(),
        &mut Vec::new(),
        0,
        &mut remaining_nodes,
    )?;
    let universe = (0..units.len()).map(OccurrenceKey).collect::<HashSet<_>>();
    Ok(evaluate_expression(&expression, units, &universe))
}

fn expand_aliases(
    expression: &parser::Expression,
    aliases: &AliasEnvironment,
    bindings: &HashMap<String, parser::Expression>,
    stack: &mut Vec<String>,
    depth: usize,
    remaining_nodes: &mut usize,
) -> Result<parser::Expression, QueryError> {
    use parser::Expression;

    if depth > MAX_ALIAS_EXPANSION_DEPTH {
        return Err(QueryError::new(0, "alias expansion limit exceeded"));
    }
    consume_expansion_nodes(remaining_nodes, 1)?;
    Ok(match expression {
        Expression::Call {
            name,
            arguments,
            offset,
        } => {
            if arguments.is_empty() {
                if let Some(bound) = bindings.get(name) {
                    consume_expansion_nodes(remaining_nodes, expression_nodes(bound))?;
                    return Ok(bound.clone());
                }
            }
            let definition = aliases.definitions.get(name).ok_or_else(|| {
                QueryError::new(*offset, format!("unknown function or alias `{name}`"))
            })?;
            if definition.parameters.len() != arguments.len() {
                return Err(QueryError::new(
                    *offset,
                    format!(
                        "alias `{name}` expects {} arguments, got {}",
                        definition.parameters.len(),
                        arguments.len()
                    ),
                ));
            }
            if stack.contains(name) {
                let mut cycle = stack.clone();
                cycle.push(name.clone());
                return Err(QueryError::new(
                    *offset,
                    format!("alias cycle: {}", cycle.join(" -> ")),
                ));
            }
            let expanded_arguments = arguments
                .iter()
                .map(|argument| {
                    expand_aliases(
                        argument,
                        aliases,
                        bindings,
                        stack,
                        depth + 1,
                        remaining_nodes,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let definition_bindings = definition
                .parameters
                .iter()
                .cloned()
                .zip(expanded_arguments)
                .collect::<HashMap<_, _>>();
            stack.push(name.clone());
            let expanded = expand_aliases(
                &definition.expression,
                aliases,
                &definition_bindings,
                stack,
                depth + 1,
                remaining_nodes,
            );
            stack.pop();
            expanded?
        }
        Expression::Union(left, right) => Expression::Union(
            Box::new(expand_aliases(
                left,
                aliases,
                bindings,
                stack,
                depth,
                remaining_nodes,
            )?),
            Box::new(expand_aliases(
                right,
                aliases,
                bindings,
                stack,
                depth,
                remaining_nodes,
            )?),
        ),
        Expression::Intersection(left, right) => Expression::Intersection(
            Box::new(expand_aliases(
                left,
                aliases,
                bindings,
                stack,
                depth,
                remaining_nodes,
            )?),
            Box::new(expand_aliases(
                right,
                aliases,
                bindings,
                stack,
                depth,
                remaining_nodes,
            )?),
        ),
        Expression::Difference(left, right) => Expression::Difference(
            Box::new(expand_aliases(
                left,
                aliases,
                bindings,
                stack,
                depth,
                remaining_nodes,
            )?),
            Box::new(expand_aliases(
                right,
                aliases,
                bindings,
                stack,
                depth,
                remaining_nodes,
            )?),
        ),
        Expression::Complement(inner) => Expression::Complement(Box::new(expand_aliases(
            inner,
            aliases,
            bindings,
            stack,
            depth,
            remaining_nodes,
        )?)),
        Expression::Files(fileset)
        | Expression::BeforeFiles(fileset)
        | Expression::AfterFiles(fileset) => {
            consume_expansion_nodes(remaining_nodes, fileset_nodes(fileset))?;
            expression.clone()
        }
        other => other.clone(),
    })
}

fn consume_expansion_nodes(remaining: &mut usize, count: usize) -> Result<(), QueryError> {
    *remaining = remaining
        .checked_sub(count)
        .ok_or_else(|| QueryError::new(0, "alias expanded-node limit exceeded"))?;
    Ok(())
}

fn expression_nodes(expression: &parser::Expression) -> usize {
    use parser::Expression;
    match expression {
        Expression::Union(left, right)
        | Expression::Intersection(left, right)
        | Expression::Difference(left, right) => {
            1 + expression_nodes(left) + expression_nodes(right)
        }
        Expression::Complement(inner) => 1 + expression_nodes(inner),
        Expression::Call { arguments, .. } => {
            1 + arguments.iter().map(expression_nodes).sum::<usize>()
        }
        Expression::Files(fileset)
        | Expression::BeforeFiles(fileset)
        | Expression::AfterFiles(fileset) => 1 + fileset_nodes(fileset),
        _ => 1,
    }
}

fn fileset_nodes(expression: &parser::FilesetExpression) -> usize {
    use parser::FilesetExpression;
    match expression {
        FilesetExpression::Pattern(_) => 1,
        FilesetExpression::Union(left, right)
        | FilesetExpression::Intersection(left, right)
        | FilesetExpression::Difference(left, right) => {
            1 + fileset_nodes(left) + fileset_nodes(right)
        }
        FilesetExpression::Complement(inner) => 1 + fileset_nodes(inner),
    }
}

fn validate_alias_name(name: &str) -> Result<(), QueryError> {
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
        || name.chars().next().unwrap().is_ascii_digit()
    {
        return Err(QueryError::new(0, format!("invalid alias name `{name}`")));
    }
    Ok(())
}

fn is_builtin(name: &str) -> bool {
    parser::builtin_argument(name).is_some()
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
        Expression::Id(id) => matching_keys(units, |unit| unit.occurrence_id.as_ref() == Some(id)),
        Expression::Call { .. } => unreachable!("aliases are expanded before evaluation"),
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
