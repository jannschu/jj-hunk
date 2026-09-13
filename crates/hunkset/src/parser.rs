use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Expression {
    All,
    None,
    Files(FilesetExpression),
    Content(String),
    Union(Box<Expression>, Box<Expression>),
    Intersection(Box<Expression>, Box<Expression>),
    Difference(Box<Expression>, Box<Expression>),
    Complement(Box<Expression>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FilesetExpression {
    Pattern(String),
    Union(Box<FilesetExpression>, Box<FilesetExpression>),
    Intersection(Box<FilesetExpression>, Box<FilesetExpression>),
    Difference(Box<FilesetExpression>, Box<FilesetExpression>),
    Complement(Box<FilesetExpression>),
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

    fn new(offset: usize, message: impl Into<String>) -> Self {
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
        if name == "files" {
            let fileset = self.parse_fileset_union()?;
            self.expect(')')?;
            return Ok(Expression::Files(fileset));
        }
        let arguments = self.parse_arguments()?;
        self.expect(')')?;

        match (name.as_str(), arguments.as_slice()) {
            ("all", []) => Ok(Expression::All),
            ("none", []) => Ok(Expression::None),
            ("content", [literal]) => Ok(Expression::Content(literal.clone())),
            ("all" | "none", _) => Err(QueryError::new(
                function_offset,
                format!("{name}() expects no arguments"),
            )),
            ("content", _) => Err(QueryError::new(
                function_offset,
                format!("{name}() expects one string argument"),
            )),
            _ => Err(QueryError::new(
                function_offset,
                format!("unknown function `{name}`"),
            )),
        }
    }

    fn parse_fileset_union(&mut self) -> Result<FilesetExpression, QueryError> {
        let mut expression = self.parse_fileset_intersection_or_difference()?;
        while self.consume('|') {
            expression = FilesetExpression::Union(
                Box::new(expression),
                Box::new(self.parse_fileset_intersection_or_difference()?),
            );
        }
        Ok(expression)
    }

    fn parse_fileset_intersection_or_difference(
        &mut self,
    ) -> Result<FilesetExpression, QueryError> {
        let mut expression = self.parse_fileset_complement()?;
        loop {
            if self.consume('&') {
                expression = FilesetExpression::Intersection(
                    Box::new(expression),
                    Box::new(self.parse_fileset_complement()?),
                );
            } else if self.consume('~') {
                expression = FilesetExpression::Difference(
                    Box::new(expression),
                    Box::new(self.parse_fileset_complement()?),
                );
            } else {
                return Ok(expression);
            }
        }
    }

    fn parse_fileset_complement(&mut self) -> Result<FilesetExpression, QueryError> {
        if self.consume('~') {
            Ok(FilesetExpression::Complement(Box::new(
                self.parse_fileset_complement()?,
            )))
        } else {
            self.parse_fileset_primary()
        }
    }

    fn parse_fileset_primary(&mut self) -> Result<FilesetExpression, QueryError> {
        self.skip_whitespace();
        if self.consume_raw('(') {
            let expression = self.parse_fileset_union()?;
            self.expect(')')?;
            Ok(expression)
        } else {
            self.parse_string().map(FilesetExpression::Pattern)
        }
    }

    fn parse_arguments(&mut self) -> Result<Vec<String>, QueryError> {
        self.skip_whitespace();
        if self.current_character() == Some(')') {
            return Ok(Vec::new());
        }

        let mut arguments = vec![self.parse_string()?];
        while self.consume(',') {
            arguments.push(self.parse_string()?);
        }
        Ok(arguments)
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
