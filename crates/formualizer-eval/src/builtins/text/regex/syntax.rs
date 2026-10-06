//! The PCRE2 pattern syntax Excel's REGEX functions take, parsed to a tree.
//!
//! Excel for Windows compiles patterns with PCRE2 in UTF and UCP mode, with
//! CR, LF and CRLF as newlines (`.` matches neither `\r` nor `\n`). Every
//! construct parsed here is matched exactly; constructs this module does not
//! match (recursion and subroutine calls, `\X`, most backtracking verbs,
//! variable-length lookbehind, duplicate group names) are `Unsupported`, so the
//! formula is left to a fallback engine rather than given a value Excel might
//! not give. Syntax PCRE2 itself rejects is `Invalid` (Excel's `#VALUE!`).

use regex_syntax::hir::{ClassUnicode, ClassUnicodeRange};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PatternError {
    /// PCRE2 rejects the pattern: Excel returns `#VALUE!`.
    Invalid(String),
    /// Valid PCRE2 that this engine does not match.
    Unsupported(String),
}

fn invalid<T>(why: &str) -> Result<T, PatternError> {
    Err(PatternError::Invalid(why.to_string()))
}

fn unsupported<T>(why: &str) -> Result<T, PatternError> {
    Err(PatternError::Unsupported(why.to_string()))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Greed {
    Greedy,
    Lazy,
    Possessive,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Assertion {
    /// `^`: the start, or after an internal newline in multiline mode.
    LineStart {
        multiline: bool,
    },
    /// `$`: the end or before a final newline, or before any newline in
    /// multiline mode.
    LineEnd {
        multiline: bool,
    },
    /// `\A`
    TextStart,
    /// `\Z`: the end or before a final newline.
    TextEndNewline,
    /// `\z`
    TextEnd,
    /// `\G`: where this match attempt's search began.
    SearchStart,
    WordBoundary,
    NotWordBoundary,
}

#[derive(Debug, Clone)]
pub(crate) enum Condition {
    /// `(?(1)...)`, `(?(<name>)...)`: whether the group has matched.
    Group(usize),
    /// `(?(?=...)...)` and the other assertions.
    Look(Box<Node>),
}

#[derive(Debug, Clone)]
pub(crate) enum Node {
    Empty,
    /// A set of characters (a literal under caseless matching, a class,
    /// `\d`, `.` in dotall mode, ...).
    Set(ClassUnicode),
    Char(char),
    /// `.` outside dotall mode: anything but `\r` and `\n`.
    AnyButNewline,
    Concat(Vec<Node>),
    Alternate(Vec<Node>),
    Capture {
        index: usize,
        node: Box<Node>,
    },
    Repeat {
        node: Box<Node>,
        min: u32,
        max: Option<u32>,
        greed: Greed,
    },
    Atomic(Box<Node>),
    Look {
        behind: bool,
        negate: bool,
        node: Box<Node>,
    },
    Backref {
        group: usize,
        caseless: bool,
    },
    Assert(Assertion),
    /// `\K`: the reported match starts here.
    KeepOut,
    Conditional {
        condition: Condition,
        yes: Box<Node>,
        no: Box<Node>,
    },
    /// `(*FAIL)`, `(*F)`.
    Fail,
}

#[derive(Debug, Clone, Copy)]
struct Flags {
    caseless: bool,
    multiline: bool,
    dotall: bool,
    extended: bool,
    extended_more: bool,
    no_auto_capture: bool,
    ungreedy: bool,
}

pub(crate) struct Pattern {
    pub node: Node,
    /// Capture groups, not counting group 0.
    pub groups: usize,
    pub names: Vec<(String, usize)>,
}

/// The long names of the general categories (loosely matched: lower case,
/// no spaces, underscores or hyphens), which regex-syntax takes.
const LONG_CATEGORY_NAMES: &[&str] = &[
    "letter",
    "uppercaseletter",
    "lowercaseletter",
    "titlecaseletter",
    "modifierletter",
    "otherletter",
    "mark",
    "combiningmark",
    "nonspacingmark",
    "spacingmark",
    "enclosingmark",
    "number",
    "decimalnumber",
    "letternumber",
    "othernumber",
    "punctuation",
    "connectorpunctuation",
    "dashpunctuation",
    "openpunctuation",
    "closepunctuation",
    "initialpunctuation",
    "finalpunctuation",
    "otherpunctuation",
    "symbol",
    "mathsymbol",
    "currencysymbol",
    "modifiersymbol",
    "othersymbol",
    "separator",
    "spaceseparator",
    "lineseparator",
    "paragraphseparator",
    "other",
    "control",
    "format",
    "surrogate",
    "privateuse",
    "unassigned",
    "digit",
    "punct",
    "cntrl",
];

/// Unicode classes by name, through regex-syntax's tables.
fn property_class(name: &str) -> Option<ClassUnicode> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ' ' | '-' | '=' | ':' | '&'))
    {
        return None;
    }
    let hir = regex_syntax::ParserBuilder::new()
        .build()
        .parse(&format!("\\p{{{name}}}"))
        .ok()?;
    match hir.into_kind() {
        regex_syntax::hir::HirKind::Class(regex_syntax::hir::Class::Unicode(class)) => Some(class),
        _ => None,
    }
}

fn class_of(ranges: &[(char, char)]) -> ClassUnicode {
    ClassUnicode::new(ranges.iter().map(|&(a, b)| ClassUnicodeRange::new(a, b)))
}

fn union(mut a: ClassUnicode, b: &ClassUnicode) -> ClassUnicode {
    a.union(b);
    a
}

fn general(name: &str) -> ClassUnicode {
    property_class(name).expect("general category")
}

/// `\h`: horizontal white space.
fn horizontal_space() -> ClassUnicode {
    class_of(&[
        ('\t', '\t'),
        (' ', ' '),
        ('\u{a0}', '\u{a0}'),
        ('\u{1680}', '\u{1680}'),
        ('\u{180e}', '\u{180e}'),
        ('\u{2000}', '\u{200a}'),
        ('\u{202f}', '\u{202f}'),
        ('\u{205f}', '\u{205f}'),
        ('\u{3000}', '\u{3000}'),
    ])
}

