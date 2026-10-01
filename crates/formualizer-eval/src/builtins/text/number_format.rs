//! Excel number-format rendering (the formatting language of `TEXT`).
//!
//! Supports up to four `;`-separated sections (positive; negative; zero;
//! text) and `[>=n]` conditions, quoted/escaped literals, `_x` spacing, `*x`
//! fill (dropped), `[$sym-lcid]` currency, `[color]` (ignored), `General`,
//! digit placeholders `0 # ?`, grouping and scaling commas, `%`, scientific
//! `E+`/`E-`, fractions (`# ?/?`, `?/8`), `@`, and the date/time tokens
//! `y e b m d h s AM/PM A/P [h] [m] [s]` with fractional seconds. Code
//! letters match without regard to case or accents, and `\` and `!` show the
//! next character as it is. Output is the Excel for Mac en-US rendering;
//! values keep at most 15 significant digits. Codes Excel cannot read are
//! `#VALUE!`: an unquoted `n` (any case or accent), unterminated quotes or
//! brackets, a dangling `\` `!` `_` `*`, more than four sections, or date
//! codes mixed with digit placeholders, `%` or `@`.

use crate::engine::DateSystem;
use formualizer_common::{ExcelError, try_serial_to_display_date_parts_for};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Lit(String),
    /// `0`, `#` or `?`.
    Digit(char),
    Point,
    Comma,
    Percent,
    Exp {
        plus: bool,
        upper: bool,
    },
    Slash,
    At,
    General,
    Year(usize),
    /// `b`/`bb` (two digits) or `bbb`+ (four): the Buddhist-era year.
    BuddhistYear(usize),
    /// Month or minute, resolved from neighbouring hour/second tokens.
    MonthOrMinute(usize),
    Minute(usize),
    Day(usize),
    Hour(usize),
    Second(usize),
    /// `AM/PM` (`true`) or `A/P`, with the case to print.
    AmPm(bool, bool),
    Elapsed(char, usize),
}

#[derive(Clone, Copy, Debug)]
enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Debug, Default)]
struct Section {
    toks: Vec<Tok>,
    condition: Option<(Cmp, f64)>,
}

fn is_date_tok(t: &Tok) -> bool {
    matches!(
        t,
        Tok::Year(_)
            | Tok::BuddhistYear(_)
            | Tok::MonthOrMinute(_)
            | Tok::Minute(_)
            | Tok::Day(_)
            | Tok::Hour(_)
            | Tok::Second(_)
            | Tok::AmPm(..)
            | Tok::Elapsed(..)
    )
}

impl Section {
    fn is_date(&self) -> bool {
        self.toks.iter().any(is_date_tok)
    }
    fn has(&self, tok: &Tok) -> bool {
        self.toks.contains(tok)
    }
}

