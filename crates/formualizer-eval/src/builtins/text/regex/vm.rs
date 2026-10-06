//! A backtracking matcher for the parsed PCRE2 patterns: leftmost-first
//! alternation, greedy, lazy and possessive repeats, atomic groups and
//! lookaround, as PCRE2 matches them.

use super::syntax::{Assertion, Condition, Greed, Node, Pattern, PatternError, char_class, word};
use regex_syntax::hir::ClassUnicode;

/// Instructions a pattern compiles to.
#[derive(Debug, Clone)]
enum Inst {
    Char(char),
    Set(usize),
    AnyButNewline,
    /// Try `first`, and `second` on backtracking.
    Split(usize, usize),
    Jmp(usize),
    Save(usize),
    Assert(Assertion),
    Backref {
        group: usize,
        caseless: bool,
    },
    /// Records the position an iteration of an unlimited repeat starts at.
    SetReg(usize),
    /// Ends an iteration of an unlimited repeat whose body can match nothing:
    /// an iteration that matched nothing leaves the loop (as PCRE2 does),
    /// any other goes back to `back`.
    LoopEnd {
        reg: usize,
        back: usize,
    },
    /// An atomic group: the first way the sub-program matches, kept.
    Atomic(usize),
    Look {
        look: usize,
    },
    /// A conditional on a group: on to the next instruction when it is set,
    /// to `no` otherwise.
    CondGroup {
        group: usize,
        no: usize,
    },
    CondLook {
        look: usize,
        no: usize,
    },
    KeepOut,
    Fail,
    Match,
}

#[derive(Debug, Clone)]
struct Look {
    negate: bool,
    /// One sub-program for a lookahead; one per alternative, with the
    /// fewest and most characters it matches, for a lookbehind.
    behind: Option<Vec<(usize, usize, usize)>>,
    prog: usize,
}

pub(crate) struct Regex {
    progs: Vec<Vec<Inst>>,
    sets: Vec<ClassUnicode>,
    looks: Vec<Look>,
    pub groups: usize,
    pub names: Vec<(String, usize)>,
    regs: usize,
    word: ClassUnicode,
    /// A character every match contains, when there is one: a text without
    /// it has no match, found without backtracking (PCRE2 makes the same
    /// check before matching).
    required: Option<char>,
}

fn required_char(node: &Node) -> Option<char> {
    match node {
        Node::Char(c) => Some(*c),
        Node::Concat(items) => items.iter().find_map(required_char),
        Node::Capture { node, .. } | Node::Atomic(node) => required_char(node),
        Node::Repeat { node, min, .. } if *min >= 1 => required_char(node),
        _ => None,
    }
}

/// The most instructions a pattern may expand to (counted repeats are
/// unrolled); a larger one is not computed.
const MAX_PROGRAM: usize = 200_000;
/// The most matcher steps one call may take; beyond it the call is not
/// computed. Excel stops at PCRE2's match limit with #VALUE!, and where it
/// stops is known only from probes (Excel for Windows 16.0.20430:
/// `^(a|a)*$` is FALSE on 17 a's and a "!", 2.2 million steps here, and
/// #VALUE! on 18, 4.5 million; `(a+)+$` is FALSE on 18 a's and a "b", also
/// 4.5 million steps here, and #VALUE! on 20). The budget sits well below the
/// smallest count Excel refused, so no call Excel refuses is computed.
const STEP_BUDGET: u64 = 2_000_000;

#[derive(Debug)]
pub(crate) struct Exhausted;

struct Compiler {
    progs: Vec<Vec<Inst>>,
    sets: Vec<ClassUnicode>,
    looks: Vec<Look>,
    regs: usize,
}

fn nullable(node: &Node) -> bool {
    match node {
        Node::Set(_) | Node::Char(_) | Node::AnyButNewline | Node::Fail => false,
        Node::Concat(items) => items.iter().all(nullable),
        Node::Alternate(items) => items.iter().any(nullable),
        Node::Capture { node, .. } | Node::Atomic(node) => nullable(node),
        Node::Repeat { node, min, .. } => *min == 0 || nullable(node),
        Node::Conditional { yes, no, .. } => nullable(yes) || nullable(no),
        Node::Empty
        | Node::Look { .. }
        | Node::Backref { .. }
        | Node::Assert(_)
        | Node::KeepOut => true,
    }
}