/// `\v`: vertical white space.
fn vertical_space() -> ClassUnicode {
    class_of(&[('\n', '\r'), ('\u{85}', '\u{85}'), ('\u{2028}', '\u{2029}')])
}

/// `\d` in UCP mode: decimal digits.
pub(crate) fn digit() -> ClassUnicode {
    general("Nd")
}

/// `\s` in UCP mode: `\p{Z}`, `\h` and `\v`.
pub(crate) fn space() -> ClassUnicode {
    union(union(general("Z"), &horizontal_space()), &vertical_space())
}

/// `\w` in UCP mode: letters, numbers, nonspacing marks and connector
/// punctuation (Excel's `\w` matches U+0301, U+203F, ² and Ⅻ).
pub(crate) fn word() -> ClassUnicode {
    let mut class = general("L");
    for name in ["N", "Mn", "Pc"] {
        class.union(&general(name));
    }
    class
}

fn negated(mut class: ClassUnicode) -> ClassUnicode {
    class.negate();
    class
}

/// A POSIX class name in UCP mode.
fn posix_class(name: &str) -> Result<ClassUnicode, PatternError> {
    Ok(match name {
        "alpha" => general("L"),
        "alnum" => union(general("L"), &general("N")),
        "digit" => digit(),
        "lower" => general("Ll"),
        "upper" => general("Lu"),
        "space" => space(),
        "word" => word(),
        "blank" => horizontal_space(),
        "cntrl" => general("Cc"),
        "xdigit" => class_of(&[('0', '9'), ('A', 'F'), ('a', 'f')]),
        "ascii" => class_of(&[('\0', '\u{7f}')]),
        "punct" => {
            // Punctuation, and the symbols below U+0100.
            let mut symbols = general("S");
            symbols.intersect(&class_of(&[('\0', '\u{ff}')]));
            union(general("P"), &symbols)
        }
        "graph" | "print" => {
            // Letters, marks, numbers, punctuation, symbols and format
            // characters, except U+061C, U+180E and U+2066-U+2069; print adds
            // the space separators but U+180E.
            let mut class = general("L");
            for name in ["M", "N", "P", "S", "Cf"] {
                class.union(&general(name));
            }
            if name == "print" {
                class.union(&general("Zs"));
            }
            class.difference(&class_of(&[
                ('\u{61c}', '\u{61c}'),
                ('\u{180e}', '\u{180e}'),
                ('\u{2066}', '\u{2069}'),
            ]));
            class
        }
        _ => return invalid("unknown POSIX class name"),
    })
}

/// The characters a single character matches: itself, or every character
/// of its simple case folding under caseless matching.
pub(crate) fn char_class(c: char, caseless: bool) -> ClassUnicode {
    let mut class = class_of(&[(c, c)]);
    if caseless {
        class.case_fold_simple();
    }
    class
}

fn char_node(c: char, caseless: bool) -> Node {
    if caseless {
        let class = char_class(c, true);
        if class.ranges().len() != 1 || class.ranges()[0].start() != class.ranges()[0].end() {
            return Node::Set(class);
        }
    }
    Node::Char(c)
}

/// An item of a character class.
enum ClassItem {
    Char(char),
    Set(ClassUnicode),
}

struct Parser<'p> {
    chars: Vec<char>,
    pos: usize,
    flags: Flags,
    groups: usize,
    /// Groups the whole pattern holds, counted before parsing, which decides
    /// whether `\10` is a backreference or an octal escape.
    total_groups: usize,
    names: Vec<(String, usize)>,
    /// Backreferences by name, resolved once every group is known.
    named_refs: Vec<String>,
    /// Where the start-of-pattern options such as `(*UCP)` end.
    leading_options_end: usize,
    /// How many lookaround assertions enclose the cursor (`\K` is refused
    /// inside one, as by Excel).
    look_depth: usize,
    _marker: std::marker::PhantomData<&'p ()>,
}

