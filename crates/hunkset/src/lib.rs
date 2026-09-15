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

/// One validated alias signature without its expression body.
#[derive(Clone, Debug)]
pub struct AliasSignature {
    name: String,
    parameters: Vec<String>,
}

impl AliasSignature {
    pub fn new(
        name: impl Into<String>,
        parameters: impl IntoIterator<Item = impl Into<String>>,
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
        Ok(Self { name, parameters })
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl AliasDefinition {
    pub fn new(
        name: impl Into<String>,
        parameters: impl IntoIterator<Item = impl Into<String>>,
        expression: &str,
    ) -> Result<Self, QueryError> {
        let signature = AliasSignature::new(name, parameters)?;
        Self::from_signature(signature, expression)
    }

    pub fn from_signature(signature: AliasSignature, expression: &str) -> Result<Self, QueryError> {
        Ok(Self {
            name: signature.name,
            parameters: signature.parameters,
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
        Expression::Operation { fields, .. } => {
            consume_expansion_nodes(
                remaining_nodes,
                fields
                    .iter()
                    .map(|field| pattern_expression_nodes(&field.pattern))
                    .sum(),
            )?;
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
        Expression::Operation { fields, .. } => {
            1 + fields
                .iter()
                .map(|field| 1 + pattern_expression_nodes(&field.pattern))
                .sum::<usize>()
        }
        _ => 1,
    }
}

fn pattern_expression_nodes(expression: &parser::PatternExpression) -> usize {
    use parser::PatternExpression;
    match expression {
        PatternExpression::Pattern(_) => 1,
        PatternExpression::Union(left, right)
        | PatternExpression::Intersection(left, right)
        | PatternExpression::Difference(left, right) => {
            1 + pattern_expression_nodes(left) + pattern_expression_nodes(right)
        }
        PatternExpression::Complement(inner) => 1 + pattern_expression_nodes(inner),
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
        Expression::Operation { kind, fields } => {
            matching_keys(units, |unit| operation_matches(*kind, fields, unit))
        }
        Expression::Id(id) => matching_keys(units, |unit| unit.occurrence_id.as_ref() == Some(id)),
        Expression::Call { .. } => unreachable!("aliases are expanded before evaluation"),
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

fn evaluate_pattern_expression(expression: &parser::PatternExpression, value: &str) -> bool {
    use parser::{Pattern, PatternExpression};

    match expression {
        PatternExpression::Pattern(pattern) => match pattern {
            Pattern::Substring(pattern) => value.contains(pattern),
            Pattern::Exact(pattern) => value == pattern,
            Pattern::Glob(pattern) => glob_matches(pattern, value),
            Pattern::Regex(pattern) => pattern.is_match(value),
        },
        PatternExpression::Union(left, right) => {
            evaluate_pattern_expression(left, value) || evaluate_pattern_expression(right, value)
        }
        PatternExpression::Intersection(left, right) => {
            evaluate_pattern_expression(left, value) && evaluate_pattern_expression(right, value)
        }
        PatternExpression::Difference(left, right) => {
            evaluate_pattern_expression(left, value) && !evaluate_pattern_expression(right, value)
        }
        PatternExpression::Complement(inner) => !evaluate_pattern_expression(inner, value),
    }
}

fn operation_matches(
    kind: parser::OperationKind,
    fields: &[parser::Field],
    unit: &SelectableUnit,
) -> bool {
    use parser::{FieldKind, OperationKind};
    let kind_matches = match kind {
        OperationKind::Changed => true,
        OperationKind::Added => {
            matches!(unit.change, Change::Creation { .. })
                || matches!(unit.change, Change::Binary)
                    && unit.old_path.is_none()
                    && unit.new_path.is_some()
                || added_side(&unit.change).is_some_and(|side| !side.is_empty())
        }
        OperationKind::Removed => {
            matches!(unit.change, Change::Deletion { .. })
                || matches!(unit.change, Change::Binary)
                    && unit.new_path.is_none()
                    && unit.old_path.is_some()
                || removed_side(&unit.change).is_some_and(|side| !side.is_empty())
        }
        OperationKind::Renamed => matches!(unit.change, Change::Rename),
        OperationKind::ModeChanged => matches!(unit.change, Change::Mode),
        OperationKind::BinaryChanged => matches!(unit.change, Change::Binary),
    };
    if !kind_matches {
        return false;
    }
    fields.iter().all(|field| match field.kind {
        FieldKind::Path => unit
            .old_path
            .iter()
            .chain(unit.new_path.iter())
            .any(|path| evaluate_pattern_expression(&field.pattern, path)),
        FieldKind::BeforePath | FieldKind::From => unit
            .old_path
            .as_deref()
            .is_some_and(|path| evaluate_pattern_expression(&field.pattern, path)),
        FieldKind::AfterPath | FieldKind::To => unit
            .new_path
            .as_deref()
            .is_some_and(|path| evaluate_pattern_expression(&field.pattern, path)),
        FieldKind::File => match kind {
            OperationKind::Added => {
                unit.old_path.is_none()
                    && unit
                        .new_path
                        .as_deref()
                        .is_some_and(|path| evaluate_pattern_expression(&field.pattern, path))
            }
            OperationKind::Removed => {
                unit.new_path.is_none()
                    && unit
                        .old_path
                        .as_deref()
                        .is_some_and(|path| evaluate_pattern_expression(&field.pattern, path))
            }
            _ => false,
        },
        FieldKind::Content => match kind {
            OperationKind::Changed => changed_content_sides(&unit.change)
                .into_iter()
                .any(|side| evaluate_pattern_expression(&field.pattern, side)),
            OperationKind::Added => added_content_side(&unit.change)
                .is_some_and(|side| evaluate_pattern_expression(&field.pattern, side)),
            OperationKind::Removed => removed_content_side(&unit.change)
                .is_some_and(|side| evaluate_pattern_expression(&field.pattern, side)),
            _ => false,
        },
    })
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

fn changed_content_sides(change: &Change) -> Vec<&str> {
    match change {
        Change::Text { removed, added } => [removed.as_str(), added.as_str()]
            .into_iter()
            .filter(|side| !side.is_empty())
            .collect(),
        Change::Creation { added } => vec![added],
        Change::Deletion { removed } => vec![removed],
        Change::Rename | Change::Mode | Change::Binary => Vec::new(),
    }
}

fn added_content_side(change: &Change) -> Option<&str> {
    match change {
        Change::Creation { added } => Some(added),
        Change::Text { added, .. } if !added.is_empty() => Some(added),
        _ => None,
    }
}

fn removed_content_side(change: &Change) -> Option<&str> {
    match change {
        Change::Deletion { removed } => Some(removed),
        Change::Text { removed, .. } if !removed.is_empty() => Some(removed),
        _ => None,
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

fn glob_matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.chars().collect::<Vec<_>>();
    let path = path.chars().collect::<Vec<_>>();
    let length = path.len();
    let mut rows = vec![vec![false; length + 1]; 4];
    rows[pattern.len() % 4][length] = true;
    for index in (0..pattern.len()).rev() {
        let mut current = vec![false; length + 1];
        let next = rows[(index + 1) % 4].clone();
        let double_star = pattern.get(index) == Some(&'*') && pattern.get(index + 1) == Some(&'*');
        let directory_star = double_star && pattern.get(index + 2) == Some(&'/');
        let suffix = if double_star {
            rows[(index + if directory_star { 3 } else { 2 }) % 4].clone()
        } else {
            Vec::new()
        };
        for position in (0..=length).rev() {
            current[position] = match pattern[index] {
                '*' if double_star => {
                    let at_boundary = !directory_star || position == 0 || path[position - 1] == '/';
                    at_boundary && suffix[position] || position < length && current[position + 1]
                }
                '*' => {
                    next[position]
                        || position < length && path[position] != '/' && current[position + 1]
                }
                '?' => position < length && path[position] != '/' && next[position + 1],
                literal => position < length && path[position] == literal && next[position + 1],
            };
        }
        rows[index % 4] = current;
    }
    rows[0][0]
}