/// The longest variable-length lookbehind PCRE2 accepts.
const MAX_VARIABLE_LOOKBEHIND: usize = 255;

/// The fewest and most characters a node matches inside a lookbehind;
/// unbounded repeats are refused there, as by PCRE2.
fn length_bounds(node: &Node) -> Result<(usize, usize), PatternError> {
    Ok(match node {
        Node::Set(_) | Node::Char(_) | Node::AnyButNewline => (1, 1),
        Node::Empty | Node::Look { .. } | Node::Assert(_) | Node::KeepOut | Node::Fail => (0, 0),
        Node::Concat(items) => {
            let mut total = (0usize, 0usize);
            for item in items {
                let (lo, hi) = length_bounds(item)?;
                total = (total.0 + lo, total.1.saturating_add(hi));
            }
            total
        }
        Node::Alternate(items) => {
            let mut bounds: Option<(usize, usize)> = None;
            for item in items {
                let (lo, hi) = length_bounds(item)?;
                bounds = Some(match bounds {
                    None => (lo, hi),
                    Some((a, b)) => (a.min(lo), b.max(hi)),
                });
            }
            bounds.unwrap_or((0, 0))
        }
        Node::Capture { node, .. } | Node::Atomic(node) => length_bounds(node)?,
        Node::Repeat { node, min, max, .. } => {
            let Some(max) = max else {
                return Err(PatternError::Invalid(
                    "length of lookbehind assertion is not limited".into(),
                ));
            };
            let (lo, hi) = length_bounds(node)?;
            (lo * *min as usize, hi.saturating_mul(*max as usize))
        }
        Node::Conditional { yes, no, .. } => {
            let (a, b) = length_bounds(yes)?;
            let (c, d) = length_bounds(no)?;
            (a.min(c), b.max(d))
        }
        Node::Backref { .. } => {
            return Err(PatternError::Unsupported(
                "backreference in lookbehind".into(),
            ));
        }
    })
}

impl Compiler {
    fn emit(&mut self, prog: usize, inst: Inst) -> usize {
        self.progs[prog].push(inst);
        if self.progs.iter().map(Vec::len).sum::<usize>() > MAX_PROGRAM {
            // Checked by the caller through `too_big`.
        }
        self.progs[prog].len() - 1
    }

    fn too_big(&self) -> bool {
        self.progs.iter().map(Vec::len).sum::<usize>() > MAX_PROGRAM
    }

    fn here(&self, prog: usize) -> usize {
        self.progs[prog].len()
    }

    fn patch(&mut self, prog: usize, at: usize, target: usize) {
        match &mut self.progs[prog][at] {
            Inst::Split(_, second) => *second = target,
            Inst::Jmp(t) => *t = target,
            Inst::CondGroup { no, .. } | Inst::CondLook { no, .. } => *no = target,
            _ => unreachable!(),
        }
    }

    fn new_prog(&mut self) -> usize {
        self.progs.push(Vec::new());
        self.progs.len() - 1
    }

    fn sub_program(&mut self, node: &Node) -> Result<usize, PatternError> {
        let prog = self.new_prog();
        self.compile(prog, node)?;
        self.emit(prog, Inst::Match);
        Ok(prog)
    }

    fn look(&mut self, behind: bool, negate: bool, node: &Node) -> Result<usize, PatternError> {
        let look = if behind {
            let alternatives: Vec<&Node> = match node {
                Node::Alternate(items) => items.iter().collect(),
                other => vec![other],
            };
            let mut parts = Vec::new();
            for alternative in alternatives {
                let (min, max) = length_bounds(alternative)?;
                if min != max && max > MAX_VARIABLE_LOOKBEHIND {
                    return Err(PatternError::Invalid(
                        "branch too long in variable-length lookbehind assertion".into(),
                    ));
                }
                parts.push((self.sub_program(alternative)?, min, max));
            }
            Look {
                negate,
                behind: Some(parts),
                prog: 0,
            }
        } else {
            Look {
                negate,
                behind: None,
                prog: self.sub_program(node)?,
            }
        };
        self.looks.push(look);
        Ok(self.looks.len() - 1)
    }