fn split_sections(code: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted, mut bracket, mut escaped) = (0, false, false, false);
    for (i, c) in code.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '"' if !bracket => quoted = !quoted,
            // `\x`, `!x`, `_x` and `*x` take the next character literally.
            '\\' | '!' | '_' | '*' if !quoted && !bracket => escaped = true,
            '[' if !quoted => bracket = true,
            ']' if !quoted => bracket = false,
            ';' if !quoted && !bracket => {
                out.push(&code[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&code[start..]);
    out
}

fn parse_condition(text: &str) -> Option<(Cmp, f64)> {
    let (cmp, rest) = if let Some(r) = text.strip_prefix("<=") {
        (Cmp::Le, r)
    } else if let Some(r) = text.strip_prefix(">=") {
        (Cmp::Ge, r)
    } else if let Some(r) = text.strip_prefix("<>") {
        (Cmp::Ne, r)
    } else if let Some(r) = text.strip_prefix('<') {
        (Cmp::Lt, r)
    } else if let Some(r) = text.strip_prefix('>') {
        (Cmp::Gt, r)
    } else if let Some(r) = text.strip_prefix('=') {
        (Cmp::Eq, r)
    } else {
        return None;
    };
    rest.trim().parse().ok().map(|n| (cmp, n))
}

fn push_lit(toks: &mut Vec<Tok>, text: &str) {
    if let Some(Tok::Lit(prev)) = toks.last_mut() {
        prev.push_str(text);
    } else {
        toks.push(Tok::Lit(text.to_owned()));
    }
}

/// The format-code letter `c` stands for. Excel matches code letters without
/// regard to case or accents (Excel for Mac reads `ÅÅÅÅ` as `aaaa` and `É` as
/// `e`), so a Latin letter with a diacritic folds to its base letter.
fn code_letter(c: char) -> char {
    match c {
        'À'..='Å' | 'à'..='å' | 'Ā'..='ą' | 'Ǎ' | 'ǎ' | 'Ȁ'..='ȃ' | 'Ȧ' | 'ȧ' => 'a',
        'È'..='Ë' | 'è'..='ë' | 'Ē'..='ě' | 'Ȅ'..='ȇ' | 'Ȩ' | 'ȩ' => 'e',
        'Ñ' | 'ñ' | 'Ń'..='ň' | 'Ǹ' | 'ǹ' => 'n',
        'Ý' | 'ý' | 'ÿ' | 'Ŷ'..='Ÿ' | 'Ȳ' | 'ȳ' => 'y',
        'Ď' | 'ď' => 'd',
        'Ĥ' | 'ĥ' | 'Ȟ' | 'ȟ' => 'h',
        'Ś'..='š' | 'Ș' | 'ș' => 's',
        _ => c.to_ascii_lowercase(),
    }
}

/// Parse one section, rejecting codes Excel cannot read.
fn parse_section(text: &str) -> Result<Section, ExcelError> {
    let chars: Vec<char> = text.chars().collect();
    let mut section = Section::default();
    let toks = &mut section.toks;
    let mut i = 0;
    let run = |i: usize, lower: char| {
        chars[i..]
            .iter()
            .take_while(|&&c| code_letter(c) == lower)
            .count()
    };
    let closing = |i: usize, close: char| {
        chars[i + 1..]
            .iter()
            .position(|&q| q == close)
            .map(|p| i + 1 + p)
            .ok_or_else(ExcelError::new_value)
    };
    while i < chars.len() {
        let c = chars[i];
        let lower = code_letter(c);
        match c {
            '"' => {
                let end = closing(i, '"')?;
                push_lit(toks, &chars[i + 1..end].iter().collect::<String>());
                i = end + 1;
                continue;
            }
            // `\`, `!`, `_` and `*` need a character to act on.
            '\\' | '!' | '_' | '*' if i + 1 == chars.len() => {
                return Err(ExcelError::new_value());
            }
            // `!` shows the next character as it is, like `\`.
            '\\' | '!' => {
                push_lit(toks, &chars[i + 1].to_string());
                i += 2;
                continue;
            }
            '_' => {
                push_lit(toks, " ");
                i += 2;
                continue;
            }
            '*' => {
                i += 2;
                continue;
            }
            '[' => {
                let end = closing(i, ']')?;
                let inner: String = chars[i + 1..end].iter().collect();
                let lower_inner = inner.to_ascii_lowercase();
                if let Some(currency) = inner.strip_prefix('$') {
                    let symbol = currency.split('-').next().unwrap_or("");
                    push_lit(toks, symbol);
                } else if let Some(condition) = parse_condition(&inner) {
                    section.condition = Some(condition);
                } else if !lower_inner.is_empty()
                    && lower_inner
                        .chars()
                        .all(|x| x == lower_inner.as_bytes()[0] as char)
                    && matches!(lower_inner.as_bytes()[0], b'h' | b'm' | b's')
                {
                    toks.push(Tok::Elapsed(
                        lower_inner.as_bytes()[0] as char,
                        lower_inner.len(),
                    ));
                }
                i = end + 1;
                continue;
            }
            '0' | '#' | '?' => toks.push(Tok::Digit(c)),
            '.' => toks.push(Tok::Point),
            ',' => toks.push(Tok::Comma),
            '%' => toks.push(Tok::Percent),
            '/' => toks.push(Tok::Slash),
            '@' => toks.push(Tok::At),
            _ if lower == 'e' && matches!(chars.get(i + 1), Some('+' | '-')) => {
                toks.push(Tok::Exp {
                    plus: chars[i + 1] == '+',
                    upper: c.is_uppercase(),
                });
                i += 2;
                continue;
            }
            _ if chars[i..]
                .iter()
                .take(7)
                .collect::<String>()
                .eq_ignore_ascii_case("general") =>
            {
                toks.push(Tok::General);
                i += 7;
                continue;
            }
            _ if chars[i..]
                .iter()
                .take(5)
                .collect::<String>()
                .eq_ignore_ascii_case("am/pm") =>
            {
                toks.push(Tok::AmPm(true, c.is_ascii_lowercase()));
                i += 5;
                continue;
            }
            _ if chars[i..]
                .iter()
                .take(3)
                .collect::<String>()
                .eq_ignore_ascii_case("a/p") =>
            {
                toks.push(Tok::AmPm(false, c.is_ascii_lowercase()));
                i += 3;
                continue;
            }
            // `n` is no code letter, and Excel will not show it unquoted.
            _ if lower == 'n' => return Err(ExcelError::new_value()),
            // `b` is the Buddhist-era year; `B1`/`B2` (calendar prefixes) stay
            // as written.
            _ if lower == 'b' && !matches!(chars.get(i + 1), Some('1' | '2')) => {
                let n = run(i, 'b');
                toks.push(Tok::BuddhistYear(n));
                i += n;
                continue;
            }
            _ if matches!(lower, 'y' | 'e') => {
                let n = run(i, lower);
                toks.push(Tok::Year(if lower == 'e' { 4 } else { n }));
                i += n;
                continue;
            }
            _ if matches!(lower, 'm' | 'd' | 'h' | 's') => {
                let n = run(i, lower);
                toks.push(match lower {
                    'm' => Tok::MonthOrMinute(n),
                    'd' => Tok::Day(n),
                    'h' => Tok::Hour(n),
                    _ => Tok::Second(n),
                });
                i += n;
                continue;
            }
            _ => push_lit(toks, &c.to_string()),
        }
        i += 1;
    }
    if (section.is_date() || section.has(&Tok::General)) && mixes_numbers(&section.toks) {
        return Err(ExcelError::new_value());
    }
    resolve_minutes(&mut section.toks);
    Ok(section)
}

/// Whether a section has digit placeholders, `%` or `@`, apart from the up to
/// three `0`s of fractional seconds after a date or time code's `.`.
fn mixes_numbers(toks: &[Tok]) -> bool {
    let mut dated = false;
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            Tok::Point if dated => {
                i += toks[i + 1..]
                    .iter()
                    .take(3)
                    .take_while(|t| **t == Tok::Digit('0'))
                    .count();
            }
            Tok::Digit(_) | Tok::Percent | Tok::At => return true,
            t => dated |= is_date_tok(t),
        }
        i += 1;
    }
    false
}

/// Parse every section of a format code. Excel rejects a code with more than
/// four sections or with any section it cannot read.
fn parse_sections(code: &str) -> Result<Vec<Section>, ExcelError> {
    let parts = split_sections(code);
    if parts.len() > 4 {
        return Err(ExcelError::new_value());
    }
    parts.into_iter().map(parse_section).collect()
}

/// `m`/`mm` directly after an hour or before a second is a minute.
fn resolve_minutes(toks: &mut [Tok]) {
    let is_time = |t: &Tok| matches!(t, Tok::Hour(_) | Tok::Elapsed('h', _));
    let is_second = |t: &Tok| matches!(t, Tok::Second(_) | Tok::Elapsed('s', _));
    let dated: Vec<usize> = (0..toks.len())
        .filter(|&i| !matches!(toks[i], Tok::Lit(_) | Tok::Point | Tok::Digit(_)))
        .collect();
    for (k, &i) in dated.iter().enumerate() {
        if let Tok::MonthOrMinute(n) = toks[i]
            && n <= 2
        {
            let after_hour = k > 0 && is_time(&toks[dated[k - 1]]);
            let before_second = dated.get(k + 1).is_some_and(|&j| is_second(&toks[j]));
            if after_hour || before_second {
                toks[i] = Tok::Minute(n);
            }
        }
    }
}

