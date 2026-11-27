//! A small expression language: numbers, variables, `+ - * / % ^`, comparisons, `&& || !`,
//! `cond ? a : b`, and functions (`sin`, `min`, `clamp`, `rand()`, `randn()`, …).
//!
//! * [`compile`] parses once and reports the column of a syntax error, so a UI can point at it.
//! * [`Expr::eval`] runs it against a variable lookup and an [`Rng`]; the result is always finite
//!   (a divide-by-zero or `sqrt(-1)` is an `Err`, never a silent NaN).
//! * [`Rng`] is a seeded xorshift generator — the same seed gives the same sequence, which makes
//!   generated demo data repeatable and tests deterministic.
//!
//! Pure: no I/O, no globals, no dependencies.

use std::fmt;

// ---------------------------------------------------------------------------
// Random numbers
// ---------------------------------------------------------------------------

/// Seeded xorshift64* generator.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
    spare: Option<f64>,
}

impl Rng {
    /// A seed of 0 is remapped (xorshift cannot start from zero); mix so nearby seeds diverge.
    pub fn new(seed: u64) -> Self {
        let mut s = seed ^ 0x9E37_79B9_7F4A_7C15;
        if s == 0 {
            s = 0x2545_F491_4F6C_DD1D;
        }
        let mut r = Self { state: s, spare: None };
        for _ in 0..4 {
            r.next_u64();
        }
        r
    }