const MAX_REPEAT: u32 = 65535;

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }
    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn starts_with(&self, s: &str) -> bool {
        s.chars()
            .enumerate()
            .all(|(i, c)| self.peek_at(i) == Some(c))
    }

    /// Skips white space and `#` comments in extended mode.
    fn skip_extended(&mut self) {
        if !self.flags.extended {
            return;
        }
        loop {
            match self.peek() {
                Some(c) if is_pattern_white_space(c) => self.pos += 1,
                Some('#') => {
                    while let Some(c) = self.peek() {
                        self.pos += 1;
                        if c == '\n' {
                            break;
                        }
                    }
                }
                _ => return,
            }
        }
    }

    fn parse_alternation(&mut self) -> Result<Node, PatternError> {
        let mut branches = vec![self.parse_concat()?];
        while self.eat('|') {
            branches.push(self.parse_concat()?);
        }
        Ok(if branches.len() == 1 {
            branches.pop().unwrap()
        } else {
            Node::Alternate(branches)
        })
    }

    fn parse_concat(&mut self) -> Result<Node, PatternError> {
        let mut items = Vec::new();
        loop {
            self.skip_extended();
            match self.peek() {
                None | Some('|') | Some(')') => break,
                _ => {}
            }
            // An option setting such as `(?i)` changes the flags for the rest
            // of the group and matches nothing.
            let Some(atom) = self.parse_atom()? else {
                continue;
            };
            let atom = self.parse_quantifier(atom)?;
            items.push(atom);
        }
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.pop().unwrap(),
            _ => Node::Concat(items),
        })
    }

    fn parse_quantifier(&mut self, atom: Node) -> Result<Node, PatternError> {
        let mut atom = atom;
        loop {
            self.skip_extended();
            let (min, max) = match self.peek() {
                Some('*') => {
                    self.pos += 1;
                    (0, None)
                }
                Some('+') => {
                    self.pos += 1;
                    (1, None)
                }
                Some('?') => {
                    self.pos += 1;
                    (0, Some(1))
                }
                Some('{') => match self.counted_quantifier()? {
                    Some(bounds) => bounds,
                    None => return Ok(atom),
                },
                _ => return Ok(atom),
            };
            if !repeatable(&atom) {
                return invalid("quantifier does not follow a repeatable item");
            }
            let mut greed = Greed::Greedy;
            if self.eat('?') {
                greed = Greed::Lazy;
            } else if self.eat('+') {
                greed = Greed::Possessive;
            }
            if self.flags.ungreedy {
                greed = match greed {
                    Greed::Greedy => Greed::Lazy,
                    Greed::Lazy => Greed::Greedy,
                    Greed::Possessive => Greed::Possessive,
                };
            }
            // A lookaround repeated more than once is the same assertion.
            let (min, max) = match &atom {
                Node::Look { .. } => (min.min(1), max.map(|m| m.min(1)).or(Some(1))),
                _ => (min, max),
            };
            atom = Node::Repeat {
                node: Box::new(atom),
                min,
                max,
                greed,
            };
            // A quantifier may not follow another (`a++?`, `a**`).
            self.skip_extended();
            let follows = match self.peek() {
                Some('*') | Some('+') | Some('?') => true,
                Some('{') => {
                    let save = self.pos;
                    let quantifier = self.counted_quantifier();
                    self.pos = save;
                    !matches!(quantifier, Ok(None))
                }
                _ => false,
            };
            if follows {
                return invalid("quantifier does not follow a repeatable item");
            }
            return Ok(atom);
        }
    }

    /// `{n}`, `{n,}`, `{n,m}`; `None` (a literal `{`) for any other text.
    fn counted_quantifier(&mut self) -> Result<Option<(u32, Option<u32>)>, PatternError> {
        let start = self.pos;
        self.pos += 1;
        let number = |p: &mut Self| -> Option<u64> {
            let begin = p.pos;
            while p.peek().is_some_and(|c| c.is_ascii_digit()) {
                p.pos += 1;
            }
            if p.pos == begin {
                return None;
            }
            let text: String = p.chars[begin..p.pos].iter().collect();
            Some(text.parse().unwrap_or(u64::MAX))
        };
        // `{,n}` is `{0,n}` (PCRE2 10.43 and later, as Excel reads it).
        let min = if self.peek() == Some(',') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit())
        {
            0
        } else {
            match number(self) {
                Some(min) => min,
                None => {
                    self.pos = start;
                    return Ok(None);
                }
            }
        };
        let max = if self.eat(',') {
            if self.peek() == Some('}') {
                None
            } else {
                match number(self) {
                    Some(max) => Some(max),
                    None => {
                        self.pos = start;
                        return Ok(None);
                    }
                }
            }
        } else {
            Some(min)
        };
        if !self.eat('}') {
            self.pos = start;
            return Ok(None);
        }
        if min > MAX_REPEAT as u64 || max.is_some_and(|m| m > MAX_REPEAT as u64) {
            return invalid("number too big in {} quantifier");
        }
        if max.is_some_and(|m| m < min) {
            return invalid("numbers out of order in {} quantifier");
        }
        Ok(Some((min as u32, max.map(|m| m as u32))))
    }

    /// An atom; `None` for an option setting that matches nothing.
    fn parse_atom(&mut self) -> Result<Option<Node>, PatternError> {
        let c = self.peek().unwrap();
        self.pos += 1;
        Ok(Some(match c {
            '(' => return self.parse_group(),
            '[' => Node::Set(self.parse_class()?),
            '.' => {
                if self.flags.dotall {
                    Node::Set(class_of(&[('\0', '\u{10ffff}')]))
                } else {
                    Node::AnyButNewline
                }
            }
            '^' => Node::Assert(Assertion::LineStart {
                multiline: self.flags.multiline,
            }),
            '$' => Node::Assert(Assertion::LineEnd {
                multiline: self.flags.multiline,
            }),
            '\\' => return self.parse_escape().map(Some),
            '*' | '+' | '?' => return invalid("quantifier does not follow a repeatable item"),
            '{' if self.counted_quantifier_ahead() => {
                return invalid("quantifier does not follow a repeatable item");
            }
            c => char_node(c, self.flags.caseless),
        }))
    }

    /// Whether a counted quantifier starts just before the cursor (`{` was
    /// consumed).
    fn counted_quantifier_ahead(&mut self) -> bool {
        self.pos -= 1;
        let save = self.pos;
        let found = matches!(self.counted_quantifier(), Ok(Some(_)) | Err(_));
        self.pos = save + 1;
        found
    }

    fn parse_name(&mut self, terminator: char) -> Result<String, PatternError> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == terminator {
                break;
            }
            if !(c.is_alphanumeric() || c == '_') {
                return invalid("syntax error in subpattern name");
            }
            self.pos += 1;
        }
        let name: String = self.chars[start..self.pos].iter().collect();
        if !self.eat(terminator) {
            return invalid("syntax error in subpattern name (missing terminator)");
        }
        if name.is_empty() || name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return invalid("subpattern name expected");
        }
        if name.chars().count() > 128 {
            return invalid("subpattern name is too long");
        }
        Ok(name)
    }

    fn capture(&mut self, name: Option<String>) -> Result<Option<Node>, PatternError> {
        self.groups += 1;
        let index = self.groups;
        if let Some(name) = name {
            if self.names.iter().any(|(n, _)| *n == name) {
                return invalid("two named subpatterns have the same name");
            }
            self.names.push((name, index));
        }
        let saved = self.flags;
        let node = self.parse_alternation()?;
        self.flags = saved;
        if !self.eat(')') {
            return invalid("missing closing parenthesis");
        }
        Ok(Some(Node::Capture {
            index,
            node: Box::new(node),
        }))
    }

    fn look_body(&mut self) -> Result<Node, PatternError> {
        self.look_depth += 1;
        let node = self.group_body();
        self.look_depth -= 1;
        node
    }

    fn group_body(&mut self) -> Result<Node, PatternError> {
        let saved = self.flags;
        let node = self.parse_alternation()?;
        self.flags = saved;
        if !self.eat(')') {
            return invalid("missing closing parenthesis");
        }
        Ok(node)
    }

    fn parse_group(&mut self) -> Result<Option<Node>, PatternError> {
        if self.eat('*') {
            return self.parse_verb();
        }
        if !self.eat('?') {
            if self.flags.no_auto_capture {
                return self.group_body().map(Some);
            }
            return self.capture(None);
        }
        let Some(c) = self.peek() else {
            return invalid("missing closing parenthesis");
        };
        match c {
            '#' => {
                while let Some(c) = self.peek() {
                    self.pos += 1;
                    if c == ')' {
                        return Ok(None);
                    }
                }
                invalid("missing ) after comment")
            }
            ':' => {
                self.pos += 1;
                self.group_body().map(Some)
            }
            '|' => {
                self.pos += 1;
                self.branch_reset().map(Some)
            }
            '>' => {
                self.pos += 1;
                Ok(Some(Node::Atomic(Box::new(self.group_body()?))))
            }
            '=' | '!' => {
                self.pos += 1;
                let node = self.look_body()?;
                Ok(Some(Node::Look {
                    behind: false,
                    negate: c == '!',
                    node: Box::new(node),
                }))
            }
            '<' if matches!(self.peek_at(1), Some('=') | Some('!')) => {
                let negate = self.peek_at(1) == Some('!');
                self.pos += 2;
                let node = self.look_body()?;
                Ok(Some(Node::Look {
                    behind: true,
                    negate,
                    node: Box::new(node),
                }))
            }
            '<' => {
                self.pos += 1;
                let name = self.parse_name('>')?;
                self.capture(Some(name))
            }
            '\'' => {
                self.pos += 1;
                let name = self.parse_name('\'')?;
                self.capture(Some(name))
            }
            'P' => {
                self.pos += 1;
                match self.peek() {
                    Some('<') => {
                        self.pos += 1;
                        let name = self.parse_name('>')?;
                        self.capture(Some(name))
                    }
                    Some('=') => {
                        self.pos += 1;
                        let name = self.parse_name(')')?;
                        Ok(Some(self.named_backref(name)))
                    }
                    Some('>') => unsupported("subroutine call"),
                    _ => invalid("unrecognized character after (?P"),
                }
            }
            '(' => {
                self.pos += 1;
                self.conditional().map(Some)
            }
            'R' | '&' | '+' | '0'..='9' => unsupported("recursion or subroutine call"),
            '-' if self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) => {
                unsupported("subroutine call")
            }
            'C' => invalid("callouts are not allowed"),
            _ => self.option_setting(),
        }
    }

    /// `(?imnsxU-imnsx)` for the rest of the group, `(?imnsxU-imnsx:...)` for
    /// a group of its own, `(?^...)` from the defaults.
    fn option_setting(&mut self) -> Result<Option<Node>, PatternError> {
        let mut flags = self.flags;
        let mut on = true;
        if self.eat('^') {
            flags.caseless = false;
            flags.multiline = false;
            flags.dotall = false;
            flags.extended = false;
            flags.extended_more = false;
            flags.no_auto_capture = false;
            flags.ungreedy = false;
        }
        loop {
            let Some(c) = self.peek() else {
                return invalid("missing closing parenthesis");
            };
            self.pos += 1;
            match c {
                'i' => flags.caseless = on,
                'm' => flags.multiline = on,
                's' => flags.dotall = on,
                'n' => flags.no_auto_capture = on,
                'U' => flags.ungreedy = on,
                'x' => {
                    if on && self.peek() == Some('x') {
                        self.pos += 1;
                        flags.extended_more = true;
                    } else {
                        flags.extended_more = false;
                    }
                    flags.extended = on;
                }
                'J' => return unsupported("duplicate group names (?J)"),
                '-' if on => on = false,
                ')' => {
                    self.flags = flags;
                    return Ok(None);
                }
                ':' => {
                    let saved = self.flags;
                    self.flags = flags;
                    let node = self.parse_alternation()?;
                    self.flags = saved;
                    if !self.eat(')') {
                        return invalid("missing closing parenthesis");
                    }
                    return Ok(Some(node));
                }
                _ => return invalid("unrecognized character after (? or (?-"),
            }
        }
    }

    /// `(?|...)`: each alternative numbers its groups from the same start.
    fn branch_reset(&mut self) -> Result<Node, PatternError> {
        let saved = self.flags;
        let start = self.groups;
        let mut highest = start;
        let mut branches = Vec::new();
        loop {
            self.groups = start;
            branches.push(self.parse_concat()?);
            highest = highest.max(self.groups);
            if !self.eat('|') {
                break;
            }
        }
        self.groups = highest;
        self.flags = saved;
        if !self.eat(')') {
            return invalid("missing closing parenthesis");
        }
        Ok(Node::Alternate(branches))
    }

    fn conditional(&mut self) -> Result<Node, PatternError> {
        // The condition: a group number or name, or an assertion.
        let condition = if self.peek().is_some_and(|c| c.is_ascii_digit()) {
            let start = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
            let digits: String = self.chars[start..self.pos].iter().collect();
            if !self.eat(')') {
                return invalid("malformed number or name after (?(");
            }
            let group: usize = digits.parse().unwrap_or(usize::MAX);
            if group == 0 || group > self.total_groups {
                return invalid("reference to non-existent subpattern");
            }
            Condition::Group(group)
        } else if matches!(self.peek(), Some('+') | Some('-')) {
            return unsupported("relative conditional reference");
        } else if self.eat('<') {
            let name = self.parse_name('>')?;
            if !self.eat(')') {
                return invalid("malformed number or name after (?(");
            }
            self.named_refs.push(name.clone());
            Condition::Group(self.name_placeholder(&name))
        } else if self.eat('\'') {
            let name = self.parse_name('\'')?;
            if !self.eat(')') {
                return invalid("malformed number or name after (?(");
            }
            self.named_refs.push(name.clone());
            Condition::Group(self.name_placeholder(&name))
        } else if self.eat('?') {
            let (behind, negate) = match (self.peek(), self.peek_at(1)) {
                (Some('='), _) => (false, false),
                (Some('!'), _) => (false, true),
                (Some('<'), Some('=')) => (true, false),
                (Some('<'), Some('!')) => (true, true),
                _ => return invalid("assertion expected after (?( or (?(?C)"),
            };
            self.pos += if behind { 2 } else { 1 };
            let node = self.look_body()?;
            Condition::Look(Box::new(Node::Look {
                behind,
                negate,
                node: Box::new(node),
            }))
        } else if self.starts_with("R") || self.starts_with("DEFINE") {
            return unsupported("recursion condition or DEFINE");
        } else {
            // A bare name: the group of that name.
            let start = self.pos;
            while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                self.pos += 1;
            }
            let name: String = self.chars[start..self.pos].iter().collect();
            if name.is_empty() || !self.eat(')') {
                return invalid("malformed number or name after (?(");
            }
            self.named_refs.push(name.clone());
            Condition::Group(self.name_placeholder(&name))
        };
        let saved = self.flags;
        let yes = self.parse_concat()?;
        let no = if self.eat('|') {
            self.parse_concat()?
        } else {
            Node::Empty
        };
        self.flags = saved;
        if self.peek() == Some('|') {
            return invalid("conditional subpattern contains more than two branches");
        }
        if !self.eat(')') {
            return invalid("missing closing parenthesis");
        }
        Ok(Node::Conditional {
            condition,
            yes: Box::new(yes),
            no: Box::new(no),
        })
    }

    /// A temporary group number for a name, resolved after parsing.
    fn name_placeholder(&self, name: &str) -> usize {
        usize::MAX - self.named_refs.iter().position(|n| n == name).unwrap()
    }

    fn named_backref(&mut self, name: String) -> Node {
        self.named_refs.push(name.clone());
        Node::Backref {
            group: self.name_placeholder(&name),
            caseless: self.flags.caseless,
        }
    }

    fn parse_verb(&mut self) -> Result<Option<Node>, PatternError> {
        let open = self.pos - 2;
        let start = self.pos;
        while let Some(c) = self.peek() {
            self.pos += 1;
            if c == ')' {
                let verb: String = self.chars[start..self.pos - 1].iter().collect();
                // Options Excel already sets, or that change no result, at
                // the start of the pattern.
                let leading = open == self.leading_options_end;
                return match verb.as_str() {
                    "FAIL" | "F" => Ok(Some(Node::Fail)),
                    "UTF" | "UCP" | "NO_AUTO_POSSESS" | "NO_DOTSTAR_ANCHOR" | "NO_JIT"
                    | "NO_START_OPT"
                        if leading =>
                    {
                        self.leading_options_end = self.pos;
                        Ok(None)
                    }
                    _ => unsupported("backtracking control verb or start option"),
                };
            }
        }
        invalid("(*VERB) not terminated")
    }

    fn parse_hex(&mut self, max_digits: usize) -> u32 {
        let mut value = 0u32;
        let mut count = 0;
        while count < max_digits {
            match self.peek().and_then(|c| c.to_digit(16)) {
                Some(d) => {
                    value = value * 16 + d;
                    self.pos += 1;
                    count += 1;
                }
                None => break,
            }
        }
        value
    }

    fn braced_number(&mut self, radix: u32) -> Result<u32, PatternError> {
        if !self.eat('{') {
            return invalid("missing opening brace");
        }
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_digit(radix)) {
            self.pos += 1;
        }
        let digits: String = self.chars[start..self.pos].iter().collect();
        if digits.is_empty() || !self.eat('}') {
            return invalid("non-hex or non-octal character in braces");
        }
        u32::from_str_radix(&digits, radix)
            .or_else(|_| invalid("character code point value is too large"))
    }

    fn code_point(value: u32) -> Result<char, PatternError> {
        char::from_u32(value).map_or_else(
            || invalid("character code point value is too large or a surrogate"),
            Ok,
        )
    }

    /// An escape that stands for one character, shared by classes and atoms;
    /// `None` when the escape is something else.
    fn escaped_char(&mut self, c: char) -> Result<Option<char>, PatternError> {
        Ok(Some(match c {
            'a' => '\u{7}',
            'e' => '\u{1b}',
            'f' => '\u{c}',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'x' => {
                if self.peek() == Some('{') {
                    Self::code_point(self.braced_number(16)?)?
                } else {
                    Self::code_point(self.parse_hex(2))?
                }
            }
            'o' => Self::code_point(self.braced_number(8)?)?,
            '0' => {
                // Excel refuses `\0` that no octal digit follows (`a\08`), as
                // with PCRE2's EXTRA_NO_BS0; `\000` and `\x00` are the NUL.
                if !self.peek().is_some_and(|c| c.is_digit(8)) {
                    return invalid("\\0 is not followed by an octal digit");
                }
                let mut value = 0u32;
                let mut count = 0;
                while count < 2 {
                    match self.peek().and_then(|c| c.to_digit(8)) {
                        Some(d) => {
                            value = value * 8 + d;
                            self.pos += 1;
                            count += 1;
                        }
                        None => break,
                    }
                }
                Self::code_point(value)?
            }
            'c' => {
                let Some(x) = self
                    .peek()
                    .filter(|x| x.is_ascii() && !x.is_ascii_control())
                else {
                    return invalid("\\c must be followed by a printable ASCII character");
                };
                self.pos += 1;
                Self::code_point((x.to_ascii_uppercase() as u32) ^ 0x40)?
            }
            'N' if self.starts_with("{U+") => {
                self.pos += 3;
                let start = self.pos;
                let value = self.parse_hex(8);
                if self.pos == start || !self.eat('}') {
                    return invalid("malformed \\N{U+dddd}");
                }
                Self::code_point(value)?
            }
            _ => return Ok(None),
        }))
    }

    /// A class escape: `\d`, `\p{..}`, ... ; `None` for other escapes.
    fn class_escape(&mut self, c: char) -> Result<Option<ClassUnicode>, PatternError> {
        Ok(Some(match c {
            'd' => digit(),
            'D' => negated(digit()),
            'w' => word(),
            'W' => negated(word()),
            's' => space(),
            'S' => negated(space()),
            'h' => horizontal_space(),
            'H' => negated(horizontal_space()),
            'v' => vertical_space(),
            'V' => negated(vertical_space()),
            'p' | 'P' => {
                let mut negate = c == 'P';
                let name = if self.eat('{') {
                    if self.eat('^') {
                        negate = !negate;
                    }
                    let start = self.pos;
                    while self.peek().is_some_and(|c| c != '}') {
                        self.pos += 1;
                    }
                    let name: String = self.chars[start..self.pos].iter().collect();
                    if !self.eat('}') {
                        return invalid("malformed \\P or \\p sequence");
                    }
                    name
                } else {
                    match self.peek() {
                        Some(c) if c.is_ascii_alphabetic() => {
                            self.pos += 1;
                            c.to_string()
                        }
                        _ => return invalid("malformed \\P or \\p sequence"),
                    }
                };
                let class = self.property(&name)?;
                if negate { negated(class) } else { class }
            }
            _ => return Ok(None),
        }))
    }

    fn property(&self, name: &str) -> Result<ClassUnicode, PatternError> {
        let key: String = name
            .chars()
            .filter(|c| !matches!(c, ' ' | '_' | '-'))
            .collect::<String>()
            .to_ascii_lowercase();
        let caseless = self.flags.caseless;
        Ok(match key.as_str() {
            "any" => class_of(&[('\0', '\u{10ffff}')]),
            "l&" | "lc" => general("LC"),
            // Under caseless matching the cased letter categories match
            // either case (Excel: REGEXTEST("a","\p{Lu}",1) is TRUE).
            "lu" | "ll" | "lt" if caseless => general("LC"),
            "xan" => union(general("L"), &general("N")),
            "xsp" | "xps" => space(),
            "xwd" => word(),
            "xuc" => return unsupported("\\p{Xuc}"),
            // PCRE2 refuses a general category's long name (Excel:
            // \p{Cased_Letter} is #VALUE!); regex-syntax would take it.
            "casedletter" => return invalid("unknown property name after \\P or \\p"),
            _ if LONG_CATEGORY_NAMES.contains(&key.as_str()) => {
                return unsupported("long general category name");
            }
            // Of the `name=value` forms only scripts and script extensions
            // are read here (PCRE2's bidi classes are not computed).
            _ if key.contains('=') || key.contains(':') => {
                let kind = key.split(['=', ':']).next().unwrap_or("");
                if kind == "age" {
                    // Excel: \p{Age=1.1} is #VALUE!.
                    return invalid("unknown property name after \\P or \\p");
                }
                if !matches!(kind, "sc" | "scx" | "script" | "scriptextensions") {
                    return unsupported("property type");
                }
                match property_class(name) {
                    Some(class) => class,
                    None => return invalid("unknown property name after \\P or \\p"),
                }
            }
            // regex-syntax knows the general categories, scripts and binary
            // properties PCRE2 knows; any other name is unknown.
            _ => match property_class(name) {
                Some(class) => class,
                None => return invalid("unknown property name after \\P or \\p"),
            },
        })
    }

    fn parse_escape(&mut self) -> Result<Node, PatternError> {
        let Some(c) = self.peek() else {
            return invalid("\\ at end of pattern");
        };
        self.pos += 1;
        if let Some(class) = self.class_escape(c)? {
            return Ok(Node::Set(class));
        }
        if let Some(ch) = self.escaped_char(c)? {
            return Ok(char_node(ch, self.flags.caseless));
        }
        Ok(match c {
            'b' => Node::Assert(Assertion::WordBoundary),
            'B' => Node::Assert(Assertion::NotWordBoundary),
            'A' => Node::Assert(Assertion::TextStart),
            'Z' => Node::Assert(Assertion::TextEndNewline),
            'z' => Node::Assert(Assertion::TextEnd),
            'G' => Node::Assert(Assertion::SearchStart),
            'K' if self.look_depth > 0 => {
                return invalid("\\K is not allowed in lookarounds");
            }
            'K' => Node::KeepOut,
            'N' => Node::AnyButNewline,
            'R' => {
                // (?>\r\n|\n|\x0b|\f|\r|\x85|\x{2028}|\x{2029})
                Node::Atomic(Box::new(Node::Alternate(vec![
                    Node::Concat(vec![Node::Char('\r'), Node::Char('\n')]),
                    Node::Set(vertical_space()),
                ])))
            }
            'Q' => {
                let mut items = Vec::new();
                while self.pos < self.chars.len() {
                    if self.starts_with("\\E") {
                        self.pos += 2;
                        break;
                    }
                    items.push(char_node(self.chars[self.pos], self.flags.caseless));
                    self.pos += 1;
                }
                Node::Concat(items)
            }
            'E' => Node::Empty,
            'g' => self.g_escape()?,
            'k' => {
                let terminator = match self.peek() {
                    Some('<') => '>',
                    Some('\'') => '\'',
                    Some('{') => '}',
                    _ => return invalid("\\k is not followed by a name"),
                };
                self.pos += 1;
                let name = self.parse_name(terminator)?;
                self.named_backref(name)
            }
            '1'..='9' => {
                let start = self.pos - 1;
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.pos += 1;
                }
                let digits: String = self.chars[start..self.pos].iter().collect();
                let number: usize = digits.parse().unwrap_or(usize::MAX);
                if number < 10 || c == '8' || c == '9' || number <= self.total_groups {
                    if number > self.total_groups {
                        return invalid("reference to non-existent subpattern");
                    }
                    Node::Backref {
                        group: number,
                        caseless: self.flags.caseless,
                    }
                } else {
                    // Up to three octal digits.
                    self.pos = start;
                    let mut value = 0u32;
                    let mut count = 0;
                    while count < 3 {
                        match self.peek().and_then(|c| c.to_digit(8)) {
                            Some(d) => {
                                value = value * 8 + d;
                                self.pos += 1;
                                count += 1;
                            }
                            None => break,
                        }
                    }
                    char_node(Self::code_point(value)?, self.flags.caseless)
                }
            }
            'X' => return unsupported("\\X"),
            'C' => return unsupported("\\C"),
            c if c.is_ascii_alphanumeric() => {
                return invalid("unrecognized character follows \\");
            }
            c => char_node(c, self.flags.caseless),
        })
    }

    /// `\g{n}`, `\gn`, `\g{-n}`, `\g-n`, `\g{name}`; `\g<..>` calls a group.
    fn g_escape(&mut self) -> Result<Node, PatternError> {
        if matches!(self.peek(), Some('<') | Some('\'')) {
            return unsupported("subroutine call");
        }
        let braced = self.eat('{');
        let negative = self.eat('-');
        let _ = !negative && self.eat('+');
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.pos == start {
            if braced && !negative {
                let name = self.parse_name('}')?;
                return Ok(self.named_backref(name));
            }
            return invalid("a numbered reference must not be zero");
        }
        let digits: String = self.chars[start..self.pos].iter().collect();
        if braced && !self.eat('}') {
            return invalid(
                "\\g is not followed by a braced, angle-bracketed, or quoted name/number or by a plain number",
            );
        }
        let number: usize = digits.parse().unwrap_or(usize::MAX);
        let group = if negative {
            if number == 0 || number > self.groups {
                return invalid("reference to non-existent subpattern");
            }
            self.groups + 1 - number
        } else {
            number
        };
        if group == 0 || group > self.total_groups {
            return invalid("reference to non-existent subpattern");
        }
        Ok(Node::Backref {
            group,
            caseless: self.flags.caseless,
        })
    }

    fn class_item(&mut self) -> Result<ClassItem, PatternError> {
        let c = self.peek().unwrap();
        self.pos += 1;
        if c == '[' && self.peek() == Some(':') {
            let save = self.pos;
            self.pos += 1;
            let negate = self.eat('^');
            let start = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                self.pos += 1;
            }
            let name: String = self.chars[start..self.pos].iter().collect();
            if self.eat(':') && self.eat(']') {
                if name == "<" || name == ">" {
                    return unsupported("[[:<:]] word boundary");
                }
                let class = posix_class(&name)?;
                return Ok(ClassItem::Set(if negate { negated(class) } else { class }));
            }
            self.pos = save;
        }
        if c != '\\' {
            return Ok(ClassItem::Char(c));
        }
        let Some(e) = self.peek() else {
            return invalid("\\ at end of pattern");
        };
        self.pos += 1;
        if let Some(class) = self.class_escape(e)? {
            return Ok(ClassItem::Set(class));
        }
        if let Some(ch) = self.escaped_char(e)? {
            return Ok(ClassItem::Char(ch));
        }
        Ok(match e {
            'b' => ClassItem::Char('\u{8}'),
            '1'..='7' => {
                // Octal in a class.
                self.pos -= 1;
                let mut value = 0u32;
                let mut count = 0;
                while count < 3 {
                    match self.peek().and_then(|c| c.to_digit(8)) {
                        Some(d) => {
                            value = value * 8 + d;
                            self.pos += 1;
                            count += 1;
                        }
                        None => break,
                    }
                }
                ClassItem::Char(Self::code_point(value)?)
            }
            'N' | 'R' | 'X' | 'B' => {
                return invalid("escape sequence is invalid in character class");
            }
            c if c.is_ascii_alphanumeric() => {
                return invalid("unrecognized character follows \\");
            }
            c => ClassItem::Char(c),
        })
    }

    /// Skips white space in a class under `(?xx)`, and `\Q..\E` and `\E`.
    fn skip_in_class(&mut self, quoted: &mut bool) {
        loop {
            if *quoted {
                if self.starts_with("\\E") {
                    self.pos += 2;
                    *quoted = false;
                    continue;
                }
                return;
            }
            if self.starts_with("\\Q") {
                self.pos += 2;
                *quoted = true;
                continue;
            }
            if self.starts_with("\\E") {
                self.pos += 2;
                continue;
            }
            if self.flags.extended_more && matches!(self.peek(), Some(' ') | Some('\t')) {
                self.pos += 1;
                continue;
            }
            return;
        }
    }

    fn parse_class(&mut self) -> Result<ClassUnicode, PatternError> {
        let mut quoted = false;
        self.skip_in_class(&mut quoted);
        let negate = !quoted && self.eat('^');
        // Characters and ranges fold under caseless matching; POSIX classes
        // and escapes such as \d do not (Excel: REGEXTEST("a","[[:upper:]]",1)
        // is FALSE).
        let mut class = ClassUnicode::empty();
        let mut sets = ClassUnicode::empty();
        let mut first = true;
        loop {
            self.skip_in_class(&mut quoted);
            let Some(c) = self.peek() else {
                return invalid("missing terminating ] for character class");
            };
            if c == ']' && !first && !quoted {
                self.pos += 1;
                break;
            }
            first = false;
            let item = if quoted {
                self.pos += 1;
                ClassItem::Char(c)
            } else {
                self.class_item()?
            };
            // A range `a-z`.
            self.skip_in_class(&mut quoted);
            let range_follows =
                self.peek() == Some('-') && !quoted && self.peek_at(1).is_some_and(|c| c != ']');
            if range_follows {
                let save = self.pos;
                self.pos += 1;
                self.skip_in_class(&mut quoted);
                let end = if quoted {
                    let c = self.peek().unwrap();
                    self.pos += 1;
                    ClassItem::Char(c)
                } else {
                    self.class_item()?
                };
                match (&item, end) {
                    (ClassItem::Char(a), ClassItem::Char(b)) => {
                        if b < *a {
                            return invalid("range out of order in character class");
                        }
                        class.push(ClassUnicodeRange::new(*a, b));
                        continue;
                    }
                    (ClassItem::Set(_), _) | (_, ClassItem::Set(_)) => {
                        let _ = save;
                        return invalid("invalid range in character class");
                    }
                }
            }
            match item {
                ClassItem::Char(c) => class.push(ClassUnicodeRange::new(c, c)),
                ClassItem::Set(set) => sets.union(&set),
            }
        }
        if self.flags.caseless {
            class.case_fold_simple();
        }
        class.union(&sets);
        if negate {
            class.negate();
        }
        Ok(class)
    }
}