/// Render a number with an Excel format code.
pub(crate) fn format_number(
    value: f64,
    code: &str,
    system: DateSystem,
) -> Result<String, ExcelError> {
    if !value.is_finite() {
        return Err(ExcelError::new_value());
    }
    let sections = parse_sections(code)?;
    // A fourth section formats text only.
    let numeric = &sections[..sections.len().min(3)];
    let conditional = numeric.iter().any(|s| s.condition.is_some());
    let (section, magnitude, signed) = if conditional {
        let holds = |s: &Section| match s.condition {
            Some((cmp, n)) => match cmp {
                Cmp::Lt => value < n,
                Cmp::Le => value <= n,
                Cmp::Gt => value > n,
                Cmp::Ge => value >= n,
                Cmp::Eq => value == n,
                Cmp::Ne => value != n,
            },
            None => true,
        };
        // Sections are tried in order; an unconditioned later section takes
        // every value the conditioned ones did not. Values keep their sign.
        let chosen = numeric
            .iter()
            .find(|s| holds(s))
            .unwrap_or(&numeric[numeric.len() - 1]);
        (chosen, value, true)
    } else {
        match numeric.len() {
            1 => (&numeric[0], value, true),
            2 if value < 0.0 => (&numeric[1], value.abs(), false),
            2 => (&numeric[0], value, true),
            _ if value < 0.0 => (&numeric[1], value.abs(), false),
            _ if value == 0.0 => (&numeric[2], value, false),
            _ => (&numeric[0], value, true),
        }
    };
    if section.toks.is_empty() {
        return Ok(String::new());
    }
    if section.is_date() {
        return format_date(section, magnitude, system);
    }
    let negative = signed && magnitude < 0.0;
    let body = format_numeric(section, magnitude.abs())?;
    Ok(if negative { format!("-{body}") } else { body })
}

/// Render text with the text section (the fourth, or one containing `@`).
/// Without one, Excel returns the text unchanged.
pub(crate) fn format_text(text: &str, code: &str) -> Result<String, ExcelError> {
    let sections = parse_sections(code)?;
    let section = if sections.len() >= 4 {
        Some(&sections[3])
    } else {
        sections.iter().find(|s| s.has(&Tok::At))
    };
    let Some(section) = section else {
        return Ok(text.to_owned());
    };
    let mut out = String::new();
    for tok in &section.toks {
        match tok {
            Tok::At => out.push_str(text),
            Tok::Lit(s) => out.push_str(s),
            _ => {}
        }
    }
    Ok(out)
}

/// Decimal digits of `value` (non-negative) with at most 15 significant
/// digits, rounded half away from zero to `decimals` places: (integer digits
/// without leading zeros, exactly `decimals` fraction digits).
fn round_digits(value: f64, decimals: usize) -> (String, String) {
    if value == 0.0 {
        return (String::new(), "0".repeat(decimals));
    }
    let sci = format!("{value:.14e}");
    let (mantissa, exponent) = sci.split_once('e').expect("scientific");
    let exponent: i64 = exponent.parse().expect("exponent");
    let digits: Vec<u8> = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    // Fixed-point digit vector: `int_len` integer digits followed by fraction.
    let int_len = (exponent + 1).max(0) as usize;
    let mut fixed: Vec<u8> = Vec::new();
    if exponent < 0 {
        fixed.extend(std::iter::repeat_n(0, (-exponent - 1) as usize));
        fixed.extend(&digits);
    } else {
        fixed.extend(&digits);
        if fixed.len() < int_len {
            fixed.resize(int_len, 0);
        }
    }
    let int_len_in_fixed = if exponent < 0 { 0 } else { int_len };
    let keep = int_len_in_fixed + decimals;
    let round_up = fixed.get(keep).is_some_and(|&d| d >= 5);
    fixed.resize(keep, 0);
    if round_up {
        let mut i = keep;
        loop {
            if i == 0 {
                fixed.insert(0, 1);
                break;
            }
            i -= 1;
            if fixed[i] == 9 {
                fixed[i] = 0;
            } else {
                fixed[i] += 1;
                break;
            }
        }
    }
    let split = fixed.len() - decimals;
    let int: String = fixed[..split]
        .iter()
        .map(|d| (b'0' + d) as char)
        .collect::<String>()
        .trim_start_matches('0')
        .to_owned();
    let frac: String = fixed[split..].iter().map(|d| (b'0' + d) as char).collect();
    (int, frac)
}

/// Excel's General format: up to 11 characters, scientific for very large or
/// small magnitudes.
pub(crate) fn format_general(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    let sign = if value < 0.0 { "-" } else { "" };
    let a = value.abs();
    if !(1e-9..1e11).contains(&a) {
        let exponent = a.log10().floor() as i32;
        let mut mantissa = a / 10f64.powi(exponent);
        let mut exponent = exponent;
        let (mut int, mut frac) = round_digits(mantissa, 5);
        if int.len() > 1 {
            exponent += 1;
            mantissa /= 10.0;
            (int, frac) = round_digits(mantissa, 5);
        }
        let frac = frac.trim_end_matches('0');
        let mantissa = if frac.is_empty() {
            int
        } else {
            format!("{int}.{frac}")
        };
        let esign = if exponent < 0 { '-' } else { '+' };
        return format!("{sign}{mantissa}E{esign}{:02}", exponent.abs());
    }
    let int_digits = if a < 1.0 {
        1
    } else {
        a.log10().floor() as usize + 1
    };
    let decimals = 10usize.saturating_sub(int_digits);
    let (int, frac) = round_digits(a, decimals);
    let int = if int.is_empty() { "0".to_owned() } else { int };
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() {
        format!("{sign}{int}")
    } else {
        format!("{sign}{int}.{frac}")
    }
}