    /// Seed from the wall clock (for "random every run").
    pub fn from_clock() -> Self {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        Self::new(n)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[0, 1)`.
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in `[lo, hi]` (order-insensitive).
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        let (a, b) = if lo <= hi { (lo, hi) } else { (hi, lo) };
        a + (b - a) * self.uniform()
    }

    /// Standard normal (Box–Muller, caching the second value).
    pub fn gauss(&mut self) -> f64 {
        if let Some(s) = self.spare.take() {
            return s;
        }
        let u1 = self.uniform().max(1e-300);
        let u2 = self.uniform();
        let r = (-2.0 * u1.ln()).sqrt();
        let th = std::f64::consts::TAU * u2;
        self.spare = Some(r * th.sin());
        r * th.cos()
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// A syntax error with the (0-based) character column it was found at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub msg: String,
    pub col: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at column {})", self.msg, self.col + 1)
    }
}

impl std::error::Error for ParseError {}

fn perr<T>(msg: impl Into<String>, col: usize) -> Result<T, ParseError> {
    Err(ParseError { msg: msg.into(), col })
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Id(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
    Question,
    Colon,
    End,
}

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, ParseError> {
    let cs: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        let at = i;
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && cs.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let mut j = i;
            while j < cs.len() && (cs[j].is_ascii_digit() || cs[j] == '.') {
                j += 1;
            }
            // Exponent: 1e3, 2.5E-4 (only when digits follow, so `2e` stays an error below).
            if j < cs.len() && (cs[j] == 'e' || cs[j] == 'E') {
                let mut k = j + 1;
                if k < cs.len() && (cs[k] == '+' || cs[k] == '-') {
                    k += 1;
                }
                if k < cs.len() && cs[k].is_ascii_digit() {
                    while k < cs.len() && cs[k].is_ascii_digit() {
                        k += 1;
                    }
                    j = k;
                }
            }
            let text: String = cs[i..j].iter().collect();
            match text.parse::<f64>() {
                Ok(v) => out.push((Tok::Num(v), at)),
                Err(_) => return perr(format!("bad number '{text}'"), at),
            }
            i = j;
        } else if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < cs.len() && (cs[j].is_alphanumeric() || cs[j] == '_' || cs[j] == '.') {
                j += 1;
            }
            out.push((Tok::Id(cs[i..j].iter().collect()), at));
            i = j;
        } else {
            let two: String = cs[i..(i + 2).min(cs.len())].iter().collect();
            let t = match two.as_str() {
                "==" => Some("=="),
                "!=" => Some("!="),
                "<=" => Some("<="),
                ">=" => Some(">="),
                "&&" => Some("&&"),
                "||" => Some("||"),
                _ => None,
            };
            if let Some(t) = t {
                out.push((Tok::Op(t), at));
                i += 2;
                continue;
            }
            let tok = match c {
                '+' => Tok::Op("+"),
                '-' => Tok::Op("-"),
                '*' => Tok::Op("*"),
                '/' => Tok::Op("/"),
                '%' => Tok::Op("%"),
                '^' => Tok::Op("^"),
                '<' => Tok::Op("<"),
                '>' => Tok::Op(">"),
                '!' => Tok::Op("!"),
                '(' => Tok::LParen,
                ')' => Tok::RParen,
                ',' => Tok::Comma,
                '?' => Tok::Question,
                ':' => Tok::Colon,
                other => return perr(format!("unexpected '{other}'"), at),
            };
            out.push((tok, at));
            i += 1;
        }
    }
    out.push((Tok::End, cs.len()));
    Ok(out)
}

// ---------------------------------------------------------------------------
// AST + parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Num(f64),
    Var(String),
    Neg(Box<Node>),
    Not(Box<Node>),
    Bin(&'static str, Box<Node>, Box<Node>),
    Cond(Box<Node>, Box<Node>, Box<Node>),
    Call(Func, Vec<Node>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Func {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Abs,
    Sqrt,
    Pow,
    Exp,
    Ln,
    Log10,
    Min,
    Max,
    Clamp,
    Floor,
    Ceil,
    Round,
    Sign,
    Mod,
    Lerp,
    If,
    Rand,
    Randn,
    Noise,
    /// `rand(lo, hi)` — uniform in a range.
    RandRange,
}

/// `(name, min args, max args, func)`.
const FUNCS: &[(&str, usize, usize, Func)] = &[
    ("sin", 1, 1, Func::Sin),
    ("cos", 1, 1, Func::Cos),
    ("tan", 1, 1, Func::Tan),
    ("asin", 1, 1, Func::Asin),
    ("acos", 1, 1, Func::Acos),
    ("atan", 1, 1, Func::Atan),
    ("atan2", 2, 2, Func::Atan2),
    ("abs", 1, 1, Func::Abs),
    ("sqrt", 1, 1, Func::Sqrt),
    ("pow", 2, 2, Func::Pow),
    ("exp", 1, 1, Func::Exp),
    ("ln", 1, 1, Func::Ln),
    ("log", 1, 1, Func::Ln),
    ("log10", 1, 1, Func::Log10),
    ("min", 2, 8, Func::Min),
    ("max", 2, 8, Func::Max),
    ("clamp", 3, 3, Func::Clamp),
    ("floor", 1, 1, Func::Floor),
    ("ceil", 1, 1, Func::Ceil),
    ("round", 1, 1, Func::Round),
    ("sign", 1, 1, Func::Sign),
    ("mod", 2, 2, Func::Mod),
    ("lerp", 3, 3, Func::Lerp),
    ("if", 3, 3, Func::If),
    ("rand", 0, 2, Func::Rand),
    ("randn", 0, 0, Func::Randn),
    ("noise", 1, 1, Func::Noise),
];

/// Function names, for editor hints.
pub fn function_names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = FUNCS.iter().map(|f| f.0).collect();
    v.dedup();
    v
}

struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].0
    }
    fn col(&self) -> usize {
        self.toks[self.pos].1
    }
    fn bump(&mut self) -> Tok {
        let t = self.toks[self.pos].0.clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }
    fn eat_op(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Tok::Op(o) if *o == s) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Result<Node, ParseError> {
        let c = self.or()?;
        if matches!(self.peek(), Tok::Question) {
            self.bump();
            let a = self.expr()?;
            if !matches!(self.peek(), Tok::Colon) {
                return perr("expected ':' in ?: expression", self.col());
            }
            self.bump();
            let b = self.expr()?;
            return Ok(Node::Cond(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }

    fn or(&mut self) -> Result<Node, ParseError> {
        let mut l = self.and()?;
        while self.eat_op("||") {
            let r = self.and()?;
            l = Node::Bin("||", Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn and(&mut self) -> Result<Node, ParseError> {
        let mut l = self.cmp()?;
        while self.eat_op("&&") {
            let r = self.cmp()?;
            l = Node::Bin("&&", Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn cmp(&mut self) -> Result<Node, ParseError> {
        let mut l = self.add()?;
        loop {
            let op = match self.peek() {
                Tok::Op(o) if matches!(*o, "==" | "!=" | "<" | "<=" | ">" | ">=") => *o,
                _ => return Ok(l),
            };
            self.bump();
            let r = self.add()?;
            l = Node::Bin(op, Box::new(l), Box::new(r));
        }
    }

    fn add(&mut self) -> Result<Node, ParseError> {
        let mut l = self.mul()?;
        loop {
            let op = match self.peek() {
                Tok::Op(o) if matches!(*o, "+" | "-") => *o,
                _ => return Ok(l),
            };
            self.bump();
            let r = self.mul()?;
            l = Node::Bin(op, Box::new(l), Box::new(r));
        }
    }

    fn mul(&mut self) -> Result<Node, ParseError> {
        let mut l = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Op(o) if matches!(*o, "*" | "/" | "%") => *o,
                _ => return Ok(l),
            };
            self.bump();
            let r = self.unary()?;
            l = Node::Bin(op, Box::new(l), Box::new(r));
        }
    }

    fn unary(&mut self) -> Result<Node, ParseError> {
        if self.eat_op("-") {
            return Ok(Node::Neg(Box::new(self.unary()?)));
        }
        if self.eat_op("+") {
            return self.unary();
        }
        if self.eat_op("!") {
            return Ok(Node::Not(Box::new(self.unary()?)));
        }
        self.power()
    }

    /// `^` is right-associative and binds tighter than unary minus on its left (`-2^2 = -4`).
    fn power(&mut self) -> Result<Node, ParseError> {
        let base = self.primary()?;
        if self.eat_op("^") {
            let exp = self.unary()?;
            return Ok(Node::Bin("^", Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Node, ParseError> {
        let col = self.col();
        match self.bump() {
            Tok::Num(v) => Ok(Node::Num(v)),
            Tok::LParen => {
                let e = self.expr()?;
                if !matches!(self.peek(), Tok::RParen) {
                    return perr("missing ')'", self.col());
                }
                self.bump();
                Ok(e)
            }
            Tok::Id(name) => {
                if matches!(self.peek(), Tok::LParen) {
                    self.bump();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Tok::RParen) {
                        loop {
                            args.push(self.expr()?);
                            if matches!(self.peek(), Tok::Comma) {
                                self.bump();
                                continue;
                            }
                            break;
                        }
                    }
                    if !matches!(self.peek(), Tok::RParen) {
                        return perr("missing ')' after arguments", self.col());
                    }
                    self.bump();
                    let lname = name.to_ascii_lowercase();
                    let Some(&(_, lo, hi, func)) = FUNCS.iter().find(|f| f.0 == lname) else {
                        return perr(format!("unknown function '{name}'"), col);
                    };
                    if args.len() < lo || args.len() > hi {
                        let want = if lo == hi { format!("{lo}") } else { format!("{lo}–{hi}") };
                        return perr(format!("{name}() takes {want} argument{}", if hi == 1 { "" } else { "s" }), col);
                    }
                    if func == Func::Rand && args.len() == 1 {
                        return perr("rand() takes 0 or 2 arguments", col);
                    }
                    let func = if func == Func::Rand && args.len() == 2 { Func::RandRange } else { func };
                    Ok(Node::Call(func, args))
                } else {
                    Ok(match name.to_ascii_lowercase().as_str() {
                        "pi" => Node::Num(std::f64::consts::PI),
                        "tau" => Node::Num(std::f64::consts::TAU),
                        "true" => Node::Num(1.0),
                        "false" => Node::Num(0.0),
                        _ => Node::Var(name),
                    })
                }
            }
            Tok::End => perr("expression ends early", col),
            Tok::RParen => perr("unexpected ')'", col),
            Tok::Comma => perr("unexpected ','", col),
            Tok::Question | Tok::Colon => perr("unexpected '?' / ':'", col),
            Tok::Op(o) => perr(format!("unexpected '{o}'"), col),
        }
    }
}

// ---------------------------------------------------------------------------
// Compiled expression
// ---------------------------------------------------------------------------

/// A parsed expression, cheap to evaluate repeatedly.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    root: Node,
    vars: Vec<String>,
}

/// Parse `src`.
pub fn compile(src: &str) -> Result<Expr, ParseError> {
    if src.trim().is_empty() {
        return perr("empty expression", 0);
    }
    let mut p = Parser { toks: lex(src)?, pos: 0 };
    let root = p.expr()?;
    if !matches!(p.peek(), Tok::End) {
        return perr("unexpected text after the expression", p.col());
    }
    let mut vars = Vec::new();
    collect_vars(&root, &mut vars);
    Ok(Expr { root, vars })
}

fn collect_vars(n: &Node, out: &mut Vec<String>) {
    match n {
        Node::Var(v) => {
            if !out.contains(v) {
                out.push(v.clone());
            }
        }
        Node::Num(_) => {}
        Node::Neg(a) | Node::Not(a) => collect_vars(a, out),
        Node::Bin(_, a, b) => {
            collect_vars(a, out);
            collect_vars(b, out);
        }
        Node::Cond(a, b, c) => {
            collect_vars(a, out);
            collect_vars(b, out);
            collect_vars(c, out);
        }
        Node::Call(_, args) => args.iter().for_each(|a| collect_vars(a, out)),
    }
}

impl Expr {
    /// Variable names the expression reads (in first-use order).
    pub fn variables(&self) -> &[String] {
        &self.vars
    }

    /// Evaluate. `lookup` resolves variables (`None` = unknown → error). Result is always finite.
    pub fn eval(&self, lookup: &dyn Fn(&str) -> Option<f64>, rng: &mut Rng) -> Result<f64, String> {
        let v = ev(&self.root, lookup, rng)?;
        if v.is_finite() {
            Ok(v)
        } else {
            Err("result is not a finite number".into())
        }
    }
}

/// One-shot: compile and evaluate with no variables except those in `vars`.
pub fn eval_with(src: &str, vars: &[(&str, f64)], rng: &mut Rng) -> Result<f64, String> {
    let e = compile(src).map_err(|e| e.to_string())?;
    e.eval(&|n| vars.iter().find(|(k, _)| *k == n).map(|(_, v)| *v), rng)
}

fn truthy(v: f64) -> bool {
    v != 0.0
}

fn ev(n: &Node, lookup: &dyn Fn(&str) -> Option<f64>, rng: &mut Rng) -> Result<f64, String> {
    Ok(match n {
        Node::Num(v) => *v,
        Node::Var(name) => lookup(name).ok_or_else(|| format!("unknown variable '{name}'"))?,
        Node::Neg(a) => -ev(a, lookup, rng)?,
        Node::Not(a) => (!truthy(ev(a, lookup, rng)?)) as u8 as f64,
        Node::Bin(op, a, b) => {
            // Short-circuit logic so `x != 0 && 1 / x > 2` is safe.
            if *op == "&&" {
                let l = ev(a, lookup, rng)?;
                return Ok(if truthy(l) { truthy(ev(b, lookup, rng)?) as u8 as f64 } else { 0.0 });
            }
            if *op == "||" {
                let l = ev(a, lookup, rng)?;
                return Ok(if truthy(l) { 1.0 } else { truthy(ev(b, lookup, rng)?) as u8 as f64 });
            }
            let l = ev(a, lookup, rng)?;
            let r = ev(b, lookup, rng)?;
            match *op {
                "+" => l + r,
                "-" => l - r,
                "*" => l * r,
                "/" => {
                    if r == 0.0 {
                        return Err("division by zero".into());
                    }
                    l / r
                }
                "%" => {
                    if r == 0.0 {
                        return Err("modulo by zero".into());
                    }
                    l % r
                }
                "^" => l.powf(r),
                "==" => ((l - r).abs() < 1e-12) as u8 as f64,
                "!=" => ((l - r).abs() >= 1e-12) as u8 as f64,
                "<" => (l < r) as u8 as f64,
                "<=" => (l <= r) as u8 as f64,
                ">" => (l > r) as u8 as f64,
                ">=" => (l >= r) as u8 as f64,
                _ => return Err(format!("bad operator {op}")),
            }
        }
        Node::Cond(c, a, b) => {
            if truthy(ev(c, lookup, rng)?) {
                ev(a, lookup, rng)?
            } else {
                ev(b, lookup, rng)?
            }
        }
        Node::Call(f, args) => {
            let mut a = Vec::with_capacity(args.len());
            // `if` evaluates lazily like ?: so a guarded branch never errors.
            if *f == Func::If {
                let c = ev(&args[0], lookup, rng)?;
                return ev(&args[if truthy(c) { 1 } else { 2 }], lookup, rng);
            }
            for x in args {
                a.push(ev(x, lookup, rng)?);
            }
            match f {
                Func::Sin => a[0].sin(),
                Func::Cos => a[0].cos(),
                Func::Tan => a[0].tan(),
                Func::Asin => a[0].asin(),
                Func::Acos => a[0].acos(),
                Func::Atan => a[0].atan(),
                Func::Atan2 => a[0].atan2(a[1]),
                Func::Abs => a[0].abs(),
                Func::Sqrt => {
                    if a[0] < 0.0 {
                        return Err("sqrt of a negative number".into());
                    }
                    a[0].sqrt()
                }
                Func::Pow => a[0].powf(a[1]),
                Func::Exp => a[0].exp(),
                Func::Ln => {
                    if a[0] <= 0.0 {
                        return Err("ln of a non-positive number".into());
                    }
                    a[0].ln()
                }
                Func::Log10 => {
                    if a[0] <= 0.0 {
                        return Err("log10 of a non-positive number".into());
                    }
                    a[0].log10()
                }
                Func::Min => a.iter().cloned().fold(f64::INFINITY, f64::min),
                Func::Max => a.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                Func::Clamp => {
                    let (lo, hi) = if a[1] <= a[2] { (a[1], a[2]) } else { (a[2], a[1]) };
                    a[0].clamp(lo, hi)
                }
                Func::Floor => a[0].floor(),
                Func::Ceil => a[0].ceil(),
                Func::Round => a[0].round(),
                Func::Sign => {
                    if a[0] > 0.0 {
                        1.0
                    } else if a[0] < 0.0 {
                        -1.0
                    } else {
                        0.0
                    }
                }
                Func::Mod => {
                    if a[1] == 0.0 {
                        return Err("mod by zero".into());
                    }
                    a[0].rem_euclid(a[1])
                }
                Func::Lerp => a[0] + (a[1] - a[0]) * a[2],
                Func::If => unreachable!(),
                Func::Rand => rng.uniform(),
                Func::RandRange => rng.range(a[0], a[1]),
                Func::Randn => rng.gauss(),
                Func::Noise => rng.gauss() * a[0],
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(src: &str) -> f64 {
        eval_with(src, &[], &mut Rng::new(1)).unwrap()
    }

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(num("1 + 2 * 3"), 7.0);
        assert_eq!(num("(1 + 2) * 3"), 9.0);
        assert_eq!(num("2 ^ 3 ^ 2"), 512.0); // right-associative
        assert_eq!(num("-2 ^ 2"), -4.0);
        assert_eq!(num("10 - 4 - 3"), 3.0); // left-associative
        assert_eq!(num("7 % 4"), 3.0);
        assert_eq!(num("1e3 + 2.5E-1"), 1000.25);
    }

    #[test]
    fn comparisons_logic_and_ternary() {
        assert_eq!(num("3 > 2 && 2 > 1"), 1.0);
        assert_eq!(num("1 > 2 || 5 == 5"), 1.0);
        assert_eq!(num("!(1 > 2)"), 1.0);
        assert_eq!(num("1 > 2 ? 10 : 20"), 20.0);
        assert_eq!(num("0 ? 1 : 0 ? 2 : 3"), 3.0);
        // Short-circuit and lazy branches keep guarded divisions safe.
        assert_eq!(num("0 != 0 && 1 / 0 > 1"), 0.0);
        assert_eq!(num("if(1, 4, 1 / 0)"), 4.0);
    }

    #[test]
    fn functions() {
        assert!((num("sin(pi / 2)") - 1.0).abs() < 1e-12);
        assert_eq!(num("max(1, 9, 4)"), 9.0);
        assert_eq!(num("min(3, 2)"), 2.0);
        assert_eq!(num("clamp(15, 0, 10)"), 10.0);
        assert_eq!(num("clamp(5, 10, 0)"), 5.0); // bounds in either order
        assert_eq!(num("abs(-3) + floor(2.7) + ceil(2.1) + round(2.5)"), 3.0 + 2.0 + 3.0 + 3.0);
        assert_eq!(num("mod(-1, 5)"), 4.0);
        assert_eq!(num("lerp(10, 20, 0.25)"), 12.5);
        assert_eq!(num("sign(-9)"), -1.0);
    }

    #[test]
    fn variables_and_their_listing() {
        let e = compile("a * x + a + t").unwrap();
        assert_eq!(e.variables(), ["a", "x", "t"]);
        let mut r = Rng::new(1);
        let v = e.eval(&|n| match n { "a" => Some(2.0), "x" => Some(3.0), "t" => Some(1.0), _ => None }, &mut r).unwrap();
        assert_eq!(v, 9.0);
        let err = e.eval(&|_| None, &mut r).unwrap_err();
        assert!(err.contains("unknown variable"));
        // Dotted names (signal refs like `tank.level`) are one identifier.
        assert_eq!(compile("tank.level + 1").unwrap().variables(), ["tank.level"]);
    }

    #[test]
    fn errors_are_never_nan_and_point_at_a_column() {
        let mut r = Rng::new(1);
        assert!(eval_with("1 / 0", &[], &mut r).is_err());
        assert!(eval_with("sqrt(-1)", &[], &mut r).is_err());
        assert!(eval_with("ln(0)", &[], &mut r).is_err());
        assert!(eval_with("1 % 0", &[], &mut r).is_err());
        assert!(eval_with("exp(1000)", &[], &mut r).is_err());
        let e = compile("1 + * 2").unwrap_err();
        assert_eq!(e.col, 4);
        assert_eq!(compile("2 +").unwrap_err().msg, "expression ends early");
        assert!(compile("(1 + 2").unwrap_err().msg.contains("missing ')'"));
        assert!(compile("foo(1)").unwrap_err().msg.contains("unknown function"));
        assert!(compile("sin()").unwrap_err().msg.contains("takes 1 argument"));
        assert!(compile("rand(1)").unwrap_err().msg.contains("0 or 2"));
        assert!(compile("").is_err());
        assert!(compile("1 2").unwrap_err().msg.contains("after the expression"));
        assert!(compile("1 @ 2").unwrap_err().msg.contains("unexpected '@'"));
    }

    #[test]
    fn rng_is_seeded_and_distributed() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        let sa: Vec<f64> = (0..8).map(|_| a.uniform()).collect();
        let sb: Vec<f64> = (0..8).map(|_| b.uniform()).collect();
        assert_eq!(sa, sb);
        assert_ne!(sa, (0..8).map(|_| Rng::new(43).uniform()).collect::<Vec<_>>());
        assert!(sa.iter().all(|v| (0.0..1.0).contains(v)));
        // Gaussian: mean ≈ 0, sd ≈ 1 over many draws.
        let mut r = Rng::new(7);
        let xs: Vec<f64> = (0..20000).map(|_| r.gauss()).collect();
        let mean = xs.iter().sum::<f64>() / xs.len() as f64;
        let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / xs.len() as f64;
        assert!(mean.abs() < 0.05, "mean {mean}");
        assert!((var.sqrt() - 1.0).abs() < 0.05, "sd {}", var.sqrt());
        // Range is order-insensitive and bounded.
        let mut r = Rng::new(3);
        for _ in 0..1000 {
            let v = r.range(5.0, -5.0);
            assert!((-5.0..=5.0).contains(&v));
        }
    }

    #[test]
    fn random_functions_in_expressions() {
        let e = compile("rand() + randn() * 0 + noise(0) + rand(10, 20)").unwrap();
        let mut r = Rng::new(9);
        for _ in 0..100 {
            let v = e.eval(&|_| None, &mut r).unwrap();
            assert!((10.0..21.0).contains(&v));
        }
        // Same seed → same random walk.
        let walk = compile("prev + randn() * 0.5").unwrap();
        let run = |seed| {
            let mut r = Rng::new(seed);
            let mut p = 0.0;
            for _ in 0..50 {
                p = walk.eval(&|n| (n == "prev").then_some(p), &mut r).unwrap();
            }
            p
        };
        assert_eq!(run(5), run(5));
        assert_ne!(run(5), run(6));
    }
}
