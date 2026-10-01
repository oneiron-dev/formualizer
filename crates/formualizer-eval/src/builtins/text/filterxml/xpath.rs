//! XPath 1.0 (W3C Recommendation, 16 November 1999), as MSXML evaluates it
//! for FILTERXML: the full expression language and core function library
//! over a [`Document`], with no variables and no namespace prefixes bound.
//!
//! Parsing and evaluation recurse as deep as the XPath's parentheses and
//! brackets nest ([`Tokens::nesting`], at most [`MAX_NESTING`]): operator
//! chains, runs of minus signs, unions and steps are flat lists, and the
//! document is walked without recursion.

use std::collections::HashSet;
use std::iter::Peekable;
use std::vec::IntoIter;

use super::xml::{Document, Kind, XML_NAMESPACE, is_xml_space};

/// The deepest nesting of parentheses and brackets an XPath may have. Each
/// level takes two characters, so an XPath within FILTERXML's 1024
/// characters never goes deeper; the bound keeps the depth of recursion
/// known whatever the input.
pub(super) const MAX_NESTING: usize = 512;

/// The XPath is invalid, or evaluating it fails (an operand that must be a
/// node-set is not).
#[derive(Debug)]
pub(super) struct Invalid;

type Result<T> = std::result::Result<T, Invalid>;

/// The nodes `xpath` selects in `document` from its root, in document order.
/// `Err` when the XPath is invalid or its value is not a node-set.
pub(super) fn select(document: &Document, xpath: &Compiled) -> Result<Vec<usize>> {
    let evaluator = Evaluator { document };
    let context = Context {
        node: 0,
        position: 1,
        size: 1,
    };
    match evaluator.eval(&xpath.expr, context)? {
        Value::Nodes(nodes) => Ok(nodes),
        _ => Err(Invalid),
    }
}

/// A parsed XPath.
pub(super) struct Compiled {
    expr: Expr,
    /// Whether a step uses the namespace axis (the only way to namespace
    /// nodes).
    pub uses_namespace_axis: bool,
}

/// An XPath split into tokens.
pub(super) struct Tokens(Vec<Token>);