fn group(digits: &str) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn format_numeric(section: &Section, value: f64) -> Result<String, ExcelError> {
    let toks = &section.toks;
    if toks.iter().any(|t| matches!(t, Tok::Exp { .. })) {
        return Ok(format_scientific(toks, value));
    }
    if let Some(slash) = fraction_slash(toks) {
        return Ok(format_fraction(toks, slash, value));
    }
    let point = toks.iter().position(|t| *t == Tok::Point);
    let (int_toks, frac_toks) = match point {
        Some(p) => (&toks[..p], &toks[p + 1..]),
        None => (&toks[..], &toks[..0]),
    };
    let digit_at = |ts: &[Tok], i: usize| matches!(ts.get(i), Some(Tok::Digit(_)));
    let last_int_digit = int_toks.iter().rposition(|t| matches!(t, Tok::Digit(_)));
    let first_int_digit = int_toks.iter().position(|t| matches!(t, Tok::Digit(_)));
    let mut grouping = false;
    let mut scale = 0i32;
    let mut commas = vec![false; int_toks.len()]; // literal commas
    for (i, t) in int_toks.iter().enumerate() {
        if *t != Tok::Comma {
            continue;
        }
        match (first_int_digit, last_int_digit) {
            (Some(first), Some(last)) if i > first && i < last => grouping = true,
            (Some(_), Some(last))
                if i > last && int_toks[last + 1..=i].iter().all(|t| *t == Tok::Comma) =>
            {
                scale += 1
            }
            _ => commas[i] = true,
        }
    }
    let last_frac_digit = frac_toks.iter().rposition(|t| matches!(t, Tok::Digit(_)));
    let mut frac_commas = vec![false; frac_toks.len()];
    for (i, t) in frac_toks.iter().enumerate() {
        if *t == Tok::Comma {
            match last_frac_digit {
                Some(last)
                    if i > last && frac_toks[last + 1..=i].iter().all(|t| *t == Tok::Comma) =>
                {
                    scale += 1
                }
                _ => frac_commas[i] = true,
            }
        }
    }
    let percents = toks.iter().filter(|t| **t == Tok::Percent).count() as i32;
    let scaled = value * 100f64.powi(percents) / 1000f64.powi(scale);
    let decimals = frac_toks
        .iter()
        .filter(|t| matches!(t, Tok::Digit(_)))
        .count();
    let (int_digits, frac_digits) = round_digits(scaled, decimals);

    // Integer part: placeholders take digits from the right; the leftmost
    // placeholder also takes any remaining leading digits.
    let placeholders: Vec<usize> = (0..int_toks.len())
        .filter(|&i| digit_at(int_toks, i))
        .collect();
    let digits: Vec<char> = int_digits.chars().collect();
    let mut assigned: Vec<String> = vec![String::new(); int_toks.len()];
    for (j, &pos) in placeholders.iter().rev().enumerate() {
        let kind = match int_toks[pos] {
            Tok::Digit(k) => k,
            _ => unreachable!(),
        };
        let mut text = if j < digits.len() {
            digits[digits.len() - 1 - j].to_string()
        } else {
            match kind {
                '0' => "0".into(),
                '?' => " ".into(),
                _ => String::new(),
            }
        };
        if j + 1 == placeholders.len() && digits.len() > placeholders.len() {
            let lead: String = digits[..digits.len() - placeholders.len()].iter().collect();
            text = lead + &text;
        }
        assigned[pos] = text;
    }
    let mut out = String::new();
    let interleaved = match (first_int_digit, last_int_digit) {
        (Some(f), Some(l)) => int_toks[f..=l]
            .iter()
            .any(|t| !matches!(t, Tok::Digit(_) | Tok::Comma)),
        _ => false,
    };
    let mut int_emitted = false;
    for (i, t) in int_toks.iter().enumerate() {
        match t {
            Tok::Digit(_) if grouping && !interleaved => {
                if !int_emitted {
                    let run: String = placeholders.iter().map(|&p| assigned[p].as_str()).collect();
                    let trimmed = run.trim_start();
                    let pad = run.len() - trimmed.len();
                    out.push_str(&" ".repeat(pad));
                    out.push_str(&group(trimmed));
                    int_emitted = true;
                }
            }
            Tok::Digit(_) => out.push_str(&assigned[i]),
            Tok::Lit(s) => out.push_str(s),
            Tok::Percent => out.push('%'),
            Tok::Comma if commas[i] => out.push(','),
            Tok::Slash => out.push('/'),
            Tok::General => out.push_str(&format_general(scaled)),
            Tok::At => out.push_str(&format_general(scaled)),
            _ => {}
        }
    }
    if placeholders.is_empty() && point.is_some() {
        out.push_str(&int_digits);
    }
    if point.is_some() {
        out.push('.');
        // Trailing zeros: `#` drops them and `?` turns them into spaces.
        let kinds: Vec<char> = frac_toks
            .iter()
            .filter_map(|t| match t {
                Tok::Digit(k) => Some(*k),
                _ => None,
            })
            .collect();
        let fd: Vec<char> = frac_digits.chars().collect();
        let mut shown: Vec<String> = fd.iter().map(char::to_string).collect();
        for k in (0..kinds.len()).rev() {
            if fd[k] != '0' || kinds[k] == '0' {
                break;
            }
            shown[k] = if kinds[k] == '?' {
                " ".into()
            } else {
                String::new()
            };
        }
        let mut next = 0;
        for (i, t) in frac_toks.iter().enumerate() {
            match t {
                Tok::Digit(_) => {
                    out.push_str(&shown[next]);
                    next += 1;
                }
                Tok::Lit(s) => out.push_str(s),
                Tok::Percent => out.push('%'),
                Tok::Point => out.push('.'),
                Tok::Comma if frac_commas[i] => out.push(','),
                _ => {}
            }
        }
    }
    Ok(out)
}

