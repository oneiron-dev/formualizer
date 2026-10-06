//! REGEXREPLACE's replacement text, with PCRE2's extended substitution
//! syntax as Excel for Windows reads it: `$n`, `${n}`, `$name`, `${name}`,
//! `$$`, `$&`, `` $` `` (the text before the match), `$'` (after it), `$_`
//! (the whole text), `$*MARK` (no mark: empty), `${n:-default}`,
//! `${n:+set:unset}`, backslash escapes, `\1` and `\g{1}` group references and
//! the case forcing `\U`, `\L`, `\E`, `\u`, `\l`.

use super::syntax::PatternError;

#[derive(Debug, Clone)]
pub(crate) enum Piece {
    Text(char),
    Group(usize),
    /// `` $` ``: the text before the match.
    Before,
    /// `$'`: the text after the match.
    After,
    /// `$_`: the whole text.
    Subject,
    /// `$*MARK`: the last mark's name, which is none here.
    Nothing,
    /// `${n:-default}` and `${n:+set:unset}`.
    IfSet {
        group: usize,
        set: Vec<Piece>,
        unset: Vec<Piece>,
        /// `:-`: the group's own value when it is set.
        own_when_set: bool,
    },
    Upper,
    Lower,
    EndCase,
    UpperNext,
    LowerNext,
}

fn invalid<T>(why: &str) -> Result<T, PatternError> {
    Err(PatternError::Invalid(why.to_string()))
}

struct Reader<'a> {
    chars: &'a [char],
    pos: usize,
    groups: usize,
    names: &'a [(String, usize)],
}