fn is_pattern_white_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\u{b}'
            | '\u{c}'
            | '\r'
            | ' '
            | '\u{85}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

fn repeatable(node: &Node) -> bool {
    !matches!(node, Node::Assert(_) | Node::KeepOut | Node::Empty)
}

/// Counts the capture groups of a pattern before parsing it, for the rule
/// that `\10` is a backreference only when there are ten groups.
fn count_groups(chars: &[char]) -> usize {
    let mut count = 0;
    let mut i = 0;
    let mut in_class = false;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                if chars.get(i + 1) == Some(&'Q') {
                    i += 2;
                    while i < chars.len() && !(chars[i] == '\\' && chars.get(i + 1) == Some(&'E')) {
                        i += 1;
                    }
                }
                i += 2;
                continue;
            }
            '[' if !in_class => in_class = true,
            ']' if in_class => in_class = false,
            '(' if !in_class => match chars.get(i + 1) {
                Some('?') => match chars.get(i + 2) {
                    Some('<') if !matches!(chars.get(i + 3), Some('=') | Some('!')) => count += 1,
                    Some('\'') => count += 1,
                    Some('P') if chars.get(i + 3) == Some(&'<') => count += 1,
                    _ => {}
                },
                Some('*') => {}
                _ => count += 1,
            },
            _ => {}
        }
        i += 1;
    }
    count
}