/// The slash of a fraction format: a `/` preceded by a digit placeholder.
fn fraction_slash(toks: &[Tok]) -> Option<usize> {
    let slash = toks.iter().position(|t| *t == Tok::Slash)?;
    let before = toks[..slash].iter().any(|t| matches!(t, Tok::Digit(_)));
    let after = matches!(toks.get(slash + 1), Some(Tok::Digit(_)))
        || matches!(toks.get(slash + 1), Some(Tok::Lit(s)) if s.starts_with(|c: char| c.is_ascii_digit()));
    (before && after).then_some(slash)
}

fn format_fraction(toks: &[Tok], slash: usize, value: f64) -> String {
    // Numerator placeholders: the digit run ending right before the slash.
    let mut num_start = slash;
    while num_start > 0 && matches!(toks[num_start - 1], Tok::Digit(_)) {
        num_start -= 1;
    }
    let num_kinds: Vec<char> = toks[num_start..slash]
        .iter()
        .filter_map(|t| match t {
            Tok::Digit(k) => Some(*k),
            _ => None,
        })
        .collect();
    let int_toks = &toks[..num_start];
    let has_int = int_toks.iter().any(|t| matches!(t, Tok::Digit(_)));
    // Denominator: placeholders, or a fixed number made of digits.
    let mut den_end = slash + 1;
    let mut fixed = String::new();
    let mut den_kinds = Vec::new();
    while den_end < toks.len() {
        match &toks[den_end] {
            Tok::Digit(k) if fixed.is_empty() => den_kinds.push(*k),
            Tok::Digit(_) => fixed.push('0'),
            Tok::Lit(s) if den_kinds.is_empty() && s.starts_with(|c: char| c.is_ascii_digit()) => {
                let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
                fixed.push_str(&digits);
                if digits.len() < s.len() {
                    break;
                }
            }
            _ => break,
        }
        den_end += 1;
    }
    let (whole, frac) = if has_int {
        (value.trunc(), value.fract())
    } else {
        (0.0, value)
    };
    let (mut num, mut den) = if let Ok(d) = fixed.parse::<u64>() {
        ((frac * d as f64).round() as u64, d.max(1))
    } else {
        let max_den = 10u64.pow(den_kinds.len().max(1) as u32) - 1;
        best_fraction(frac, max_den)
    };
    let mut whole = whole as u64;
    if has_int && num == den && num != 0 {
        whole += 1;
        num = 0;
        den = if fixed.is_empty() { 1 } else { den };
    }
    let mut out = String::new();
    // Integer part.
    let int_text = if has_int && whole == 0 && num != 0 {
        String::new()
    } else if has_int {
        whole.to_string()
    } else {
        String::new()
    };
    let mut int_done = false;
    for t in int_toks {
        match t {
            Tok::Digit(k) => {
                if !int_done {
                    if int_text.is_empty() && *k == '0' {
                        out.push('0');
                    } else if int_text.is_empty() && *k == '?' {
                        out.push(' ');
                    }
                    out.push_str(&int_text);
                    int_done = true;
                }
            }
            Tok::Lit(s) => out.push_str(s),
            _ => {}
        }
    }
    let blank = has_int && num == 0;
    let pad = |text: String, kinds: &[char], left: bool| {
        let width = kinds.len();
        if text.len() >= width {
            return text;
        }
        let fill = if kinds.first() == Some(&'0') {
            '0'
        } else if kinds.contains(&'?') {
            ' '
        } else {
            '\0'
        };
        if fill == '\0' {
            return text;
        }
        let padding: String = std::iter::repeat_n(fill, width - text.len()).collect();
        if left {
            padding + &text
        } else {
            text + &padding
        }
    };
    if blank {
        let width = num_kinds.len() + 1 + den_kinds.len().max(fixed.len());
        out.push_str(&" ".repeat(width));
    } else {
        out.push_str(&pad(num.to_string(), &num_kinds, true));
        out.push('/');
        if fixed.is_empty() {
            out.push_str(&pad(den.to_string(), &den_kinds, false));
        } else {
            out.push_str(&fixed);
        }
    }
    for t in &toks[den_end.min(toks.len())..] {
        if let Tok::Lit(s) = t {
            out.push_str(s);
        }
    }
    out
}

/// Closest fraction with denominator at most `max_den`.
fn best_fraction(value: f64, max_den: u64) -> (u64, u64) {
    let mut best = (value.round() as u64, 1u64);
    let mut best_err = (value - best.0 as f64).abs();
    for den in 1..=max_den {
        let num = (value * den as f64).round();
        let err = (value - num / den as f64).abs();
        if err < best_err - 1e-12 {
            best = (num as u64, den);
            best_err = err;
        }
    }
    best
}

fn format_scientific(toks: &[Tok], value: f64) -> String {
    let e = toks
        .iter()
        .position(|t| matches!(t, Tok::Exp { .. }))
        .expect("exponent");
    let Tok::Exp { plus, upper } = toks[e] else {
        unreachable!()
    };
    let mantissa_toks = &toks[..e];
    let point = mantissa_toks.iter().position(|t| *t == Tok::Point);
    let (int_toks, frac_toks) = match point {
        Some(p) => (&mantissa_toks[..p], &mantissa_toks[p + 1..]),
        None => (mantissa_toks, &mantissa_toks[..0]),
    };
    let int_kinds: Vec<char> = int_toks
        .iter()
        .filter_map(|t| match t {
            Tok::Digit(k) => Some(*k),
            _ => None,
        })
        .collect();
    let decimals = frac_toks
        .iter()
        .filter(|t| matches!(t, Tok::Digit(_)))
        .count();
    let exp_digits = toks[e + 1..]
        .iter()
        .filter(|t| matches!(t, Tok::Digit(_)))
        .count();
    let n_int = int_kinds.len().max(1) as i32;
    let engineering = int_kinds.contains(&'#');
    let step = |x: i32| {
        if engineering {
            x.div_euclid(n_int) * n_int
        } else {
            x - (n_int - 1)
        }
    };
    let mut exponent = if value == 0.0 {
        0
    } else {
        step(value.log10().floor() as i32)
    };
    let (mut int, mut frac) = round_digits(value / 10f64.powi(exponent), decimals);
    if value != 0.0 && int.len() as i32 > if engineering { n_int } else { n_int.max(1) } {
        exponent += if engineering { n_int } else { 1 };
        (int, frac) = round_digits(value / 10f64.powi(exponent), decimals);
    }
    let mut out = String::new();
    let int_width = int_kinds.len();
    let int_text = if int.len() >= int_width {
        int
    } else {
        let missing = int_width - int.len();
        let fill: String = int_kinds[..missing]
            .iter()
            .filter_map(|k| match k {
                '0' => Some('0'),
                '?' => Some(' '),
                _ => None,
            })
            .collect();
        fill + &int
    };
    for t in int_toks {
        match t {
            Tok::Lit(s) => out.push_str(s),
            Tok::Digit(_) => {}
            _ => {}
        }
    }
    out.push_str(&int_text);
    if point.is_some() {
        out.push('.');
        out.push_str(&frac);
    }
    out.push(if upper { 'E' } else { 'e' });
    if exponent < 0 {
        out.push('-');
    } else if plus {
        out.push('+');
    }
    out.push_str(&format!("{:0width$}", exponent.abs(), width = exp_digits));
    for t in &toks[e + 1..] {
        if let Tok::Lit(s) = t {
            out.push_str(s);
        }
    }
    out
}