    fn compile(&mut self, prog: usize, node: &Node) -> Result<(), PatternError> {
        if self.too_big() {
            return Err(PatternError::Unsupported(
                "pattern too large to expand".into(),
            ));
        }
        match node {
            Node::Empty => {}
            Node::Char(c) => {
                self.emit(prog, Inst::Char(*c));
            }
            Node::Set(class) => {
                self.sets.push(class.clone());
                let index = self.sets.len() - 1;
                self.emit(prog, Inst::Set(index));
            }
            Node::AnyButNewline => {
                self.emit(prog, Inst::AnyButNewline);
            }
            Node::Fail => {
                self.emit(prog, Inst::Fail);
            }
            Node::KeepOut => {
                self.emit(prog, Inst::KeepOut);
            }
            Node::Assert(assertion) => {
                self.emit(prog, Inst::Assert(*assertion));
            }
            Node::Backref { group, caseless } => {
                self.emit(
                    prog,
                    Inst::Backref {
                        group: *group,
                        caseless: *caseless,
                    },
                );
            }
            Node::Concat(items) => {
                for item in items {
                    self.compile(prog, item)?;
                }
            }
            Node::Alternate(items) => {
                let mut jumps = Vec::new();
                for (i, item) in items.iter().enumerate() {
                    if i + 1 < items.len() {
                        let split = self.emit(prog, Inst::Split(0, 0));
                        let first = self.here(prog);
                        if let Inst::Split(f, _) = &mut self.progs[prog][split] {
                            *f = first;
                        }
                        self.compile(prog, item)?;
                        jumps.push(self.emit(prog, Inst::Jmp(0)));
                        let next = self.here(prog);
                        self.patch(prog, split, next);
                    } else {
                        self.compile(prog, item)?;
                    }
                }
                let end = self.here(prog);
                for jump in jumps {
                    self.patch(prog, jump, end);
                }
            }
            Node::Capture { index, node } => {
                self.emit(prog, Inst::Save(2 * index));
                self.compile(prog, node)?;
                self.emit(prog, Inst::Save(2 * index + 1));
            }
            Node::Atomic(node) => {
                let sub = self.sub_program(node)?;
                self.emit(prog, Inst::Atomic(sub));
            }
            Node::Look {
                behind,
                negate,
                node,
            } => {
                let look = self.look(*behind, *negate, node)?;
                self.emit(prog, Inst::Look { look });
            }
            Node::Conditional { condition, yes, no } => {
                let test = match condition {
                    Condition::Group(group) => self.emit(
                        prog,
                        Inst::CondGroup {
                            group: *group,
                            no: 0,
                        },
                    ),
                    Condition::Look(look) => {
                        let Node::Look {
                            behind,
                            negate,
                            node,
                        } = look.as_ref()
                        else {
                            unreachable!()
                        };
                        let look = self.look(*behind, *negate, node)?;
                        self.emit(prog, Inst::CondLook { look, no: 0 })
                    }
                };
                self.compile(prog, yes)?;
                let jump = self.emit(prog, Inst::Jmp(0));
                let no_start = self.here(prog);
                self.patch(prog, test, no_start);
                self.compile(prog, no)?;
                let end = self.here(prog);
                self.patch(prog, jump, end);
            }
            Node::Repeat {
                node,
                min,
                max,
                greed,
            } => {
                if *greed == Greed::Possessive {
                    let greedy = Node::Repeat {
                        node: node.clone(),
                        min: *min,
                        max: *max,
                        greed: Greed::Greedy,
                    };
                    let sub = self.sub_program(&greedy)?;
                    self.emit(prog, Inst::Atomic(sub));
                    return Ok(());
                }
                let lazy = *greed == Greed::Lazy;
                for _ in 0..*min {
                    self.compile(prog, node)?;
                    if self.too_big() {
                        return Err(PatternError::Unsupported(
                            "pattern too large to expand".into(),
                        ));
                    }
                }
                match max {
                    None => {
                        let empty_check = nullable(node);
                        let reg = self.regs;
                        if empty_check {
                            self.regs += 1;
                        }
                        let top = self.emit(prog, Inst::Split(0, 0));
                        let body = self.here(prog);
                        if empty_check {
                            self.emit(prog, Inst::SetReg(reg));
                        }
                        self.compile(prog, node)?;
                        if empty_check {
                            self.emit(prog, Inst::LoopEnd { reg, back: top });
                        } else {
                            self.emit(prog, Inst::Jmp(top));
                        }
                        let exit = self.here(prog);
                        self.progs[prog][top] = if lazy {
                            Inst::Split(exit, body)
                        } else {
                            Inst::Split(body, exit)
                        };
                    }
                    Some(max) => {
                        let mut splits = Vec::new();
                        for _ in *min..*max {
                            let split = self.emit(prog, Inst::Split(0, 0));
                            splits.push(split);
                            self.compile(prog, node)?;
                            if self.too_big() {
                                return Err(PatternError::Unsupported(
                                    "pattern too large to expand".into(),
                                ));
                            }
                        }
                        let exit = self.here(prog);
                        for split in splits {
                            let body = split + 1;
                            self.progs[prog][split] = if lazy {
                                Inst::Split(exit, body)
                            } else {
                                Inst::Split(body, exit)
                            };
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl Regex {
    pub(crate) fn new(pattern: Pattern) -> Result<Self, PatternError> {
        let mut compiler = Compiler {
            progs: vec![Vec::new()],
            sets: Vec::new(),
            looks: Vec::new(),
            regs: 0,
        };
        compiler.emit(0, Inst::Save(0));
        compiler.compile(0, &pattern.node)?;
        compiler.emit(0, Inst::Save(1));
        compiler.emit(0, Inst::Match);
        if compiler.too_big() {
            return Err(PatternError::Unsupported(
                "pattern too large to expand".into(),
            ));
        }
        Ok(Self {
            progs: compiler.progs,
            sets: compiler.sets,
            looks: compiler.looks,
            groups: pattern.groups,
            names: pattern.names,
            regs: compiler.regs,
            word: word(),
            required: required_char(&pattern.node),
        })
    }

    pub(crate) fn slots(&self) -> usize {
        2 * (self.groups + 1)
    }
}

fn in_class(class: &ClassUnicode, c: char) -> bool {
    let ranges = class.ranges();
    let mut lo = 0;
    let mut hi = ranges.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if c < ranges[mid].start() {
            hi = mid;
        } else if c > ranges[mid].end() {
            lo = mid + 1;
        } else {
            return true;
        }
    }
    false
}

/// `\r` and `\n` are newlines; `\r\n` is one newline.
fn is_newline(c: char) -> bool {
    c == '\n' || c == '\r'
}

enum Frame {
    Alt { pc: usize, pos: usize },
    Slot { index: usize, old: Option<usize> },
    Reg { index: usize, old: usize },
}

/// One search over a subject.
pub(crate) struct Matcher<'r> {
    re: &'r Regex,
    text: &'r [char],
    /// Where the current search began, for `\G`.
    search_start: usize,
    pub steps: u64,
    /// When set, an empty match at this position is no match
    /// (PCRE2_NOTEMPTY_ATSTART).
    not_empty_at: Option<usize>,
}

impl<'r> Matcher<'r> {
    pub(crate) fn new(re: &'r Regex, text: &'r [char]) -> Self {
        Self {
            re,
            text,
            search_start: 0,
            steps: 0,
            not_empty_at: None,
        }
    }

    fn word_at(&self, pos: usize) -> bool {
        self.text
            .get(pos)
            .is_some_and(|&c| in_class(&self.re.word, c))
    }

    fn assertion(&self, assertion: Assertion, pos: usize) -> bool {
        let text = self.text;
        let len = text.len();
        // Before a newline that ends the subject.
        let final_newline = |pos: usize| {
            pos == len
                || (pos + 1 == len && is_newline(text[pos]))
                || (pos + 2 == len && text[pos] == '\r' && text[pos + 1] == '\n')
        };
        match assertion {
            Assertion::TextStart => pos == 0,
            Assertion::TextEnd => pos == len,
            Assertion::TextEndNewline | Assertion::LineEnd { multiline: false } => {
                final_newline(pos)
            }
            // In multiline mode `$` holds before every CR and LF and `^` after
            // every one, even inside a CR LF pair and after a final newline
            // (Excel: REGEXTEST("a"&CHAR(10),"(?m)^$") is TRUE).
            Assertion::LineEnd { multiline: true } => pos == len || is_newline(text[pos]),
            Assertion::LineStart { multiline: false } => pos == 0,
            Assertion::LineStart { multiline: true } => pos == 0 || is_newline(text[pos - 1]),
            Assertion::SearchStart => pos == self.search_start,
            Assertion::WordBoundary | Assertion::NotWordBoundary => {
                let before = pos > 0 && self.word_at(pos - 1);
                let after = self.word_at(pos);
                (before != after) == matches!(assertion, Assertion::WordBoundary)
            }
        }
    }

    fn same_char(a: char, b: char, caseless: bool) -> bool {
        a == b || (caseless && in_class(&char_class(a, true), b))
    }

    /// Runs program `prog` from `start`; on success returns the end and
    /// leaves the captures in `slots`. `end` requires the match to end there
    /// (a lookbehind alternative).
    fn run(
        &mut self,
        prog: usize,
        start: usize,
        slots: &mut [Option<usize>],
        regs: &mut [usize],
        end: Option<usize>,
    ) -> Result<Option<usize>, Exhausted> {
        let program = &self.re.progs[prog];
        let text = self.text;
        let mut stack: Vec<Frame> = Vec::new();
        let mut pc = 0;
        let mut pos = start;
        loop {
            self.steps += 1;
            if self.steps > STEP_BUDGET {
                return Err(Exhausted);
            }
            let mut ok = true;
            match &program[pc] {
                Inst::Char(c) => {
                    if text.get(pos) == Some(c) {
                        pos += 1;
                        pc += 1;
                    } else {
                        ok = false;
                    }
                }
                Inst::Set(index) => {
                    if text
                        .get(pos)
                        .is_some_and(|&c| in_class(&self.re.sets[*index], c))
                    {
                        pos += 1;
                        pc += 1;
                    } else {
                        ok = false;
                    }
                }
                Inst::AnyButNewline => {
                    if text.get(pos).is_some_and(|&c| !is_newline(c)) {
                        pos += 1;
                        pc += 1;
                    } else {
                        ok = false;
                    }
                }
                Inst::Split(first, second) => {
                    stack.push(Frame::Alt { pc: *second, pos });
                    pc = *first;
                }
                Inst::Jmp(target) => pc = *target,
                Inst::Save(index) => {
                    stack.push(Frame::Slot {
                        index: *index,
                        old: slots[*index],
                    });
                    slots[*index] = Some(pos);
                    pc += 1;
                }
                Inst::KeepOut => {
                    stack.push(Frame::Slot {
                        index: 0,
                        old: slots[0],
                    });
                    slots[0] = Some(pos);
                    pc += 1;
                }
                Inst::Assert(assertion) => {
                    if self.assertion(*assertion, pos) {
                        pc += 1;
                    } else {
                        ok = false;
                    }
                }
                Inst::Backref { group, caseless } => {
                    match (slots[2 * group], slots[2 * group + 1]) {
                        (Some(s), Some(e)) if s <= e => {
                            let length = e - s;
                            let matches = pos + length <= text.len()
                                && (0..length).all(|i| {
                                    Self::same_char(text[s + i], text[pos + i], *caseless)
                                });
                            if matches {
                                pos += length;
                                pc += 1;
                            } else {
                                ok = false;
                            }
                        }
                        _ => ok = false,
                    }
                }
                Inst::SetReg(index) => {
                    stack.push(Frame::Reg {
                        index: *index,
                        old: regs[*index],
                    });
                    regs[*index] = pos;
                    pc += 1;
                }
                Inst::LoopEnd { reg, back } => {
                    if regs[*reg] == pos {
                        pc += 1;
                    } else {
                        pc = *back;
                    }
                }
                Inst::Atomic(sub) => {
                    let before = slots.to_vec();
                    match self.run(*sub, pos, slots, regs, None)? {
                        Some(sub_end) => {
                            for (index, old) in before.into_iter().enumerate() {
                                if slots[index] != old {
                                    stack.push(Frame::Slot { index, old });
                                }
                            }
                            pos = sub_end;
                            pc += 1;
                        }
                        None => ok = false,
                    }
                }
                Inst::Look { look } => {
                    if self.look(*look, pos, slots, regs, &mut stack)? {
                        pc += 1;
                    } else {
                        ok = false;
                    }
                }
                Inst::CondGroup { group, no } => {
                    if slots[2 * group + 1].is_some() {
                        pc += 1;
                    } else {
                        pc = *no;
                    }
                }
                Inst::CondLook { look, no } => {
                    if self.look(*look, pos, slots, regs, &mut stack)? {
                        pc += 1;
                    } else {
                        pc = *no;
                    }
                }
                Inst::Fail => ok = false,
                Inst::Match => {
                    let empty_refused = self
                        .not_empty_at
                        .is_some_and(|at| at == start && pos == start && prog == 0);
                    if end.is_some_and(|end| end != pos) || empty_refused {
                        ok = false;
                    } else {
                        return Ok(Some(pos));
                    }
                }
            }
            if !ok {
                // Backtrack to the latest alternative, undoing captures.
                loop {
                    match stack.pop() {
                        None => return Ok(None),
                        Some(Frame::Slot { index, old }) => slots[index] = old,
                        Some(Frame::Reg { index, old }) => regs[index] = old,
                        Some(Frame::Alt {
                            pc: alt_pc,
                            pos: alt_pos,
                        }) => {
                            pc = alt_pc;
                            pos = alt_pos;
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Whether lookaround `look` holds at `pos`. A positive one keeps the
    /// captures it set (undone with `stack` on backtracking); a negative one
    /// keeps none.
    fn look(
        &mut self,
        look: usize,
        pos: usize,
        slots: &mut [Option<usize>],
        regs: &mut [usize],
        stack: &mut Vec<Frame>,
    ) -> Result<bool, Exhausted> {
        let spec = self.re.looks[look].clone();
        let before = slots.to_vec();
        let found = match &spec.behind {
            None => self.run(spec.prog, pos, slots, regs, None)?.is_some(),
            Some(parts) => {
                // Each alternative in turn, from its longest length down.
                let mut found = false;
                'alternatives: for (prog, min, max) in parts {
                    for length in (*min..=*max).rev() {
                        if pos >= length
                            && self
                                .run(*prog, pos - length, slots, regs, Some(pos))?
                                .is_some()
                        {
                            found = true;
                            break 'alternatives;
                        }
                    }
                }
                found
            }
        };
        if found && !spec.negate {
            for (index, old) in before.into_iter().enumerate() {
                if slots[index] != old {
                    stack.push(Frame::Slot { index, old });
                }
            }
        } else {
            slots.copy_from_slice(&before);
        }
        Ok(found != spec.negate)
    }

    /// The leftmost match at or after `from`: its capture slots. With
    /// `anchored`, only a match starting at `from`.
    pub(crate) fn find(
        &mut self,
        from: usize,
        anchored: bool,
        not_empty: bool,
    ) -> Result<Option<Vec<Option<usize>>>, Exhausted> {
        self.search_start = from;
        self.not_empty_at = not_empty.then_some(from);
        if let Some(required) = self.re.required
            && !self
                .text
                .get(from..)
                .is_some_and(|rest| rest.contains(&required))
        {
            return Ok(None);
        }
        let mut slots = vec![None; self.re.slots()];
        let mut regs = vec![0; self.re.regs];
        let last = if anchored { from } else { self.text.len() };
        for start in from..=last {
            if self.run(0, start, &mut slots, &mut regs, None)?.is_some() {
                return Ok(Some(slots));
            }
            slots.iter_mut().for_each(|slot| *slot = None);
        }
        Ok(None)
    }
}