fn resolve_names(
    node: &mut Node,
    refs: &[String],
    names: &[(String, usize)],
) -> Result<(), PatternError> {
    let resolve = |group: &mut usize| -> Result<(), PatternError> {
        if *group > usize::MAX - refs.len() {
            let name = &refs[usize::MAX - *group];
            match names.iter().find(|(n, _)| n == name) {
                Some((_, index)) => *group = *index,
                None => return invalid("reference to non-existent subpattern"),
            }
        }
        Ok(())
    };
    match node {
        Node::Backref { group, .. } => resolve(group)?,
        Node::Conditional { condition, yes, no } => {
            match condition {
                Condition::Group(group) => resolve(group)?,
                Condition::Look(look) => resolve_names(look, refs, names)?,
            }
            resolve_names(yes, refs, names)?;
            resolve_names(no, refs, names)?;
        }
        Node::Concat(items) | Node::Alternate(items) => {
            for item in items {
                resolve_names(item, refs, names)?;
            }
        }
        Node::Capture { node, .. }
        | Node::Repeat { node, .. }
        | Node::Atomic(node)
        | Node::Look { node, .. } => resolve_names(node, refs, names)?,
        _ => {}
    }
    Ok(())
}

/// Parses a pattern with PCRE2's syntax and Excel's options.
pub(crate) fn parse(pattern: &str, caseless: bool) -> Result<Pattern, PatternError> {
    let chars: Vec<char> = pattern.chars().collect();
    if chars.contains(&'\0') {
        return unsupported("NUL in pattern");
    }
    let total_groups = count_groups(&chars);
    let mut parser = Parser {
        chars,
        pos: 0,
        flags: Flags {
            caseless,
            multiline: false,
            dotall: false,
            extended: false,
            extended_more: false,
            no_auto_capture: false,
            ungreedy: false,
        },
        groups: 0,
        total_groups,
        names: Vec::new(),
        named_refs: Vec::new(),
        leading_options_end: 0,
        look_depth: 0,
        _marker: std::marker::PhantomData,
    };
    let mut node = parser.parse_alternation()?;
    if parser.pos < parser.chars.len() {
        return invalid("unmatched closing parenthesis");
    }
    if parser.groups != total_groups {
        // The pre-count and the parse disagree (branch reset, (?n)): numbered
        // references were checked against the count, so recheck them.
        if parser.groups < total_groups {
            check_backrefs(&node, parser.groups)?;
        }
    }
    resolve_names(&mut node, &parser.named_refs, &parser.names)?;
    Ok(Pattern {
        node,
        groups: parser.groups,
        names: parser.names,
    })
}

fn check_backrefs(node: &Node, groups: usize) -> Result<(), PatternError> {
    match node {
        Node::Backref { group, .. } if *group <= usize::MAX / 2 && *group > groups => {
            invalid("reference to non-existent subpattern")
        }
        Node::Conditional { condition, yes, no } => {
            if let Condition::Group(group) = condition
                && *group <= usize::MAX / 2
                && *group > groups
            {
                return invalid("reference to non-existent subpattern");
            }
            check_backrefs(yes, groups)?;
            check_backrefs(no, groups)
        }
        Node::Concat(items) | Node::Alternate(items) => items
            .iter()
            .try_for_each(|item| check_backrefs(item, groups)),
        Node::Capture { node, .. }
        | Node::Repeat { node, .. }
        | Node::Atomic(node)
        | Node::Look { node, .. } => check_backrefs(node, groups),
        _ => Ok(()),
    }
}