fn format_date(section: &Section, value: f64, system: DateSystem) -> Result<String, ExcelError> {
    let toks = &section.toks;
    let elapsed = toks.iter().any(|t| matches!(t, Tok::Elapsed(..)));
    if value < 0.0 && !elapsed {
        return Err(ExcelError::new_value());
    }
    let negative = value < 0.0;
    let value = value.abs();
    // Fractional-second digits shown after `s`/`ss`/`[s]` as `.0`, `.00`, ...
    let mut sub_digits = 0usize;
    for (i, t) in toks.iter().enumerate() {
        if matches!(t, Tok::Second(_) | Tok::Elapsed('s', _))
            && toks.get(i + 1) == Some(&Tok::Point)
        {
            sub_digits = toks[i + 2..]
                .iter()
                .take_while(|t| **t == Tok::Digit('0'))
                .count();
        }
    }
    let unit = 10f64.powi(sub_digits as i32);
    // Round to the displayed precision of seconds, then split fields.
    let total = (value * 86_400.0 * unit).round() / unit;
    let days = (total / 86_400.0).floor();
    let secs = total - days * 86_400.0;
    let whole_secs = secs.floor() as i64;
    let fraction = secs - whole_secs as f64;
    let (hour, minute, second) = (whole_secs / 3600, (whole_secs / 60) % 60, whole_secs % 60);
    let needs_date = toks.iter().any(|t| {
        matches!(
            t,
            Tok::Year(_) | Tok::BuddhistYear(_) | Tok::MonthOrMinute(_) | Tok::Day(_)
        )
    });
    let parts = if needs_date {
        // Serials outside Excel's calendar cannot be shown as dates.
        Some(
            try_serial_to_display_date_parts_for(system, days)
                .map_err(|_| ExcelError::new_value())?,
        )
    } else {
        None
    };
    let twelve_hour = toks.iter().any(|t| matches!(t, Tok::AmPm(..)));
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            Tok::Lit(s) => out.push_str(s),
            Tok::Year(n) | Tok::BuddhistYear(n) => {
                let year = parts.as_ref().map_or(1900, |p| p.year);
                // The Buddhist era starts 543 years before the common era.
                let year = if matches!(toks[i], Tok::BuddhistYear(_)) {
                    year + 543
                } else {
                    year
                };
                if *n <= 2 {
                    out.push_str(&format!("{:02}", year.rem_euclid(100)));
                } else {
                    out.push_str(&format!("{year:04}"));
                }
            }
            Tok::MonthOrMinute(n) => {
                let month = parts.as_ref().map_or(1, |p| p.month) as usize;
                match n {
                    1 => out.push_str(&month.to_string()),
                    2 => out.push_str(&format!("{month:02}")),
                    3 => out.push_str(&MONTHS[month - 1][..3]),
                    5 => out.push_str(&MONTHS[month - 1][..1]),
                    _ => out.push_str(MONTHS[month - 1]),
                }
            }
            Tok::Day(n) => {
                let day = parts.as_ref().map_or(0, |p| p.day);
                match n {
                    1 => out.push_str(&day.to_string()),
                    2 => out.push_str(&format!("{day:02}")),
                    _ => {
                        let offset = if system == DateSystem::Excel1904 {
                            5.0
                        } else {
                            6.0
                        };
                        let weekday = ((days + offset) % 7.0) as usize;
                        let name = DAYS[weekday];
                        out.push_str(if *n == 3 { &name[..3] } else { name });
                    }
                }
            }
            Tok::Hour(n) => {
                let h = if twelve_hour {
                    match hour % 12 {
                        0 => 12,
                        h => h,
                    }
                } else {
                    hour
                };
                out.push_str(&if *n >= 2 {
                    format!("{h:02}")
                } else {
                    h.to_string()
                });
            }
            Tok::Minute(n) => out.push_str(&if *n >= 2 {
                format!("{minute:02}")
            } else {
                minute.to_string()
            }),
            Tok::Second(n) => out.push_str(&if *n >= 2 {
                format!("{second:02}")
            } else {
                second.to_string()
            }),
            Tok::Elapsed(unit_char, n) => {
                let amount = match unit_char {
                    'h' => (days as i64) * 24 + hour,
                    'm' => ((days as i64) * 24 + hour) * 60 + minute,
                    _ => (((days as i64) * 24 + hour) * 60 + minute) * 60 + second,
                };
                out.push_str(&format!("{amount:0width$}", width = *n));
            }
            Tok::AmPm(full, lower) => {
                let pm = hour >= 12;
                let text = match (full, pm) {
                    (true, false) => "AM",
                    (true, true) => "PM",
                    (false, false) => "A",
                    (false, true) => "P",
                };
                out.push_str(&if *lower {
                    text.to_ascii_lowercase()
                } else {
                    text.to_owned()
                });
            }
            Tok::Point
                if sub_digits > 0
                    && i > 0
                    && matches!(toks[i - 1], Tok::Second(_) | Tok::Elapsed('s', _)) =>
            {
                let digits = format!("{:.*}", sub_digits, fraction);
                out.push_str(digits.trim_start_matches('0'));
                i += 1 + sub_digits;
                continue;
            }
            Tok::Point => out.push('.'),
            Tok::Digit(d) => out.push(*d),
            Tok::Comma => out.push(','),
            Tok::Percent => out.push('%'),
            Tok::Slash => out.push('/'),
            Tok::At | Tok::General | Tok::Exp { .. } => {}
        }
        i += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(value: f64, code: &str) -> String {
        format_number(value, code, DateSystem::Excel1900).unwrap()
    }

    #[test]
    fn digit_placeholders_grouping_scaling_and_percent() {
        assert_eq!(fmt(1234.0, "0000000"), "0001234");
        assert_eq!(fmt(10.0, "000"), "010");
        assert_eq!(fmt(1234.567, "#,##0.00"), "1,234.57");
        assert_eq!(fmt(1234.567, "$#,##0.00"), "$1,234.57");
        assert_eq!(fmt(-1234.567, "$#,##0.00"), "-$1,234.57");
        assert_eq!(fmt(0.285, "0.0%"), "28.5%");
        assert_eq!(fmt(0.256, "0%"), "26%");
        assert_eq!(fmt(1234567.0, "#,##0,"), "1,235");
        assert_eq!(fmt(12345678.0, "0.0,,"), "12.3");
        assert_eq!(fmt(13.0, " ###,###"), " 13");
        assert_eq!(fmt(0.25, "#,#"), "");
        assert_eq!(fmt(5.0, "#.##"), "5.");
        assert_eq!(fmt(5.5, "0.0#"), "5.5");
        assert_eq!(fmt(1.005, "0.00"), "1.01");
        assert_eq!(fmt(2.5, "0"), "3");
        assert_eq!(fmt(123456789.0, "000-00-0000"), "123-45-6789");
        assert_eq!(fmt(0.5, ".00"), ".50");
    }

    #[test]
    fn sections_conditions_and_text() {
        assert_eq!(fmt(-5.0, "0.00;(0.00)"), "(5.00)");
        assert_eq!(fmt(0.0, "0;-0;\"zero\""), "zero");
        assert_eq!(fmt(-3.0, "0.00;;0"), "");
        assert_eq!(fmt(450.0, "0.00;;0;\\0"), "450.00");
        assert_eq!(fmt(5.0, "[<10]\"small\";\"big\""), "small");
        assert_eq!(fmt(50.0, "[<10]\"small\";\"big\""), "big");
        assert_eq!(format_text("abc", "0.00").unwrap(), "abc");
        assert_eq!(format_text("abc", "0;0;0;\"<\"@\">\"").unwrap(), "<abc>");
        assert_eq!(format_text("abc", "\"x\"@").unwrap(), "xabc");
    }

    #[test]
    fn scientific_fraction_and_general() {
        assert_eq!(fmt(12200000.0, "0.00E+00"), "1.22E+07");
        assert_eq!(fmt(0.00012, "0.0E+0"), "1.2E-4");
        assert_eq!(fmt(12200000.0, "##0.0E+0"), "12.2E+6");
        assert_eq!(fmt(1.5, "# ?/?"), "1 1/2");
        assert_eq!(fmt(0.75, "?/?"), "3/4");
        assert_eq!(fmt(0.3, "# ??/??"), "  3/10");
        assert_eq!(fmt(2.625, "# ?/8"), "2 5/8");
        assert_eq!(fmt(1.0 / 3.0, "General"), "0.333333333");
        assert_eq!(fmt(1234.5, "General"), "1234.5");
        assert_eq!(fmt(123456789012.0, "General"), "1.23457E+11");
        assert_eq!(fmt(-2.0, "General"), "-2");
    }

    #[test]
    fn dates_and_times() {
        // 2024-03-05 is a Tuesday; 45356.5 is noon.
        assert_eq!(fmt(45356.0, "dddd"), "Tuesday");
        assert_eq!(fmt(45356.0, "ddd"), "Tue");
        assert_eq!(fmt(45356.0, "DDD"), "Tue");
        assert_eq!(fmt(45356.0, "mmm"), "Mar");
        assert_eq!(fmt(45356.0, "mmmm yyyy"), "March 2024");
        assert_eq!(fmt(45356.0, "MMM YY"), "Mar 24");
        assert_eq!(fmt(45356.0, "dd-mmm"), "05-Mar");
        assert_eq!(fmt(45356.0, "d/m/yyyy"), "5/3/2024");
        assert_eq!(fmt(45356.0, "yyyy-mm-dd"), "2024-03-05");
        assert_eq!(fmt(45356.0, "mmmmm"), "M");
        assert_eq!(fmt(45356.5, "h:mm AM/PM"), "12:00 PM");
        assert_eq!(fmt(45356.25, "hh:mm:ss"), "06:00:00");
        assert_eq!(fmt(0.025763888888888888, "h:mm:ss"), "0:37:06");
        assert_eq!(fmt(1.5, "[h]:mm"), "36:00");
        assert_eq!(
            fmt(0.5 + 1.0 / 86_400.0 * 1.25, "hh:mm:ss.00"),
            "12:00:01.25"
        );
        assert_eq!(fmt(45356.75, "m/d/yy h:mm"), "3/5/24 18:00");
        assert_eq!(fmt(1.0, "dddd"), "Sunday");
        assert!(format_number(-1.0, "yyyy", DateSystem::Excel1900).is_err());
    }

    #[test]
    fn codes_excel_cannot_read_are_rejected() {
        for code in [
            // `n` is not a format code, in any case or with any accent.
            "eeee.hh.nn ",
            "nn",
            "0 N",
            "\u{f1}",
            "0 \u{d1}",
            "\u{144}\u{148}",
            // Unterminated quotes and brackets, dangling `\` `!` `_` `*`.
            "\"abc",
            "[Red",
            "0\\",
            "0!",
            "0_",
            "0*",
            // At most four sections.
            "0;0;0;@;0",
            "0;0;0;@;",
            // Date and time codes mixed with placeholders, `%` or `@`.
            "yyyy 0",
            "mm%",
            "mm@",
            "d .#",
            "h #",
            "[h] 0",
            "AM/PM ?",
            "ss.0000",
            "General 0",
            // `e`/`E` without a sign and `b`/`B` are year codes, so they cannot
            // stand beside digit placeholders either, in any section.
            "0E0",
            "0.0\u{c9}",
            "0.0B",
            "0.0b",
            "0.0B ",
            "0.0B;0",
            "0;0.0B",
        ] {
            assert!(
                format_number(1.0, code, DateSystem::Excel1900).is_err(),
                "{code:?}"
            );
            assert!(format_text("abc", code).is_err(), "{code:?}");
        }
    }

    #[test]
    fn literal_letters_and_protected_codes_still_format() {
        assert_eq!(fmt(0.400544, "\u{f3}\u{f3}:pp:mm"), "\u{f3}\u{f3}:pp:01");
        assert_eq!(fmt(1.5, "0.0x"), "1.5x");
        assert_eq!(fmt(1.0, "acfijklopqrtuvwxz"), "acfijklopqrtuvwxz");
        assert_eq!(fmt(1.0, "CFIJKLOPQRTUVWXZ"), "CFIJKLOPQRTUVWXZ");
        assert_eq!(fmt(5.0, "0 \\n"), "5 n");
        assert_eq!(fmt(5.0, "0 \"min\""), "5 min");
        assert_eq!(fmt(5.0, "0_N"), "5 ");
        assert_eq!(fmt(5.0, "0*E"), "5");
        assert_eq!(
            fmt(1234.5, "_-* #,##0.00_-;-* #,##0.00_-;_-* \"-\"??_-;_-@_-"),
            " 1,234.50 "
        );
        assert_eq!(fmt(5.0, "[Green]0;[Red]-0"), "5");
        assert_eq!(fmt(5.0, "[$-409]0"), "5");
        assert_eq!(fmt(5.0, "0;0;0;@"), "5");
        assert_eq!(fmt(1234.5, "GENERAL"), "1234.5");
        assert_eq!(fmt(45356.0, "mmmm d, yyyy"), "March 5, 2024");
        assert_eq!(fmt(45356.5, "hh:mm:ss.000"), "12:00:00.000");
        assert_eq!(fmt(45356.0, "eeee"), "2024");
        assert_eq!(format_text("abc", "\"n\"@").unwrap(), "nabc");
    }

    #[test]
    fn code_letters_match_without_case_or_accents() {
        // 2024-03-05. `E`/`e` without a sign is the year, whatever its case.
        assert_eq!(fmt(45356.0, "EEEE"), "2024");
        assert_eq!(fmt(45356.0, "E"), "2024");
        assert_eq!(fmt(45356.0, "dd/mm/EE"), "05/03/2024");
        assert_eq!(fmt(45356.0, "eeee"), "2024");
        // Accented letters fold to their base letter: E-like ones are the
        // year, and so on for the other date letters.
        assert_eq!(fmt(45356.0, "\u{e9}\u{e9}\u{e9}\u{e9}"), "2024");
        assert_eq!(fmt(45356.0, "\u{c9}"), "2024");
        assert_eq!(fmt(45356.0, "\u{e8}\u{ea}\u{cb}\u{113}"), "2024");
        assert_eq!(fmt(45356.0, "\u{ff}\u{ff}/mm/dd"), "24/03/05");
        assert_eq!(fmt(45356.0, "D\u{10e}.\u{160}S"), "05.00");
        assert_eq!(fmt(12200000.0, "0.00\u{c9}+00"), "1.22E+07");
        // Letters that are not codes stay as written.
        assert_eq!(fmt(0.400544, "\u{f3}\u{f3}:pp:mm"), "\u{f3}\u{f3}:pp:01");
        assert_eq!(fmt(45356.0, "d \"\u{e9}\u{f1}\""), "5 \u{e9}\u{f1}");
    }

    #[test]
    fn b_is_the_buddhist_year_code() {
        // 2024 is 2567 in the Buddhist era; serial 5 is in 1900 (2443).
        assert_eq!(fmt(45356.0, "bbbb"), "2567");
        assert_eq!(fmt(45356.0, "BBBB"), "2567");
        assert_eq!(fmt(45356.0, "bb"), "67");
        assert_eq!(fmt(45356.0, "B"), "67");
        assert_eq!(fmt(45356.0, "dd/mm/bbbb"), "05/03/2567");
        assert_eq!(fmt(5.0, "B;0"), "43");
        // Escaped or quoted, `b` is text.
        assert_eq!(fmt(5.0, "0\\B"), "5B");
        assert_eq!(fmt(5.0, "0.0\"b\""), "5.0b");
    }

    #[test]
    fn bang_shows_the_next_character_as_written() {
        assert_eq!(fmt(5.0, "0!n"), "5n");
        assert_eq!(fmt(5.0, "0!E"), "5E");
        assert_eq!(fmt(5.0, "0!B"), "5B");
        assert_eq!(fmt(5.0, "0!b"), "5b");
        assert_eq!(fmt(5.0, "0\\n"), "5n");
        assert_eq!(fmt(203.0, "!r0c00"), "r2c03");
        assert_eq!(fmt(123456.0, "0!.0,"), "12.3");
        assert_eq!(fmt(3.0, "0!!"), "3!");
        // An escaped `;` does not start a section.
        assert_eq!(fmt(-5.0, "0;0!;"), "5;");
        assert_eq!(format_text("abc", "!n@").unwrap(), "nabc");
    }
}