impl Reader<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn group(&self, number: usize) -> Result<usize, PatternError> {
        if number > self.groups {
            invalid("unknown substring")
        } else {
            Ok(number)
        }
    }

    fn named(&self, name: &str) -> Result<usize, PatternError> {
        match self.names.iter().find(|(n, _)| n == name) {
            Some((_, index)) => Ok(*index),
            None => invalid("unknown substring"),
        }
    }

    /// The pieces up to the end, or (inside `${..}`) up to an unescaped
    /// `stop` character, which is consumed.
    fn pieces(&mut self, stop: &[char]) -> Result<Vec<Piece>, PatternError> {
        let mut out = Vec::new();
        loop {
            let Some(c) = self.peek() else {
                if stop.is_empty() {
                    return Ok(out);
                }
                return invalid("missing terminating } in replacement");
            };
            if stop.contains(&c) {
                return Ok(out);
            }
            self.pos += 1;
            match c {
                '$' => out.push(self.dollar()?),
                '\\' => self.escape(&mut out)?,
                c => out.push(Piece::Text(c)),
            }
        }
    }

    fn digits(&mut self) -> Option<usize> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        (self.pos > start).then(|| {
            self.chars[start..self.pos]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or(usize::MAX)
        })
    }

    fn name(&mut self) -> Option<String> {
        let start = self.pos;
        if self.peek().is_some_and(|c| c.is_alphabetic() || c == '_') {
            while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                self.pos += 1;
            }
        }
        (self.pos > start).then(|| self.chars[start..self.pos].iter().collect())
    }

    fn dollar(&mut self) -> Result<Piece, PatternError> {
        match self.peek() {
            Some('$') => {
                self.pos += 1;
                Ok(Piece::Text('$'))
            }
            Some('&') => {
                self.pos += 1;
                Ok(Piece::Group(0))
            }
            Some('`') => {
                self.pos += 1;
                Ok(Piece::Before)
            }
            Some('\'') => {
                self.pos += 1;
                Ok(Piece::After)
            }
            Some('_')
                if !self
                    .chars
                    .get(self.pos + 1)
                    .is_some_and(|c| c.is_alphanumeric() || *c == '_') =>
            {
                self.pos += 1;
                Ok(Piece::Subject)
            }
            Some('*') => {
                // The name of the last (*MARK): none, as marks are not
                // computed here.
                self.pos += 1;
                if self.name().as_deref() != Some("MARK") {
                    return invalid("bad substitution in replacement");
                }
                Ok(Piece::Nothing)
            }
            Some('{') if self.chars.get(self.pos + 1) == Some(&'*') => {
                self.pos += 2;
                if self.name().as_deref() != Some("MARK") || self.peek() != Some('}') {
                    return invalid("bad substitution in replacement");
                }
                self.pos += 1;
                Ok(Piece::Nothing)
            }
            Some('{') => {
                self.pos += 1;
                let group = match self.digits() {
                    Some(number) => self.group(number)?,
                    None => match self.name() {
                        Some(name) => self.named(&name)?,
                        None => return invalid("bad substitution in replacement"),
                    },
                };
                match self.peek() {
                    Some('}') => {
                        self.pos += 1;
                        Ok(Piece::Group(group))
                    }
                    Some(':') => {
                        self.pos += 1;
                        match self.peek() {
                            Some('-') => {
                                self.pos += 1;
                                let unset = self.pieces(&['}'])?;
                                self.pos += 1;
                                Ok(Piece::IfSet {
                                    group,
                                    set: Vec::new(),
                                    unset,
                                    own_when_set: true,
                                })
                            }
                            Some('+') => {
                                self.pos += 1;
                                let set = self.pieces(&[':', '}'])?;
                                let unset = if self.peek() == Some(':') {
                                    self.pos += 1;
                                    self.pieces(&['}'])?
                                } else {
                                    Vec::new()
                                };
                                self.pos += 1;
                                Ok(Piece::IfSet {
                                    group,
                                    set,
                                    unset,
                                    own_when_set: false,
                                })
                            }
                            _ => invalid("bad substitution in replacement"),
                        }
                    }
                    _ => invalid("bad substitution in replacement"),
                }
            }
            _ => match self.digits() {
                Some(number) => Ok(Piece::Group(self.group(number)?)),
                None => match self.name() {
                    Some(name) => Ok(Piece::Group(self.named(&name)?)),
                    None => invalid("bad substitution in replacement"),
                },
            },
        }
    }

    fn escape(&mut self, out: &mut Vec<Piece>) -> Result<(), PatternError> {
        let Some(c) = self.peek() else {
            return invalid("\\ at end of replacement");
        };
        self.pos += 1;
        let ch = match c {
            'U' => {
                out.push(Piece::Upper);
                return Ok(());
            }
            'L' => {
                out.push(Piece::Lower);
                return Ok(());
            }
            'E' => {
                out.push(Piece::EndCase);
                return Ok(());
            }
            'u' => {
                out.push(Piece::UpperNext);
                return Ok(());
            }
            'l' => {
                out.push(Piece::LowerNext);
                return Ok(());
            }
            'Q' => {
                while self.pos < self.chars.len() {
                    if self.chars[self.pos] == '\\' && self.chars.get(self.pos + 1) == Some(&'E') {
                        self.pos += 2;
                        break;
                    }
                    out.push(Piece::Text(self.chars[self.pos]));
                    self.pos += 1;
                }
                return Ok(());
            }
            'g' => {
                // `\g<n>` and `\g<name>` refer to a group (Excel refuses
                // `\g1`, `\g{1}` and `\g{-1}` here).
                if self.peek() != Some('<') {
                    return invalid("bad \\g in replacement");
                }
                self.pos += 1;
                let group = match self.digits() {
                    Some(number) => self.group(number)?,
                    None => match self.name() {
                        Some(name) => self.named(&name)?,
                        None => return invalid("bad \\g in replacement"),
                    },
                };
                if self.peek() != Some('>') {
                    return invalid("bad \\g in replacement");
                }
                self.pos += 1;
                out.push(Piece::Group(group));
                return Ok(());
            }
            'N' if self.chars.get(self.pos..self.pos + 3) == Some(&['{', 'U', '+'][..]) => {
                self.pos += 3;
                let start = self.pos;
                while self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                    self.pos += 1;
                }
                let hex: String = self.chars[start..self.pos].iter().collect();
                if hex.is_empty() || self.peek() != Some('}') {
                    return invalid("malformed \\N{U+dddd} in replacement");
                }
                self.pos += 1;
                code_point(u32::from_str_radix(&hex, 16).unwrap_or(u32::MAX))?
            }
            'c' => {
                let Some(x) = self
                    .peek()
                    .filter(|x| x.is_ascii() && !x.is_ascii_control())
                else {
                    return invalid("\\c must be followed by a printable ASCII character");
                };
                self.pos += 1;
                code_point((x.to_ascii_uppercase() as u32) ^ 0x40)?
            }
            'a' => '\u{7}',
            'e' => '\u{1b}',
            'f' => '\u{c}',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'x' => {
                if self.peek() == Some('{') {
                    self.pos += 1;
                    let start = self.pos;
                    while self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                        self.pos += 1;
                    }
                    let hex: String = self.chars[start..self.pos].iter().collect();
                    if hex.is_empty() || self.peek() != Some('}') {
                        return invalid("malformed \\x{} in replacement");
                    }
                    self.pos += 1;
                    code_point(u32::from_str_radix(&hex, 16).unwrap_or(u32::MAX))?
                } else {
                    let start = self.pos;
                    while self.pos < start + 2 && self.peek().is_some_and(|c| c.is_ascii_hexdigit())
                    {
                        self.pos += 1;
                    }
                    let hex: String = self.chars[start..self.pos].iter().collect();
                    code_point(u32::from_str_radix(&hex, 16).unwrap_or(0))?
                }
            }
            'o' => {
                if self.peek() != Some('{') {
                    return invalid("missing opening brace after \\o");
                }
                self.pos += 1;
                let start = self.pos;
                while self.peek().is_some_and(|c| c.is_digit(8)) {
                    self.pos += 1;
                }
                let oct: String = self.chars[start..self.pos].iter().collect();
                if oct.is_empty() || self.peek() != Some('}') {
                    return invalid("malformed \\o{} in replacement");
                }
                self.pos += 1;
                code_point(u32::from_str_radix(&oct, 8).unwrap_or(u32::MAX))?
            }
            // `\n` refers to group n when n is below 10 or names a group;
            // otherwise three octal digits are a character (`\101` is "A")
            // and anything else is #VALUE! (`\10` with one group).
            '1'..='9' => {
                self.pos -= 1;
                let start = self.pos;
                let number = self.digits().unwrap_or(0);
                if number < 10 || number <= self.groups {
                    out.push(Piece::Group(self.group(number)?));
                    return Ok(());
                }
                let octal = &self.chars[start..(start + 3).min(self.chars.len())];
                if octal.len() < 3 || !octal.iter().all(|c| c.is_digit(8)) {
                    return invalid("unknown substring");
                }
                self.pos = start + 3;
                let value = octal.iter().fold(0, |v, c| v * 8 + c.to_digit(8).unwrap());
                code_point(value)?
            }
            '0' => return invalid("\\0 in replacement"),
            c if c.is_ascii_alphanumeric() => {
                return invalid("unrecognized escape in replacement");
            }
            c => c,
        };
        out.push(Piece::Text(ch));
        Ok(())
    }
}