impl Tokens {
    /// How deep parentheses and brackets nest.
    pub fn nesting(&self) -> usize {
        let mut depth = 0usize;
        let mut deepest = 0;
        for token in &self.0 {
            match token {
                Token::LeftParen | Token::LeftBracket => {
                    depth += 1;
                    deepest = deepest.max(depth);
                }
                Token::RightParen | Token::RightBracket => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        deepest
    }
}

/// Parses the tokens of an XPath.
pub(super) fn compile(tokens: Tokens) -> Result<Compiled> {
    if tokens.nesting() > MAX_NESTING {
        return Err(Invalid);
    }
    let mut parser = Parser {
        tokens: tokens.0.into_iter().peekable(),
        uses_namespace_axis: false,
    };
    let expr = parser.expr()?;
    if parser.tokens.next().is_some() {
        return Err(Invalid);
    }
    Ok(Compiled {
        expr,
        uses_namespace_axis: parser.uses_namespace_axis,
    })
}

// ---------------------------------------------------------------------------
// Tokens (§3.7)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

impl Op {
    fn precedence(self) -> u8 {
        match self {
            Op::Or => 1,
            Op::And => 2,
            Op::Eq | Op::Ne => 3,
            Op::Lt | Op::Le | Op::Gt | Op::Ge => 4,
            Op::Add | Op::Sub => 5,
            Op::Mul | Op::Div | Op::Mod => 6,
        }
    }

    /// The operator with its operands swapped (`a < b` is `b > a`).
    fn flipped(self) -> Op {
        match self {
            Op::Lt => Op::Gt,
            Op::Le => Op::Ge,
            Op::Gt => Op::Lt,
            Op::Ge => Op::Le,
            op => op,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Ancestor,
    AncestorOrSelf,
    Attribute,
    Child,
    Descendant,
    DescendantOrSelf,
    Following,
    FollowingSibling,
    Namespace,
    Parent,
    Preceding,
    PrecedingSibling,
    Itself,
}

impl Axis {
    fn from_name(name: &str) -> Option<Axis> {
        Some(match name {
            "ancestor" => Axis::Ancestor,
            "ancestor-or-self" => Axis::AncestorOrSelf,
            "attribute" => Axis::Attribute,
            "child" => Axis::Child,
            "descendant" => Axis::Descendant,
            "descendant-or-self" => Axis::DescendantOrSelf,
            "following" => Axis::Following,
            "following-sibling" => Axis::FollowingSibling,
            "namespace" => Axis::Namespace,
            "parent" => Axis::Parent,
            "preceding" => Axis::Preceding,
            "preceding-sibling" => Axis::PrecedingSibling,
            "self" => Axis::Itself,
            _ => return None,
        })
    }

    /// The kind of node a name test or `*` selects on this axis.
    fn principal_kind(self) -> Kind {
        match self {
            Axis::Attribute => Kind::Attribute,
            Axis::Namespace => Kind::Namespace,
            _ => Kind::Element,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeType {
    Comment,
    Text,
    ProcessingInstruction,
    Node,
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    Dot,
    DotDot,
    At,
    Comma,
    ColonColon,
    Slash,
    DoubleSlash,
    Pipe,
    /// A binary operator; `Sub` is also the unary minus.
    Operator(Op),
    /// The name test `*`.
    Star,
    /// A name test.
    Name(String),
    NodeType(NodeType),
    Function(String),
    Axis(Axis),
    Literal(String),
    Number(f64),
}

/// The XML `NameStartChar` production, without the colon.
fn is_name_start_char(c: char) -> bool {
    matches!(c,
        'A'..='Z' | '_' | 'a'..='z' | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}'
        | '\u{F8}'..='\u{2FF}' | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}'
        | '\u{200C}'..='\u{200D}' | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}'
        | '\u{3001}'..='\u{D7FF}' | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}'
        | '\u{10000}'..='\u{EFFFF}')
}

/// The XML `NameChar` production, without the colon.
fn is_name_char(c: char) -> bool {
    is_name_start_char(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// Splits `xpath` into tokens.
pub(super) fn tokenize(xpath: &str) -> Result<Tokens> {
    let chars: Vec<char> = xpath.chars().collect();
    let at = |index: usize| chars.get(index).copied();
    let mut tokens = Vec::new();
    let mut index = 0;
    loop {
        while at(index).is_some_and(is_xml_space) {
            index += 1;
        }
        let Some(c) = at(index) else {
            return Ok(Tokens(tokens));
        };
        let next = at(index + 1);
        // §3.7: after a token that ends an operand, `*` is the multiply
        // operator and a name is an operator name.
        let after_operand = tokens.last().is_some_and(|token| {
            !matches!(
                token,
                Token::At
                    | Token::ColonColon
                    | Token::LeftParen
                    | Token::LeftBracket
                    | Token::Comma
                    | Token::Operator(_)
                    | Token::Slash
                    | Token::DoubleSlash
                    | Token::Pipe
            )
        });
        let (token, len) = match c {
            '(' => (Token::LeftParen, 1),
            ')' => (Token::RightParen, 1),
            '[' => (Token::LeftBracket, 1),
            ']' => (Token::RightBracket, 1),
            ',' => (Token::Comma, 1),
            '@' => (Token::At, 1),
            '|' => (Token::Pipe, 1),
            '+' => (Token::Operator(Op::Add), 1),
            '-' => (Token::Operator(Op::Sub), 1),
            '=' => (Token::Operator(Op::Eq), 1),
            '!' if next == Some('=') => (Token::Operator(Op::Ne), 2),
            '<' if next == Some('=') => (Token::Operator(Op::Le), 2),
            '<' => (Token::Operator(Op::Lt), 1),
            '>' if next == Some('=') => (Token::Operator(Op::Ge), 2),
            '>' => (Token::Operator(Op::Gt), 1),
            '/' if next == Some('/') => (Token::DoubleSlash, 2),
            '/' => (Token::Slash, 1),
            ':' if next == Some(':') => (Token::ColonColon, 2),
            '.' if next == Some('.') => (Token::DotDot, 2),
            '.' if !next.is_some_and(|d| d.is_ascii_digit()) => (Token::Dot, 1),
            '.' | '0'..='9' => {
                // Digits ('.' Digits?)? | '.' Digits
                let mut end = index;
                while at(end).is_some_and(|d| d.is_ascii_digit()) {
                    end += 1;
                }
                if at(end) == Some('.') {
                    end += 1;
                    while at(end).is_some_and(|d| d.is_ascii_digit()) {
                        end += 1;
                    }
                }
                let number: String = chars[index..end].iter().collect();
                (
                    Token::Number(number.parse().map_err(|_| Invalid)?),
                    end - index,
                )
            }
            '"' | '\'' => {
                let close = chars[index + 1..]
                    .iter()
                    .position(|&q| q == c)
                    .ok_or(Invalid)?;
                let literal = chars[index + 1..index + 1 + close].iter().collect();
                (Token::Literal(literal), close + 2)
            }
            '*' if after_operand => (Token::Operator(Op::Mul), 1),
            '*' => (Token::Star, 1),
            c if is_name_start_char(c) => {
                let end = (index + 1..chars.len())
                    .find(|&i| !is_name_char(chars[i]))
                    .unwrap_or(chars.len());
                let name: String = chars[index..end].iter().collect();
                let mut after = end;
                while at(after).is_some_and(is_xml_space) {
                    after += 1;
                }
                let token = if after_operand {
                    Token::Operator(match name.as_str() {
                        "and" => Op::And,
                        "or" => Op::Or,
                        "div" => Op::Div,
                        "mod" => Op::Mod,
                        _ => return Err(Invalid),
                    })
                } else if at(end) == Some(':') && at(end + 1) != Some(':') {
                    // A prefixed name (`p:a`, `p:*`, `p:f()`): FILTERXML binds
                    // no namespace prefix for the XPath.
                    return Err(Invalid);
                } else if at(after) == Some('(') {
                    match name.as_str() {
                        "comment" => Token::NodeType(NodeType::Comment),
                        "text" => Token::NodeType(NodeType::Text),
                        "processing-instruction" => {
                            Token::NodeType(NodeType::ProcessingInstruction)
                        }
                        "node" => Token::NodeType(NodeType::Node),
                        _ => Token::Function(name),
                    }
                } else if at(after) == Some(':') && at(after + 1) == Some(':') {
                    Token::Axis(Axis::from_name(&name).ok_or(Invalid)?)
                } else {
                    Token::Name(name)
                };
                (token, end - index)
            }
            // Includes `$`: no variable is bound.
            _ => return Err(Invalid),
        };
        tokens.push(token);
        index += len;
    }
}

// ---------------------------------------------------------------------------
// Syntax (§2, §3)
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum Expr {
    Number(f64),
    Literal(String),
    /// Operands joined left to right by operators of one precedence.
    Binary(Box<Expr>, Vec<(Op, Expr)>),
    /// The unary minus: the operand as a number, negated when `true` (an odd
    /// number of minus signs).
    Negate(bool, Box<Expr>),
    Union(Vec<Expr>),
    Path(Start, Vec<Step>),
    /// A primary expression and its predicates.
    Filter(Box<Expr>, Vec<Expr>),
    Call(Function, Vec<Expr>),
}

#[derive(Debug)]
enum Start {
    Root,
    Context,
    Filter(Box<Expr>),
}

#[derive(Debug)]
struct Step {
    axis: Axis,
    test: NodeTest,
    predicates: Vec<Expr>,
}

impl Step {
    /// `//` is `/descendant-or-self::node()/`.
    fn descendant_or_self() -> Step {
        Step {
            axis: Axis::DescendantOrSelf,
            test: NodeTest::Node,
            predicates: Vec::new(),
        }
    }
}

#[derive(Debug)]
enum NodeTest {
    /// A name test without a prefix: a node of the principal kind with that
    /// name and no namespace.
    Name(String),
    /// `*`: any node of the principal kind.
    Any,
    Node,
    Text,
    Comment,
    ProcessingInstruction(Option<String>),
}

/// The core function library (§4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Function {
    Last,
    Position,
    Count,
    Id,
    LocalName,
    NamespaceUri,
    Name,
    String,
    Concat,
    StartsWith,
    Contains,
    SubstringBefore,
    SubstringAfter,
    Substring,
    StringLength,
    NormalizeSpace,
    Translate,
    Boolean,
    Not,
    True,
    False,
    Lang,
    Number,
    Sum,
    Floor,
    Ceiling,
    Round,
}

impl Function {
    /// The function and the least and most arguments it takes.
    fn from_name(name: &str) -> Option<(Function, usize, usize)> {
        use Function as F;
        Some(match name {
            "last" => (F::Last, 0, 0),
            "position" => (F::Position, 0, 0),
            "count" => (F::Count, 1, 1),
            "id" => (F::Id, 1, 1),
            "local-name" => (F::LocalName, 0, 1),
            "namespace-uri" => (F::NamespaceUri, 0, 1),
            "name" => (F::Name, 0, 1),
            "string" => (F::String, 0, 1),
            "concat" => (F::Concat, 2, usize::MAX),
            "starts-with" => (F::StartsWith, 2, 2),
            "contains" => (F::Contains, 2, 2),
            "substring-before" => (F::SubstringBefore, 2, 2),
            "substring-after" => (F::SubstringAfter, 2, 2),
            "substring" => (F::Substring, 2, 3),
            "string-length" => (F::StringLength, 0, 1),
            "normalize-space" => (F::NormalizeSpace, 0, 1),
            "translate" => (F::Translate, 3, 3),
            "boolean" => (F::Boolean, 1, 1),
            "not" => (F::Not, 1, 1),
            "true" => (F::True, 0, 0),
            "false" => (F::False, 0, 0),
            "lang" => (F::Lang, 1, 1),
            "number" => (F::Number, 0, 1),
            "sum" => (F::Sum, 1, 1),
            "floor" => (F::Floor, 1, 1),
            "ceiling" => (F::Ceiling, 1, 1),
            "round" => (F::Round, 1, 1),
            _ => return None,
        })
    }
}

struct Parser {
    tokens: Peekable<IntoIter<Token>>,
    uses_namespace_axis: bool,
}

impl Parser {
    fn eat(&mut self, token: &Token) -> bool {
        self.tokens.next_if_eq(token).is_some()
    }

    fn expect(&mut self, token: &Token) -> Result<()> {
        if self.eat(token) {
            Ok(())
        } else {
            Err(Invalid)
        }
    }

    /// Expr: unary expressions joined by binary operators, by precedence and
    /// left to right.
    fn expr(&mut self) -> Result<Expr> {
        let mut operands = vec![self.unary()?];
        let mut operators: Vec<Op> = Vec::new();
        while let Some(&Token::Operator(op)) = self.tokens.peek() {
            self.tokens.next();
            while let Some(&top) = operators.last()
                && top.precedence() >= op.precedence()
            {
                operators.pop();
                reduce(&mut operands, top);
            }
            operators.push(op);
            operands.push(self.unary()?);
        }
        while let Some(op) = operators.pop() {
            reduce(&mut operands, op);
        }
        operands.pop().ok_or(Invalid)
    }

    /// UnaryExpr: minus signs, then a union.
    fn unary(&mut self) -> Result<Expr> {
        let mut minus_signs = 0usize;
        while self.eat(&Token::Operator(Op::Sub)) {
            minus_signs += 1;
        }
        let union = self.union()?;
        Ok(if minus_signs == 0 {
            union
        } else {
            Expr::Negate(minus_signs % 2 == 1, Box::new(union))
        })
    }

    /// UnionExpr: paths joined by `|`.
    fn union(&mut self) -> Result<Expr> {
        let first = self.path()?;
        if self.tokens.peek() != Some(&Token::Pipe) {
            return Ok(first);
        }
        let mut paths = vec![first];
        while self.eat(&Token::Pipe) {
            paths.push(self.path()?);
        }
        Ok(Expr::Union(paths))
    }

    /// PathExpr: a location path, or a filter expression optionally followed
    /// by `/` or `//` and a relative location path.
    fn path(&mut self) -> Result<Expr> {
        let mut steps = Vec::new();
        let start = match self.tokens.peek() {
            Some(Token::Slash) => {
                self.tokens.next();
                if self.tokens.peek().is_some_and(starts_step) {
                    self.relative_path(&mut steps)?;
                }
                Start::Root
            }
            Some(Token::DoubleSlash) => {
                self.tokens.next();
                steps.push(Step::descendant_or_self());
                self.relative_path(&mut steps)?;
                Start::Root
            }
            Some(token) if starts_step(token) => {
                self.relative_path(&mut steps)?;
                Start::Context
            }
            _ => {
                let filter = self.filter()?;
                if self.eat(&Token::DoubleSlash) {
                    steps.push(Step::descendant_or_self());
                } else if !self.eat(&Token::Slash) {
                    return Ok(filter);
                }
                self.relative_path(&mut steps)?;
                Start::Filter(Box::new(filter))
            }
        };
        Ok(Expr::Path(start, steps))
    }

    /// RelativeLocationPath: steps separated by `/` or `//`.
    fn relative_path(&mut self, steps: &mut Vec<Step>) -> Result<()> {
        loop {
            steps.push(self.step()?);
            if self.eat(&Token::DoubleSlash) {
                steps.push(Step::descendant_or_self());
            } else if !self.eat(&Token::Slash) {
                return Ok(());
            }
        }
    }

    /// Step: an axis, a node test and predicates; `.` and `..` (which take no
    /// predicates).
    fn step(&mut self) -> Result<Step> {
        let axis = match self.tokens.peek() {
            Some(Token::Dot | Token::DotDot) => {
                let axis = match self.tokens.next() {
                    Some(Token::Dot) => Axis::Itself,
                    _ => Axis::Parent,
                };
                return Ok(Step {
                    axis,
                    test: NodeTest::Node,
                    predicates: Vec::new(),
                });
            }
            Some(Token::At) => {
                self.tokens.next();
                Axis::Attribute
            }
            Some(&Token::Axis(axis)) => {
                self.tokens.next();
                self.expect(&Token::ColonColon)?;
                axis
            }
            _ => Axis::Child,
        };
        self.uses_namespace_axis |= axis == Axis::Namespace;
        let test = match self.tokens.next() {
            Some(Token::Name(name)) => NodeTest::Name(name),
            Some(Token::Star) => NodeTest::Any,
            Some(Token::NodeType(node_type)) => {
                self.expect(&Token::LeftParen)?;
                let test = match node_type {
                    NodeType::Comment => NodeTest::Comment,
                    NodeType::Text => NodeTest::Text,
                    NodeType::Node => NodeTest::Node,
                    NodeType::ProcessingInstruction => {
                        NodeTest::ProcessingInstruction(match self.tokens.peek() {
                            Some(Token::Literal(_)) => match self.tokens.next() {
                                Some(Token::Literal(target)) => Some(target),
                                _ => None,
                            },
                            _ => None,
                        })
                    }
                };
                self.expect(&Token::RightParen)?;
                test
            }
            _ => return Err(Invalid),
        };
        Ok(Step {
            axis,
            test,
            predicates: self.predicates()?,
        })
    }

    fn predicates(&mut self) -> Result<Vec<Expr>> {
        let mut predicates = Vec::new();
        while self.eat(&Token::LeftBracket) {
            predicates.push(self.expr()?);
            self.expect(&Token::RightBracket)?;
        }
        Ok(predicates)
    }

    /// FilterExpr: a primary expression (a parenthesized expression, a
    /// literal, a number or a function call) and its predicates.
    fn filter(&mut self) -> Result<Expr> {
        let primary = match self.tokens.next() {
            Some(Token::LeftParen) => {
                let expr = self.expr()?;
                self.expect(&Token::RightParen)?;
                expr
            }
            Some(Token::Literal(literal)) => Expr::Literal(literal),
            Some(Token::Number(number)) => Expr::Number(number),
            Some(Token::Function(name)) => {
                let (function, min, max) = Function::from_name(&name).ok_or(Invalid)?;
                self.expect(&Token::LeftParen)?;
                let mut args = Vec::new();
                if !self.eat(&Token::RightParen) {
                    loop {
                        args.push(self.expr()?);
                        if self.eat(&Token::RightParen) {
                            break;
                        }
                        self.expect(&Token::Comma)?;
                    }
                }
                if args.len() < min || args.len() > max {
                    return Err(Invalid);
                }
                Expr::Call(function, args)
            }
            _ => return Err(Invalid),
        };
        let predicates = self.predicates()?;
        Ok(if predicates.is_empty() {
            primary
        } else {
            Expr::Filter(Box::new(primary), predicates)
        })
    }
}

/// Whether `token` begins a location step.
fn starts_step(token: &Token) -> bool {
    matches!(
        token,
        Token::Name(_)
            | Token::Star
            | Token::NodeType(_)
            | Token::Axis(_)
            | Token::At
            | Token::Dot
            | Token::DotDot
    )
}

/// Joins the last two operands with `op`. A left operand that is already a
/// chain of `op`'s precedence takes the right one as its next link, so a
/// long chain (`1+1+...`) is one flat node.
fn reduce(operands: &mut Vec<Expr>, op: Op) {
    let (Some(right), Some(left)) = (operands.pop(), operands.pop()) else {
        unreachable!("each operator follows an operand and precedes one");
    };
    operands.push(match left {
        Expr::Binary(first, mut rest) if rest[0].0.precedence() == op.precedence() => {
            rest.push((op, right));
            Expr::Binary(first, rest)
        }
        left => Expr::Binary(Box::new(left), vec![(op, right)]),
    });
}

// ---------------------------------------------------------------------------
// Evaluation (§2-§4)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Value {
    Boolean(bool),
    Number(f64),
    String(String),
    /// Distinct nodes in document order.
    Nodes(Vec<usize>),
}

impl Value {
    /// The `boolean()` of the value (§4.3).
    fn boolean(&self) -> bool {
        match self {
            Value::Boolean(b) => *b,
            Value::Number(n) => *n != 0.0 && !n.is_nan(),
            Value::String(s) => !s.is_empty(),
            Value::Nodes(nodes) => !nodes.is_empty(),
        }
    }
}

/// The `number()` of a string (§4.4): optional white space, an optional
/// minus sign and digits with an optional decimal point (`Number`); any other
/// string (an exponent, a plus sign, `Infinity`) is NaN.
fn string_to_number(s: &str) -> f64 {
    let s = s.trim_matches(is_xml_space);
    let unsigned = s.strip_prefix('-').unwrap_or(s);
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    let is_number = digits(whole)
        && fraction.is_none_or(digits)
        && (!whole.is_empty() || fraction.is_some_and(|f| !f.is_empty()));
    if is_number {
        s.parse().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

/// The `string()` of a number (§4.2): no exponent, as many digits as the
/// number needs; `NaN`, `Infinity`, `-Infinity`, and `0` for either zero.
fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else if n == 0.0 {
        "0".into()
    } else {
        n.to_string()
    }
}

/// `round()` (§4.4): the closest integer, the one toward positive infinity
/// on a tie; a negative argument that rounds to zero gives negative zero.
fn round(n: f64) -> f64 {
    if !n.is_finite() {
        return n;
    }
    let floor = n.floor();
    let rounded = if n - floor >= 0.5 { floor + 1.0 } else { floor };
    if rounded == 0.0 && n.is_sign_negative() {
        -0.0
    } else {
        rounded
    }
}

fn compare_numbers(op: Op, left: f64, right: f64) -> bool {
    match op {
        Op::Eq => left == right,
        Op::Ne => left != right,
        Op::Lt => left < right,
        Op::Le => left <= right,
        Op::Gt => left > right,
        Op::Ge => left >= right,
        _ => unreachable!("not a comparison"),
    }
}

/// `substring()` (§4.2): the characters at positions from the rounded start
/// to before the rounded start plus the rounded length, by IEEE 754
/// comparisons (so NaN selects nothing).
fn substring(s: &str, start: f64, length: Option<f64>) -> String {
    let first = round(start);
    let end = length.map(|length| first + round(length));
    s.chars()
        .zip(1..)
        .filter(|&(_, position)| {
            let position = f64::from(position);
            position >= first && end.is_none_or(|end| position < end)
        })
        .map(|(c, _)| c)
        .collect()
}

/// `translate()` (§4.2).
fn translate(s: &str, from: &str, to: &str) -> String {
    let to: Vec<char> = to.chars().collect();
    s.chars()
        .filter_map(|c| match from.chars().position(|f| f == c) {
            Some(index) => to.get(index).copied(),
            None => Some(c),
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct Context {
    node: usize,
    /// The 1-based position of the node in the node list being filtered.
    position: usize,
    size: usize,
}

struct Evaluator<'a> {
    document: &'a Document,
}

impl Evaluator<'_> {
    fn eval(&self, expr: &Expr, context: Context) -> Result<Value> {
        match expr {
            Expr::Number(n) => Ok(Value::Number(*n)),
            Expr::Literal(s) => Ok(Value::String(s.clone())),
            Expr::Binary(first, rest) => self.binary(first, rest, context),
            Expr::Negate(negate, operand) => {
                let n = self.number(&self.eval(operand, context)?);
                Ok(Value::Number(if *negate { -n } else { n }))
            }
            Expr::Union(paths) => self.union(paths, context).map(Value::Nodes),
            Expr::Path(start, steps) => self.path(start, steps, context).map(Value::Nodes),
            Expr::Filter(primary, predicates) => {
                self.filter(primary, predicates, context).map(Value::Nodes)
            }
            Expr::Call(function, args) => self.call(*function, args, context),
        }
    }

    fn nodes(&self, expr: &Expr, context: Context) -> Result<Vec<usize>> {
        match self.eval(expr, context)? {
            Value::Nodes(nodes) => Ok(nodes),
            _ => Err(Invalid),
        }
    }

    fn string_value(&self, node: usize) -> String {
        self.document.string_value(node)
    }

    /// The `string()` of a value (§4.2): a node-set gives the string-value of
    /// its first node.
    fn string(&self, value: &Value) -> String {
        match value {
            Value::Boolean(b) => if *b { "true" } else { "false" }.into(),
            Value::Number(n) => number_to_string(*n),
            Value::String(s) => s.clone(),
            Value::Nodes(nodes) => nodes
                .first()
                .map(|&node| self.string_value(node))
                .unwrap_or_default(),
        }
    }

    /// The `number()` of a value (§4.4).
    fn number(&self, value: &Value) -> f64 {
        match value {
            Value::Boolean(b) => f64::from(u8::from(*b)),
            Value::Number(n) => *n,
            Value::String(s) => string_to_number(s),
            Value::Nodes(_) => string_to_number(&self.string(value)),
        }
    }

    fn binary(&self, first: &Expr, rest: &[(Op, Expr)], context: Context) -> Result<Value> {
        let mut value = self.eval(first, context)?;
        for (op, operand) in rest {
            value = match op {
                // The right operand is not evaluated when the left decides.
                Op::Or => Value::Boolean(value.boolean() || self.eval(operand, context)?.boolean()),
                Op::And => {
                    Value::Boolean(value.boolean() && self.eval(operand, context)?.boolean())
                }
                Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                    Value::Boolean(self.compare(*op, &value, &self.eval(operand, context)?))
                }
                Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod => {
                    let left = self.number(&value);
                    let right = self.number(&self.eval(operand, context)?);
                    Value::Number(match op {
                        Op::Add => left + right,
                        Op::Sub => left - right,
                        Op::Mul => left * right,
                        Op::Div => left / right,
                        // Truncating, like Java's `%` (5 mod -2 is 1).
                        _ => left % right,
                    })
                }
            };
        }
        Ok(value)
    }

    /// The comparisons of §3.4. A node-set compares true when some node's
    /// string-value (or its number, for a number and for `<`, `<=`, `>`,
    /// `>=`) does.
    fn compare(&self, op: Op, left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Nodes(left), Value::Nodes(right)) => {
                if matches!(op, Op::Eq | Op::Ne) {
                    let strings = |nodes: &[usize]| -> HashSet<String> {
                        nodes.iter().map(|&node| self.string_value(node)).collect()
                    };
                    let (left, right) = (strings(left), strings(right));
                    if op == Op::Eq {
                        !left.is_disjoint(&right)
                    } else {
                        // Some pair differs unless both hold one same string.
                        !(left.is_empty() || right.is_empty() || (left.len() == 1 && left == right))
                    }
                } else {
                    let numbers = |nodes: &[usize]| -> Vec<f64> {
                        nodes
                            .iter()
                            .map(|&node| string_to_number(&self.string_value(node)))
                            .filter(|n| !n.is_nan())
                            .collect()
                    };
                    let (left, right) = (numbers(left), numbers(right));
                    let min = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
                    let max = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    !left.is_empty()
                        && !right.is_empty()
                        && match op {
                            Op::Lt | Op::Le => compare_numbers(op, min(&left), max(&right)),
                            _ => compare_numbers(op, max(&left), min(&right)),
                        }
                }
            }
            (Value::Nodes(nodes), other) => self.compare_nodes(op, nodes, other),
            (other, Value::Nodes(nodes)) => self.compare_nodes(op.flipped(), nodes, other),
            _ => self.compare_values(op, left, right),
        }
    }

    /// Whether `node op other` holds for some node of `nodes`; `other` is not
    /// a node-set.
    fn compare_nodes(&self, op: Op, nodes: &[usize], other: &Value) -> bool {
        match other {
            Value::Boolean(_) => self.compare_values(op, &Value::Boolean(!nodes.is_empty()), other),
            Value::String(s) if matches!(op, Op::Eq | Op::Ne) => nodes
                .iter()
                .any(|&node| (self.string_value(node) == *s) == (op == Op::Eq)),
            _ => {
                let other = self.number(other);
                nodes.iter().any(|&node| {
                    compare_numbers(op, string_to_number(&self.string_value(node)), other)
                })
            }
        }
    }

    /// Compares two values that are not node-sets: `=` and `!=` as booleans
    /// when either is one, else as numbers when either is one, else as
    /// strings; the others as numbers.
    fn compare_values(&self, op: Op, left: &Value, right: &Value) -> bool {
        if !matches!(op, Op::Eq | Op::Ne) {
            return compare_numbers(op, self.number(left), self.number(right));
        }
        let either = |f: fn(&Value) -> bool| f(left) || f(right);
        if either(|v| matches!(v, Value::Boolean(_))) {
            (left.boolean() == right.boolean()) == (op == Op::Eq)
        } else if either(|v| matches!(v, Value::Number(_))) {
            compare_numbers(op, self.number(left), self.number(right))
        } else {
            (self.string(left) == self.string(right)) == (op == Op::Eq)
        }
    }

    fn union(&self, paths: &[Expr], context: Context) -> Result<Vec<usize>> {
        let mut nodes = Vec::new();
        for path in paths {
            nodes.extend(self.nodes(path, context)?);
        }
        nodes.sort_unstable();
        nodes.dedup();
        Ok(nodes)
    }

    fn path(&self, start: &Start, steps: &[Step], context: Context) -> Result<Vec<usize>> {
        let mut nodes = match start {
            Start::Root => vec![0],
            Start::Context => vec![context.node],
            Start::Filter(filter) => self.nodes(filter, context)?,
        };
        for step in steps {
            nodes = self.step(step, &nodes)?;
        }
        Ok(nodes)
    }

    /// The nodes `step` selects from each of `nodes`, in document order.
    fn step(&self, step: &Step, nodes: &[usize]) -> Result<Vec<usize>> {
        let mut selected = Vec::new();
        for &node in nodes {
            // Predicates count positions in the axis's order.
            let mut candidates: Vec<usize> = self
                .axis(step.axis, node)
                .into_iter()
                .filter(|&candidate| self.test(step, candidate))
                .collect();
            for predicate in &step.predicates {
                candidates = self.predicate(predicate, candidates)?;
            }
            selected.extend(candidates);
        }
        selected.sort_unstable();
        selected.dedup();
        Ok(selected)
    }

    /// The nodes on `axis` from `node`, in the axis's order: document order,
    /// or reverse document order on the reverse axes (ancestor,
    /// ancestor-or-self, preceding, preceding-sibling).
    fn axis(&self, axis: Axis, node: usize) -> Vec<usize> {
        let document = self.document;
        let this = &document.nodes[node];
        let ancestors =
            || std::iter::successors(this.parent, |&ancestor| document.nodes[ancestor].parent);
        match axis {
            Axis::Itself => vec![node],
            Axis::Child => this.children.clone(),
            Axis::Descendant => document.descendants(node).collect(),
            Axis::DescendantOrSelf => std::iter::once(node)
                .chain(document.descendants(node))
                .collect(),
            Axis::Parent => this.parent.into_iter().collect(),
            Axis::Ancestor => ancestors().collect(),
            Axis::AncestorOrSelf => std::iter::once(node).chain(ancestors()).collect(),
            Axis::FollowingSibling => document
                .siblings(node)
                .get(this.sibling_index + 1..)
                .unwrap_or_default()
                .to_vec(),
            Axis::PrecedingSibling => document
                .siblings(node)
                .get(..this.sibling_index)
                .unwrap_or_default()
                .iter()
                .rev()
                .copied()
                .collect(),
            // After the node's subtree (an attribute's element's children
            // come after the attribute).
            Axis::Following => (this.end..document.nodes.len())
                .filter(|&other| document.nodes[other].is_tree_node())
                .collect(),
            // Before the node, ancestors excluded: an ancestor's subtree
            // reaches past the node.
            Axis::Preceding => (0..node)
                .rev()
                .filter(|&other| {
                    let other = &document.nodes[other];
                    other.end <= node && other.is_tree_node()
                })
                .collect(),
            Axis::Attribute => this.attributes.clone().collect(),
            Axis::Namespace => this.namespaces.clone().collect(),
        }
    }

    fn test(&self, step: &Step, node: usize) -> bool {
        let node = &self.document.nodes[node];
        match &step.test {
            NodeTest::Name(name) => {
                node.kind == step.axis.principal_kind()
                    && node.namespace_uri.is_none()
                    && node.local_name == *name
            }
            NodeTest::Any => node.kind == step.axis.principal_kind(),
            NodeTest::Node => true,
            NodeTest::Text => node.kind == Kind::Text,
            NodeTest::Comment => node.kind == Kind::Comment,
            NodeTest::ProcessingInstruction(target) => {
                node.kind == Kind::ProcessingInstruction
                    && target
                        .as_ref()
                        .is_none_or(|target| *target == node.local_name)
            }
        }
    }

    /// The nodes of `nodes` (in the order positions count) the predicate
    /// keeps. A number keeps the node whose position equals it (§2.4), so a
    /// fraction keeps none; any other value keeps the node when true.
    fn predicate(&self, predicate: &Expr, nodes: Vec<usize>) -> Result<Vec<usize>> {
        let size = nodes.len();
        let mut kept = Vec::new();
        for (index, &node) in nodes.iter().enumerate() {
            let position = index + 1;
            let context = Context {
                node,
                position,
                size,
            };
            let keep = match self.eval(predicate, context)? {
                Value::Number(n) => n == position as f64,
                value => value.boolean(),
            };
            if keep {
                kept.push(node);
            }
        }
        Ok(kept)
    }

    fn filter(&self, primary: &Expr, predicates: &[Expr], context: Context) -> Result<Vec<usize>> {
        let mut nodes = self.nodes(primary, context)?;
        for predicate in predicates {
            nodes = self.predicate(predicate, nodes)?;
        }
        Ok(nodes)
    }

    /// Whether the language of `node` (from the `xml:lang` of it or its
    /// nearest ancestor that has one) is `language` or a sublanguage of it,
    /// ignoring case (§4.3 `lang()`).
    fn lang(&self, node: usize, language: &str) -> bool {
        let nodes = &self.document.nodes;
        let declared =
            std::iter::successors(Some(node), |&node| nodes[node].parent).find_map(|node| {
                nodes[node]
                    .attributes
                    .clone()
                    .map(|a| &nodes[a])
                    .find(|attribute| {
                        attribute.local_name == "lang"
                            && attribute.namespace_uri.as_deref() == Some(XML_NAMESPACE)
                    })
            });
        declared.is_some_and(|attribute| {
            let value = attribute.value.to_lowercase();
            value
                .strip_prefix(&language.to_lowercase())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
        })
    }

    fn call(&self, function: Function, args: &[Expr], context: Context) -> Result<Value> {
        use Function as F;
        let mut values = Vec::with_capacity(args.len());
        for arg in args {
            values.push(self.eval(arg, context)?);
        }
        let string = |index: usize| self.string(&values[index]);
        // The string of the argument, or the string-value of the context node.
        let string_or_context = || match values.first() {
            Some(value) => self.string(value),
            None => self.string_value(context.node),
        };
        let number = |index: usize| self.number(&values[index]);
        Ok(match function {
            F::Last => Value::Number(context.size as f64),
            F::Position => Value::Number(context.position as f64),
            F::Count => match &values[0] {
                Value::Nodes(nodes) => Value::Number(nodes.len() as f64),
                _ => return Err(Invalid),
            },
            // Without a DTD no attribute is of type ID, so no node has an ID.
            F::Id => Value::Nodes(Vec::new()),
            F::LocalName | F::NamespaceUri | F::Name => {
                let node = match values.first() {
                    None => Some(context.node),
                    Some(Value::Nodes(nodes)) => nodes.first().copied(),
                    Some(_) => return Err(Invalid),
                };
                let name = node.map_or_else(String::new, |node| {
                    let node = &self.document.nodes[node];
                    match function {
                        F::LocalName => node.local_name.clone(),
                        F::NamespaceUri => node.namespace_uri.clone().unwrap_or_default(),
                        _ => match &node.prefix {
                            Some(prefix) => format!("{prefix}:{}", node.local_name),
                            None => node.local_name.clone(),
                        },
                    }
                });
                Value::String(name)
            }
            F::String => Value::String(string_or_context()),
            F::Concat => Value::String(values.iter().map(|value| self.string(value)).collect()),
            F::StartsWith => Value::Boolean(string(0).starts_with(&string(1))),
            F::Contains => Value::Boolean(string(0).contains(&string(1))),
            F::SubstringBefore => {
                let (s, part) = (string(0), string(1));
                Value::String(s.find(&part).map_or_else(String::new, |at| s[..at].into()))
            }
            F::SubstringAfter => {
                let (s, part) = (string(0), string(1));
                Value::String(
                    s.find(&part)
                        .map_or_else(String::new, |at| s[at + part.len()..].into()),
                )
            }
            F::Substring => Value::String(substring(
                &string(0),
                number(1),
                (values.len() > 2).then(|| number(2)),
            )),
            F::StringLength => Value::Number(string_or_context().chars().count() as f64),
            F::NormalizeSpace => Value::String(
                string_or_context()
                    .split(is_xml_space)
                    .filter(|word| !word.is_empty())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            F::Translate => Value::String(translate(&string(0), &string(1), &string(2))),
            F::Boolean => Value::Boolean(values[0].boolean()),
            F::Not => Value::Boolean(!values[0].boolean()),
            F::True => Value::Boolean(true),
            F::False => Value::Boolean(false),
            F::Lang => Value::Boolean(self.lang(context.node, &string(0))),
            F::Number => Value::Number(match values.first() {
                Some(value) => self.number(value),
                None => string_to_number(&self.string_value(context.node)),
            }),
            F::Sum => match &values[0] {
                // From positive zero: the sum of no node is 0.
                Value::Nodes(nodes) => Value::Number(nodes.iter().fold(0.0, |sum, &node| {
                    sum + string_to_number(&self.string_value(node))
                })),
                _ => return Err(Invalid),
            },
            F::Floor => Value::Number(number(0).floor()),
            F::Ceiling => Value::Number(number(0).ceil()),
            F::Round => Value::Number(round(number(0))),
        })
    }
}
