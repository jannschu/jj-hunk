use std::error::Error;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BuiltinArgument {
    None,
    String,
    Fields,
}

pub(crate) fn builtin_argument(name: &str) -> Option<BuiltinArgument> {
    match name {
        "all" | "none" => Some(BuiltinArgument::None),
        "id" => Some(BuiltinArgument::String),
        "changed" | "added" | "removed" | "renamed" | "mode_changed" | "binary_changed" => {
            Some(BuiltinArgument::Fields)
        }
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Expression {
    All,
    None,
    Operation {
        kind: OperationKind,
        fields: Vec<Field>,
    },
    Id(crate::OccurrenceId),
    Call {
        name: String,
        arguments: Vec<Expression>,
        offset: usize,
    },
    Union(Box<Expression>, Box<Expression>),
    Intersection(Box<Expression>, Box<Expression>),
    Difference(Box<Expression>, Box<Expression>),
    Complement(Box<Expression>),
}

#[derive(Clone, Debug)]
pub(crate) enum PatternExpression {
    Pattern(Pattern),
    Union(Box<PatternExpression>, Box<PatternExpression>),
    Intersection(Box<PatternExpression>, Box<PatternExpression>),
    Difference(Box<PatternExpression>, Box<PatternExpression>),
    Complement(Box<PatternExpression>),
}

#[derive(Clone, Debug)]
pub(crate) enum Pattern {
    Substring(String),
    Exact(String),
    Glob(String),
    Regex(regex::Regex),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OperationKind {
    Changed,
    Added,
    Removed,
    Renamed,
    ModeChanged,
    BinaryChanged,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum FieldKind {
    Path,
    BeforePath,
    AfterPath,
    File,
    Content,
    From,
    To,
}

#[derive(Clone, Debug)]
pub(crate) struct Field {
    pub kind: FieldKind,
    pub pattern: PatternExpression,
}

#[derive(Clone, Copy)]
enum DefaultPattern {
    Glob,
    Substring,
}

/// A query parse error with a byte offset into the original query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryError {
    offset: usize,
    message: String,
}

impl QueryError {
    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn new(offset: usize, message: impl Into<String>) -> Self {
        Self {
            offset,
            message: message.into(),
        }
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} at byte {}", self.message, self.offset)
    }
}

impl Error for QueryError {}

pub(crate) fn parse(query: &str) -> Result<Expression, QueryError> {
    let mut parser = Parser { query, offset: 0 };
    let expression = parser.parse_union()?;
    parser.skip_whitespace();
    if parser.offset != query.len() {
        return Err(QueryError::new(
            parser.offset,
            format!("unexpected token `{}`", parser.current_character().unwrap()),
        ));
    }
    Ok(expression)
}

struct Parser<'a> {
    query: &'a str,
    offset: usize,
}

struct StringArgument {
    value: String,
    offset: usize,
}

impl Parser<'_> {
    fn parse_union(&mut self) -> Result<Expression, QueryError> {
        let mut expression = self.parse_intersection_or_difference()?;
        while self.consume('|') {
            expression = Expression::Union(
                Box::new(expression),
                Box::new(self.parse_intersection_or_difference()?),
            );
        }
        Ok(expression)
    }

    fn parse_intersection_or_difference(&mut self) -> Result<Expression, QueryError> {
        let mut expression = self.parse_complement()?;
        loop {
            if self.consume('&') {
                expression = Expression::Intersection(
                    Box::new(expression),
                    Box::new(self.parse_complement()?),
                );
            } else if self.consume('~') {
                expression = Expression::Difference(
                    Box::new(expression),
                    Box::new(self.parse_complement()?),
                );
            } else {
                return Ok(expression);
            }
        }
    }

    fn parse_complement(&mut self) -> Result<Expression, QueryError> {
        if self.consume('~') {
            Ok(Expression::Complement(Box::new(self.parse_complement()?)))
        } else {
            self.parse_primary()
        }
    }

    fn parse_primary(&mut self) -> Result<Expression, QueryError> {
        self.skip_whitespace();
        if self.consume_raw('(') {
            let expression = self.parse_union()?;
            self.expect(')')?;
            return Ok(expression);
        }

        let function_offset = self.offset;
        let name = self.parse_identifier()?;
        self.expect('(')?;
        if builtin_argument(&name) == Some(BuiltinArgument::Fields) {
            let kind = match name.as_str() {
                "changed" => OperationKind::Changed,
                "added" => OperationKind::Added,
                "removed" => OperationKind::Removed,
                "renamed" => OperationKind::Renamed,
                "mode_changed" => OperationKind::ModeChanged,
                "binary_changed" => OperationKind::BinaryChanged,
                _ => unreachable!(),
            };
            let fields = self.parse_fields(kind)?;
            self.expect(')')?;
            return Ok(Expression::Operation { kind, fields });
        }
        if builtin_argument(&name).is_none() {
            let arguments = self.parse_expression_arguments()?;
            self.expect(')')?;
            return Ok(Expression::Call {
                name,
                arguments,
                offset: function_offset,
            });
        }
        let arguments = self.parse_arguments()?;
        self.expect(')')?;

        match (name.as_str(), arguments.as_slice()) {
            ("all", []) => Ok(Expression::All),
            ("none", []) => Ok(Expression::None),
            ("id", [argument]) => Ok(Expression::Id(
                crate::OccurrenceId::parse(&argument.value)
                    .map_err(|error| QueryError::new(argument.offset, error.to_string()))?,
            )),
            ("all" | "none", _) => Err(QueryError::new(
                function_offset,
                format!("{name}() expects no arguments"),
            )),
            ("id", _) => Err(QueryError::new(
                function_offset,
                format!("{name}() expects one string argument"),
            )),
            _ => Err(QueryError::new(
                function_offset,
                format!("unknown function `{name}`"),
            )),
        }
    }

    fn parse_expression_arguments(&mut self) -> Result<Vec<Expression>, QueryError> {
        self.skip_whitespace();
        if self.current_character() == Some(')') {
            return Ok(Vec::new());
        }

        let mut arguments = vec![self.parse_union()?];
        while self.consume(',') {
            arguments.push(self.parse_union()?);
        }
        Ok(arguments)
    }

    fn parse_pattern_union(
        &mut self,
        default_pattern: DefaultPattern,
    ) -> Result<PatternExpression, QueryError> {
        let mut expression = self.parse_pattern_intersection_or_difference(default_pattern)?;
        while self.consume('|') {
            expression = PatternExpression::Union(
                Box::new(expression),
                Box::new(self.parse_pattern_intersection_or_difference(default_pattern)?),
            );
        }
        Ok(expression)
    }

    fn parse_pattern_intersection_or_difference(
        &mut self,
        default_pattern: DefaultPattern,
    ) -> Result<PatternExpression, QueryError> {
        let mut expression = self.parse_pattern_complement(default_pattern)?;
        loop {
            if self.consume('&') {
                expression = PatternExpression::Intersection(
                    Box::new(expression),
                    Box::new(self.parse_pattern_complement(default_pattern)?),
                );
            } else if self.consume('~') {
                expression = PatternExpression::Difference(
                    Box::new(expression),
                    Box::new(self.parse_pattern_complement(default_pattern)?),
                );
            } else {
                return Ok(expression);
            }
        }
    }

    fn parse_pattern_complement(
        &mut self,
        default_pattern: DefaultPattern,
    ) -> Result<PatternExpression, QueryError> {
        if self.consume('~') {
            Ok(PatternExpression::Complement(Box::new(
                self.parse_pattern_complement(default_pattern)?,
            )))
        } else {
            self.parse_pattern_primary(default_pattern)
        }
    }

    fn parse_pattern_primary(
        &mut self,
        default_pattern: DefaultPattern,
    ) -> Result<PatternExpression, QueryError> {
        self.skip_whitespace();
        if self.consume_raw('(') {
            let expression = self.parse_pattern_union(default_pattern)?;
            self.expect(')')?;
            Ok(expression)
        } else {
            let modifier_offset = self.offset;
            let modifier = if self.current_character() == Some('"') {
                match default_pattern {
                    DefaultPattern::Glob => "glob".to_owned(),
                    DefaultPattern::Substring => "substring".to_owned(),
                }
            } else {
                let modifier = self.parse_identifier()?;
                self.expect(':')?;
                modifier
            };
            let argument = self.parse_string_argument()?;
            let pattern = match modifier.as_str() {
                "substring" => Pattern::Substring(argument.value),
                "exact" => Pattern::Exact(argument.value),
                "glob" => Pattern::Glob(argument.value),
                "regex" => Pattern::Regex(regex::Regex::new(&argument.value).map_err(|error| {
                    QueryError::new(argument.offset, format!("invalid regex: {error}"))
                })?),
                _ => {
                    return Err(QueryError::new(
                        modifier_offset,
                        format!("unknown pattern modifier `{modifier}`"),
                    ))
                }
            };
            Ok(PatternExpression::Pattern(pattern))
        }
    }

    fn parse_fields(&mut self, operation: OperationKind) -> Result<Vec<Field>, QueryError> {
        use FieldKind::*;
        self.skip_whitespace();
        if self.current_character() == Some(')') {
            return Ok(Vec::new());
        }
        let mut fields = Vec::new();
        loop {
            let offset = self.offset;
            let name = self.parse_identifier()?;
            let kind = match name.as_str() {
                "path" => Path,
                "before_path" => BeforePath,
                "after_path" => AfterPath,
                "file" => File,
                "content" => Content,
                "from" => From,
                "to" => To,
                _ => return Err(QueryError::new(offset, format!("unknown field `{name}`"))),
            };
            if fields.iter().any(|field: &Field| field.kind == kind) {
                return Err(QueryError::new(offset, format!("repeated field `{name}`")));
            }
            let allowed = match kind {
                Path | BeforePath | AfterPath => true,
                File => matches!(operation, OperationKind::Added | OperationKind::Removed),
                Content => matches!(
                    operation,
                    OperationKind::Changed | OperationKind::Added | OperationKind::Removed
                ),
                From | To => operation == OperationKind::Renamed,
            };
            if !allowed {
                return Err(QueryError::new(
                    offset,
                    format!("field `{name}` is not supported by this operation"),
                ));
            }
            self.expect(':')?;
            let default_pattern = if kind == Content {
                DefaultPattern::Substring
            } else {
                DefaultPattern::Glob
            };
            let pattern = self.parse_pattern_union(default_pattern)?;
            fields.push(Field { kind, pattern });
            if !self.consume(',') {
                return Ok(fields);
            }
        }
    }

    fn parse_arguments(&mut self) -> Result<Vec<StringArgument>, QueryError> {
        self.skip_whitespace();
        if self.current_character() == Some(')') {
            return Ok(Vec::new());
        }

        let mut arguments = vec![self.parse_string_argument()?];
        while self.consume(',') {
            arguments.push(self.parse_string_argument()?);
        }
        Ok(arguments)
    }

    fn parse_string_argument(&mut self) -> Result<StringArgument, QueryError> {
        self.skip_whitespace();
        let offset = self.offset;
        Ok(StringArgument {
            value: self.parse_string()?,
            offset,
        })
    }

    fn parse_identifier(&mut self) -> Result<String, QueryError> {
        self.skip_whitespace();
        let start = self.offset;
        while matches!(self.current_character(), Some(character) if character.is_ascii_alphanumeric() || character == '_')
        {
            self.advance_character();
        }
        if start == self.offset {
            Err(QueryError::new(self.offset, "expected a function or `(`"))
        } else {
            Ok(self.query[start..self.offset].to_owned())
        }
    }

    fn parse_string(&mut self) -> Result<String, QueryError> {
        self.skip_whitespace();
        let start = self.offset;
        if !self.consume_raw('"') {
            return Err(QueryError::new(self.offset, "expected a string argument"));
        }

        let mut value = String::new();
        loop {
            match self.current_character() {
                Some('"') => {
                    self.advance_character();
                    return Ok(value);
                }
                Some('\\') => {
                    self.advance_character();
                    let escape_offset = self.offset;
                    let escaped = self
                        .current_character()
                        .ok_or_else(|| QueryError::new(start, "unterminated string argument"))?;
                    let decoded = match escaped {
                        '"' => '"',
                        '\\' => '\\',
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        _ => {
                            return Err(QueryError::new(
                                escape_offset,
                                format!("unsupported escape `\\{escaped}`"),
                            ))
                        }
                    };
                    value.push(decoded);
                    self.advance_character();
                }
                Some(character) => {
                    value.push(character);
                    self.advance_character();
                }
                None => return Err(QueryError::new(start, "unterminated string argument")),
            }
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), QueryError> {
        if self.consume(expected) {
            Ok(())
        } else {
            Err(QueryError::new(
                self.offset,
                format!("expected `{expected}`"),
            ))
        }
    }

    fn consume(&mut self, expected: char) -> bool {
        self.skip_whitespace();
        self.consume_raw(expected)
    }

    fn consume_raw(&mut self, expected: char) -> bool {
        if self.current_character() == Some(expected) {
            self.advance_character();
            true
        } else {
            false
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.current_character(), Some(character) if character.is_whitespace()) {
            self.advance_character();
        }
    }

    fn current_character(&self) -> Option<char> {
        self.query[self.offset..].chars().next()
    }

    fn advance_character(&mut self) {
        self.offset += self.current_character().unwrap().len_utf8();
    }
}