fn code_point(value: u32) -> Result<char, PatternError> {
    char::from_u32(value).map_or_else(|| invalid("character value in replacement"), Ok)
}

pub(crate) fn parse(
    replacement: &str,
    groups: usize,
    names: &[(String, usize)],
) -> Result<Vec<Piece>, PatternError> {
    let chars: Vec<char> = replacement.chars().collect();
    Reader {
        chars: &chars,
        pos: 0,
        groups,
        names,
    }
    .pieces(&[])
}

#[derive(Clone, Copy, PartialEq)]
enum Case {
    None,
    Upper,
    Lower,
}

/// A character's simple case mapping, as PCRE2 applies it: one character
/// for one (ß stays ß in upper case).
fn simple_case(c: char, upper: bool) -> char {
    if c == '\u{130}' && !upper {
        return 'i';
    }
    let mut mapped = if upper {
        c.to_uppercase().collect::<Vec<_>>()
    } else {
        c.to_lowercase().collect::<Vec<_>>()
    };
    if mapped.len() == 1 {
        mapped.pop().unwrap()
    } else {
        c
    }
}

struct Writer {
    out: String,
    case: Case,
    next: Case,
}

impl Writer {
    fn push(&mut self, c: char) {
        let case = if self.next != Case::None {
            std::mem::replace(&mut self.next, Case::None)
        } else {
            self.case
        };
        self.out.push(match case {
            Case::None => c,
            Case::Upper => simple_case(c, true),
            Case::Lower => simple_case(c, false),
        });
    }
}

fn write(pieces: &[Piece], text: &[char], slots: &[Option<usize>], writer: &mut Writer) {
    let group_text = |group: usize| match (slots.get(2 * group), slots.get(2 * group + 1)) {
        (Some(Some(s)), Some(Some(e))) if s <= e => Some(&text[*s..*e]),
        _ => None,
    };
    for piece in pieces {
        match piece {
            Piece::Nothing => {}
            Piece::Text(c) => writer.push(*c),
            Piece::Before => {
                for &c in &text[..slots[0].unwrap_or(0).min(text.len())] {
                    writer.push(c);
                }
            }
            Piece::After => {
                for &c in &text[slots[1].unwrap_or(0).min(text.len())..] {
                    writer.push(c);
                }
            }
            Piece::Subject => {
                for &c in text {
                    writer.push(c);
                }
            }
            Piece::Group(group) => {
                for &c in group_text(*group).unwrap_or(&[]) {
                    writer.push(c);
                }
            }
            Piece::IfSet {
                group,
                set,
                unset,
                own_when_set,
            } => match group_text(*group) {
                Some(value) if *own_when_set => {
                    for &c in value {
                        writer.push(c);
                    }
                }
                Some(_) => write(set, text, slots, writer),
                None => write(unset, text, slots, writer),
            },
            Piece::Upper => {
                writer.case = Case::Upper;
                writer.next = Case::None;
            }
            Piece::Lower => {
                writer.case = Case::Lower;
                writer.next = Case::None;
            }
            Piece::EndCase => {
                writer.case = Case::None;
                writer.next = Case::None;
            }
            Piece::UpperNext => writer.next = Case::Upper,
            Piece::LowerNext => writer.next = Case::Lower,
        }
    }
}

/// The replacement for one match.
pub(crate) fn expand(pieces: &[Piece], text: &[char], slots: &[Option<usize>]) -> String {
    let mut writer = Writer {
        out: String::new(),
        case: Case::None,
        next: Case::None,
    };
    write(pieces, text, slots, &mut writer);
    writer.out
}
