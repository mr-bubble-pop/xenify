//! Expressions: small formulas that compute a property's value, like After
//! Effects' expressions — e.g. `value + wiggle(2, 20)`, `time * 90`,
//! `loop_out()`, `[value.x, 540 + sin(time * 3) * 40]`.
//!
//! The language is deliberately tiny and safe: numbers and vectors, `+ - * /
//! %`, `? :` with comparisons, and a fixed set of functions. There are no
//! loops or assignments, so every expression finishes quickly and the same
//! inputs always give the same result (renders and exports stay reproducible).
//!
//! Names:
//! - `time` (or `t`) seconds, `frame`, `fps`, `duration`, `pi`
//! - `value`: the property's keyframed value at this time
//! - `index`: the layer's position in its composition (1 = bottom)
//!
//! Functions:
//! - maths: `sin cos tan asin acos atan atan2 abs sqrt pow exp ln log10 min max
//!   clamp floor ceil round sign fract degrees radians length normalize`
//! - `lerp(a, b, t)`, `smoothstep(e0, e1, x)`
//! - `noise(x)`: smooth noise, -1..1
//! - `random()` / `random(max)` / `random(min, max)`: new each frame, deterministic
//! - `wiggle(freq, amount)`: `value` plus smooth random drift
//! - `value_at(t)`: the keyframed value at another time
//! - `loop_out()` / `loop_in()`: repeat the keyframes after the last / before the first
//! - `ping_pong()`: play the keyframes forward, then backward, repeatedly

use xenify_doc::{AnyProperty, Color, Property, Value, ValueKind, Vec2};

/// Longest expression accepted (keeps parsing and evaluation trivially cheap).
const MAX_LEN: usize = 4000;
/// Deepest nesting accepted.
const MAX_DEPTH: usize = 64;

/// A number or a vector (positions `[x, y]`, colours `[r, g, b, a]`).
#[derive(Clone, Debug, PartialEq)]
pub enum V {
    Num(f64),
    Vec(Vec<f64>),
}

/// What an expression can see.
pub struct Env<'a> {
    pub time: f64,
    pub fps: f64,
    pub duration: f64,
    /// 1-based position of the layer in its composition.
    pub index: usize,
    /// Seeds `random()`/`wiggle()` so each layer moves differently.
    pub seed: u64,
    /// The property the expression is on (for `value`, `value_at`, loops).
    pub property: &'a AnyProperty,
}

#[derive(Debug)]
enum Expr {
    Num(f64),
    Name(String),
    List(Vec<Expr>),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(Box<Expr>, Op, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Index(Box<Expr>, Box<Expr>),
    Field(Box<Expr>, String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

/// Evaluates `source` for a property of kind `kind`.
pub fn eval(source: &str, env: &Env) -> Result<Value, String> {
    let ast = parse(source)?;
    let mut ctx = Ctx { env, random_calls: 0 };
    let v = ctx.eval(&ast, 0)?;
    to_value(v, env.property.kind())
}

/// Parses `source` without running it (for showing syntax errors while typing).
pub fn check(source: &str) -> Result<(), String> {
    parse(source).map(|_| ())
}

/// Whether expressions can drive a property of this kind.
pub fn supports(kind: ValueKind) -> bool {
    matches!(kind, ValueKind::Scalar | ValueKind::Vec2 | ValueKind::Color)
}

fn to_value(v: V, kind: ValueKind) -> Result<Value, String> {
    let bad =
        |want: &str, got: &V| Err(format!("this property needs {want}, but the expression gives {}", describe(got)));
    match (kind, v) {
        (ValueKind::Scalar, V::Num(x)) => finite(x).map(Value::Scalar),
        (ValueKind::Scalar, V::Vec(v)) if v.len() == 1 => finite(v[0]).map(Value::Scalar),
        (ValueKind::Vec2, V::Vec(v)) if v.len() == 2 => Ok(Value::Vec2(Vec2::new(finite(v[0])?, finite(v[1])?))),
        (ValueKind::Vec2, V::Num(x)) => Ok(Value::Vec2(Vec2::new(finite(x)?, finite(x)?))),
        (ValueKind::Color, V::Vec(v)) if v.len() == 3 || v.len() == 4 => {
            let c = |i: usize| finite(v.get(i).copied().unwrap_or(1.0)).map(|x| x.clamp(0.0, 1.0));
            Ok(Value::Color(Color::rgba(c(0)?, c(1)?, c(2)?, c(3)?)))
        }
        (ValueKind::Scalar, v) => bad("a number", &v),
        (ValueKind::Vec2, v) => bad("[x, y]", &v),
        (ValueKind::Color, v) => bad("[r, g, b] or [r, g, b, a] (0-1)", &v),
        _ => Err("expressions work on numbers, positions/sizes and colours".into()),
    }
}

fn finite(x: f64) -> Result<f64, String> {
    if x.is_finite() { Ok(x) } else { Err("the result isn't a finite number (division by zero?)".into()) }
}

fn describe(v: &V) -> String {
    match v {
        V::Num(_) => "a number".into(),
        V::Vec(v) => format!("a list of {}", v.len()),
    }
}

fn value_at(p: &AnyProperty, t: f64) -> V {
    match p {
        AnyProperty::Scalar(p) => V::Num(p.value_at(t)),
        AnyProperty::Vec2(p) => {
            let v = p.value_at(t);
            V::Vec(vec![v.x, v.y])
        }
        AnyProperty::Color(p) => {
            let c = p.value_at(t);
            V::Vec(vec![c.r, c.g, c.b, c.a])
        }
        _ => V::Num(0.0),
    }
}

fn key_range(p: &AnyProperty) -> Option<(f64, f64)> {
    let times = p.keyframe_times();
    Some((*times.first()?, *times.last()?)).filter(|(a, b)| b > a)
}

struct Ctx<'a> {
    env: &'a Env<'a>,
    random_calls: u64,
}

impl Ctx<'_> {
    fn eval(&mut self, e: &Expr, depth: usize) -> Result<V, String> {
        if depth > MAX_DEPTH {
            return Err("the expression is nested too deeply".into());
        }
        let d = depth + 1;
        Ok(match e {
            Expr::Num(x) => V::Num(*x),
            Expr::Name(n) => self.name(n)?,
            Expr::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for i in items {
                    match self.eval(i, d)? {
                        V::Num(x) => out.push(x),
                        V::Vec(v) => out.extend(v),
                    }
                }
                V::Vec(out)
            }
            Expr::Neg(a) => map(self.eval(a, d)?, |x| -x),
            Expr::Not(a) => V::Num(if truthy(&self.eval(a, d)?) { 0.0 } else { 1.0 }),
            Expr::Bin(a, op, b) => {
                let (a, b) = (self.eval(a, d)?, self.eval(b, d)?);
                binary(*op, a, b)?
            }
            Expr::Cond(c, a, b) => {
                if truthy(&self.eval(c, d)?) {
                    self.eval(a, d)?
                } else {
                    self.eval(b, d)?
                }
            }
            Expr::Index(a, i) => {
                let a = self.eval(a, d)?;
                let i = num(&self.eval(i, d)?, "an index")?;
                index(&a, i)?
            }
            Expr::Field(a, f) => {
                let i = match f.as_str() {
                    "x" | "r" => 0.0,
                    "y" | "g" => 1.0,
                    "z" | "b" => 2.0,
                    "w" | "a" => 3.0,
                    _ => return Err(format!("unknown field .{f} (use .x .y or .r .g .b .a)")),
                };
                index(&self.eval(a, d)?, i)?
            }
            Expr::Call(f, args) => {
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    vals.push(self.eval(a, d)?);
                }
                self.call(f, vals)?
            }
        })
    }

    fn name(&self, n: &str) -> Result<V, String> {
        let env = self.env;
        Ok(V::Num(match n {
            "time" | "t" => env.time,
            "fps" => env.fps,
            "frame" => (env.time * env.fps).round(),
            "duration" => env.duration,
            "pi" | "PI" => std::f64::consts::PI,
            "index" => env.index as f64,
            "value" => return Ok(value_at(env.property, env.time)),
            _ => return Err(format!("unknown name “{n}”")),
        }))
    }

    fn call(&mut self, f: &str, a: Vec<V>) -> Result<V, String> {
        let env = self.env;
        let n = a.len();
        let want = |k: usize| -> Result<(), String> {
            if n == k { Ok(()) } else { Err(format!("{f}() takes {k} argument{}", if k == 1 { "" } else { "s" })) }
        };
        let arg = |i: usize| num(&a[i], &format!("{f}() argument {}", i + 1));
        let unary = |g: fn(f64) -> f64| -> Result<V, String> {
            if n != 1 {
                return Err(format!("{f}() takes 1 argument"));
            }
            Ok(map(a[0].clone(), g))
        };
        Ok(match f {
            "sin" => unary(f64::sin)?,
            "cos" => unary(f64::cos)?,
            "tan" => unary(f64::tan)?,
            "asin" => unary(f64::asin)?,
            "acos" => unary(f64::acos)?,
            "atan" => unary(f64::atan)?,
            "abs" => unary(f64::abs)?,
            "sqrt" => unary(f64::sqrt)?,
            "exp" => unary(f64::exp)?,
            "ln" => unary(f64::ln)?,
            "log10" => unary(f64::log10)?,
            "floor" => unary(f64::floor)?,
            "ceil" => unary(f64::ceil)?,
            "round" => unary(f64::round)?,
            "sign" => unary(f64::signum)?,
            "fract" => unary(f64::fract)?,
            "degrees" => unary(f64::to_degrees)?,
            "radians" => unary(f64::to_radians)?,
            "atan2" => {
                want(2)?;
                V::Num(arg(0)?.atan2(arg(1)?))
            }
            "pow" => {
                want(2)?;
                let e = arg(1)?;
                map(a[0].clone(), |x| x.powf(e))
            }
            "min" | "max" => {
                if n < 2 {
                    return Err(format!("{f}() takes 2 or more arguments"));
                }
                let mut acc = a[0].clone();
                for b in &a[1..] {
                    acc = zip(acc, b.clone(), if f == "min" { f64::min } else { f64::max })?;
                }
                acc
            }
            "clamp" => {
                want(3)?;
                let (lo, hi) = (a[1].clone(), a[2].clone());
                zip(zip(a[0].clone(), lo, f64::max)?, hi, f64::min)?
            }
            "lerp" | "mix" => {
                want(3)?;
                let k = arg(2)?;
                zip(a[0].clone(), a[1].clone(), |x, y| x + (y - x) * k)?
            }
            "smoothstep" => {
                want(3)?;
                let (e0, e1) = (arg(0)?, arg(1)?);
                map(a[2].clone(), |x| {
                    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                })
            }
            "length" => {
                want(1)?;
                V::Num(match &a[0] {
                    V::Num(x) => x.abs(),
                    V::Vec(v) => v.iter().map(|x| x * x).sum::<f64>().sqrt(),
                })
            }
            "normalize" => {
                want(1)?;
                match &a[0] {
                    V::Num(x) => V::Num(x.signum()),
                    V::Vec(v) => {
                        let l = v.iter().map(|x| x * x).sum::<f64>().sqrt();
                        V::Vec(v.iter().map(|x| if l > 0.0 { x / l } else { 0.0 }).collect())
                    }
                }
            }
            "noise" => {
                want(1)?;
                let x = arg(0)?;
                let w = xenify_doc::Wiggle { frequency: 1.0, position: 1.0, rotation: 0.0, scale: 0.0, seed: env.seed };
                V::Num(w.noise(7, x))
            }
            "random" => {
                // A different number per call site order and frame, stable across renders.
                self.random_calls += 1;
                let frame = (env.time * env.fps).round() as i64;
                let r = hash01(env.seed, self.random_calls, frame);
                match n {
                    0 => V::Num(r),
                    1 => V::Num(r * arg(0)?),
                    2 => {
                        let (lo, hi) = (arg(0)?, arg(1)?);
                        V::Num(lo + r * (hi - lo))
                    }
                    _ => return Err("random() takes 0, 1 or 2 arguments".into()),
                }
            }
            "wiggle" => {
                want(2)?;
                let (freq, amount) = (arg(0)?, arg(1)?);
                let w = xenify_doc::Wiggle {
                    frequency: freq.clamp(0.0, 100.0),
                    position: 1.0,
                    rotation: 0.0,
                    scale: 0.0,
                    seed: env.seed,
                };
                match value_at(env.property, env.time) {
                    V::Num(x) => V::Num(x + w.noise(0, env.time) * amount),
                    V::Vec(v) => {
                        V::Vec(v.iter().enumerate().map(|(i, x)| x + w.noise(i as u64, env.time) * amount).collect())
                    }
                }
            }
            "value_at" => {
                want(1)?;
                value_at(env.property, arg(0)?)
            }
            "loop_out" | "loop_in" | "ping_pong" => {
                if n != 0 {
                    return Err(format!("{f}() takes no arguments"));
                }
                let Some((first, last)) = key_range(env.property) else {
                    return Ok(value_at(env.property, env.time));
                };
                let span = last - first;
                let t = env.time;
                let local = match f {
                    "loop_out" if t > last => first + (t - first).rem_euclid(span),
                    "loop_in" if t < first => last - (last - t).rem_euclid(span),
                    "ping_pong" if t > first => {
                        let k = (t - first).rem_euclid(2.0 * span);
                        first + if k <= span { k } else { 2.0 * span - k }
                    }
                    _ => t,
                };
                value_at(env.property, local)
            }
            _ => return Err(format!("unknown function {f}()")),
        })
    }
}

fn hash01(seed: u64, call: u64, frame: i64) -> f64 {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ call.wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (frame as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 31;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 29;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

fn truthy(v: &V) -> bool {
    match v {
        V::Num(x) => *x != 0.0,
        V::Vec(v) => v.iter().any(|x| *x != 0.0),
    }
}

fn num(v: &V, what: &str) -> Result<f64, String> {
    match v {
        V::Num(x) => Ok(*x),
        V::Vec(v) if v.len() == 1 => Ok(v[0]),
        V::Vec(_) => Err(format!("{what} must be a number, not a list")),
    }
}

fn index(v: &V, i: f64) -> Result<V, String> {
    let i = i.round();
    match v {
        V::Vec(v) if i >= 0.0 && (i as usize) < v.len() => Ok(V::Num(v[i as usize])),
        V::Vec(v) => Err(format!("index {i} is out of range (the list has {})", v.len())),
        V::Num(x) if i == 0.0 => Ok(V::Num(*x)),
        V::Num(_) => Err("a number has no parts to index".into()),
    }
}

fn map(v: V, f: impl Fn(f64) -> f64) -> V {
    match v {
        V::Num(x) => V::Num(f(x)),
        V::Vec(v) => V::Vec(v.into_iter().map(f).collect()),
    }
}

/// Element-wise, with numbers applying to every element.
fn zip(a: V, b: V, f: impl Fn(f64, f64) -> f64) -> Result<V, String> {
    Ok(match (a, b) {
        (V::Num(x), V::Num(y)) => V::Num(f(x, y)),
        (V::Num(x), V::Vec(v)) => V::Vec(v.into_iter().map(|y| f(x, y)).collect()),
        (V::Vec(v), V::Num(y)) => V::Vec(v.into_iter().map(|x| f(x, y)).collect()),
        (V::Vec(a), V::Vec(b)) => {
            if a.len() != b.len() {
                return Err(format!("can't combine lists of {} and {}", a.len(), b.len()));
            }
            V::Vec(a.into_iter().zip(b).map(|(x, y)| f(x, y)).collect())
        }
    })
}

fn binary(op: Op, a: V, b: V) -> Result<V, String> {
    let bool_num = |b: bool| if b { 1.0 } else { 0.0 };
    match op {
        Op::Add => zip(a, b, |x, y| x + y),
        Op::Sub => zip(a, b, |x, y| x - y),
        Op::Mul => zip(a, b, |x, y| x * y),
        Op::Div => zip(a, b, |x, y| x / y),
        Op::Rem => zip(a, b, |x, y| x.rem_euclid(y)),
        Op::And => Ok(V::Num(bool_num(truthy(&a) && truthy(&b)))),
        Op::Or => Ok(V::Num(bool_num(truthy(&a) || truthy(&b)))),
        _ => {
            let (x, y) = (num(&a, "a comparison")?, num(&b, "a comparison")?);
            Ok(V::Num(bool_num(match op {
                Op::Lt => x < y,
                Op::Le => x <= y,
                Op::Gt => x > y,
                Op::Ge => x >= y,
                Op::Eq => (x - y).abs() < 1e-9,
                _ => (x - y).abs() >= 1e-9,
            })))
        }
    }
}

// ---------------------------------------------------------------- parsing

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Sym(&'static str),
}

fn tokenize(s: &str) -> Result<Vec<Tok>, String> {
    const SYMS: [&str; 22] = [
        "<=", ">=", "==", "!=", "&&", "||", "+", "-", "*", "/", "%", "(", ")", "[", "]", ",", ".", "?", ":", "<", ">",
        "!",
    ];
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i] as char;
        if c.is_whitespace() {
            i += 1;
        } else if c == ';' && s[i + 1..].trim().is_empty() {
            // A trailing semicolon (habit from AE expressions) is fine.
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                i += 1;
                if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
                    i += 1;
                }
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text = &s[start..i];
            out.push(Tok::Num(text.parse().map_err(|_| format!("bad number “{text}”"))?));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push(Tok::Ident(s[start..i].to_string()));
        } else if let Some(sym) = SYMS.iter().find(|sym| s[i..].starts_with(**sym)) {
            out.push(Tok::Sym(sym));
            i += sym.len();
        } else {
            return Err(format!("unexpected “{c}”"));
        }
    }
    Ok(out)
}

fn parse(s: &str) -> Result<Expr, String> {
    if s.len() > MAX_LEN {
        return Err("the expression is too long".into());
    }
    let toks = tokenize(s)?;
    if toks.is_empty() {
        return Err("the expression is empty".into());
    }
    let mut p = Parser { toks, pos: 0, depth: 0 };
    let e = p.ternary()?;
    if p.pos < p.toks.len() {
        return Err(format!("unexpected {} after the end of the expression", p.show(p.pos)));
    }
    Ok(e)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn show(&self, i: usize) -> String {
        match self.toks.get(i) {
            Some(Tok::Num(x)) => format!("“{x}”"),
            Some(Tok::Ident(n)) => format!("“{n}”"),
            Some(Tok::Sym(s)) => format!("“{s}”"),
            None => "the end".into(),
        }
    }

    fn eat(&mut self, sym: &str) -> bool {
        self.eat_op(sym)
    }

    fn eat_op(&mut self, sym: &str) -> bool {
        if matches!(self.toks.get(self.pos), Some(Tok::Sym(s)) if *s == sym) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, sym: &str) -> Result<(), String> {
        if self.eat(sym) { Ok(()) } else { Err(format!("expected “{sym}” but found {}", self.show(self.pos))) }
    }

    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH { Err("the expression is nested too deeply".into()) } else { Ok(()) }
    }

    fn ternary(&mut self) -> Result<Expr, String> {
        self.enter()?;
        let c = self.or()?;
        let e = if self.eat("?") {
            let a = self.ternary()?;
            self.expect(":")?;
            let b = self.ternary()?;
            Expr::Cond(Box::new(c), Box::new(a), Box::new(b))
        } else {
            c
        };
        self.depth -= 1;
        Ok(e)
    }

    fn binary_level(
        &mut self,
        ops: &[(&str, Op)],
        next: fn(&mut Self) -> Result<Expr, String>,
    ) -> Result<Expr, String> {
        let mut a = next(self)?;
        'outer: loop {
            for (sym, op) in ops {
                if self.eat_op(sym) {
                    let b = next(self)?;
                    a = Expr::Bin(Box::new(a), *op, Box::new(b));
                    continue 'outer;
                }
            }
            return Ok(a);
        }
    }

    fn or(&mut self) -> Result<Expr, String> {
        self.binary_level(&[("||", Op::Or)], Self::and)
    }

    fn and(&mut self) -> Result<Expr, String> {
        self.binary_level(&[("&&", Op::And)], Self::compare)
    }

    fn compare(&mut self) -> Result<Expr, String> {
        self.binary_level(
            &[("<=", Op::Le), (">=", Op::Ge), ("==", Op::Eq), ("!=", Op::Ne), ("<", Op::Lt), (">", Op::Gt)],
            Self::add,
        )
    }

    fn add(&mut self) -> Result<Expr, String> {
        self.binary_level(&[("+", Op::Add), ("-", Op::Sub)], Self::mul)
    }

    fn mul(&mut self) -> Result<Expr, String> {
        self.binary_level(&[("*", Op::Mul), ("/", Op::Div), ("%", Op::Rem)], Self::unary)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        self.enter()?;
        let e = if self.eat_op("-") {
            Expr::Neg(Box::new(self.unary()?))
        } else if self.eat_op("+") {
            self.unary()?
        } else if self.eat_op("!") {
            Expr::Not(Box::new(self.unary()?))
        } else {
            self.postfix()?
        };
        self.depth -= 1;
        Ok(e)
    }

    fn postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.primary()?;
        loop {
            if self.eat("[") {
                let i = self.ternary()?;
                self.expect("]")?;
                e = Expr::Index(Box::new(e), Box::new(i));
            } else if self.eat(".") {
                match self.toks.get(self.pos).cloned() {
                    Some(Tok::Ident(f)) => {
                        self.pos += 1;
                        e = Expr::Field(Box::new(e), f);
                    }
                    _ => return Err("expected a field name after “.”".into()),
                }
            } else {
                return Ok(e);
            }
        }
    }

    fn primary(&mut self) -> Result<Expr, String> {
        match self.toks.get(self.pos).cloned() {
            Some(Tok::Num(x)) => {
                self.pos += 1;
                Ok(Expr::Num(x))
            }
            Some(Tok::Ident(name)) => {
                self.pos += 1;
                if self.eat("(") {
                    let mut args = Vec::new();
                    if !self.eat(")") {
                        loop {
                            args.push(self.ternary()?);
                            if self.eat(")") {
                                break;
                            }
                            self.expect(",")?;
                        }
                    }
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Name(name))
                }
            }
            Some(Tok::Sym("(")) => {
                self.pos += 1;
                let e = self.ternary()?;
                self.expect(")")?;
                Ok(e)
            }
            Some(Tok::Sym("[")) => {
                self.pos += 1;
                let mut items = Vec::new();
                if !self.eat("]") {
                    loop {
                        items.push(self.ternary()?);
                        if self.eat("]") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                Ok(Expr::List(items))
            }
            _ => Err(format!("expected a number, name or “(” but found {}", self.show(self.pos))),
        }
    }
}

// ---------------------------------------------------------------- baking

/// A copy of `comp` whose expression-driven properties hold their computed
/// value at `time` (so everything downstream — parenting, mattes, bounds —
/// sees the result). Expressions that fail leave the keyframed value.
pub fn bake(comp: &xenify_doc::Composition, time: f64) -> std::borrow::Cow<'_, xenify_doc::Composition> {
    if comp.layers.iter().all(|l| l.meta.expressions.is_empty()) {
        return std::borrow::Cow::Borrowed(comp);
    }
    let mut out = comp.clone();
    for (i, layer) in out.layers.iter_mut().enumerate() {
        for e in layer.meta.expressions.clone() {
            let Ok(prop) = layer.property(e.path) else { continue };
            let env = Env {
                time,
                fps: comp.settings.fps,
                duration: comp.settings.duration,
                index: i + 1,
                seed: layer.id.0,
                property: &prop,
            };
            if let Ok(v) = eval(&e.source, &env)
                && let Some(p) = constant(v)
            {
                let _ = layer.replace_property(e.path, p);
            }
        }
    }
    std::borrow::Cow::Owned(out)
}

/// The result of evaluating an expression on one layer property at one time
/// (the error explains what's wrong), for the properties panel.
pub fn preview(
    comp: &xenify_doc::Composition,
    layer: xenify_doc::Id,
    source: &str,
    path: xenify_doc::PropPath,
    time: f64,
) -> Result<Value, String> {
    let (i, l) = comp.layers.iter().enumerate().find(|(_, l)| l.id == layer).ok_or("no such layer")?;
    let prop = l.property(path).map_err(|e| e.to_string())?;
    if !supports(prop.kind()) {
        return Err("expressions work on numbers, positions/sizes and colours".into());
    }
    let env = Env {
        time,
        fps: comp.settings.fps,
        duration: comp.settings.duration,
        index: i + 1,
        seed: l.id.0,
        property: &prop,
    };
    eval(source, &env)
}

/// A copy of `comp` where expression-driven properties become one linear
/// keyframe per frame — for exports (e.g. Lottie) that can't run expressions.
pub fn keyframed(comp: &xenify_doc::Composition) -> std::borrow::Cow<'_, xenify_doc::Composition> {
    if comp.layers.iter().all(|l| l.meta.expressions.is_empty()) {
        return std::borrow::Cow::Borrowed(comp);
    }
    let s = &comp.settings;
    let frames = (s.duration * s.fps).ceil().max(0.0) as u64;
    let mut out = comp.clone();
    for (i, layer) in out.layers.iter_mut().enumerate() {
        for e in std::mem::take(&mut layer.meta.expressions) {
            let Ok(prop) = layer.property(e.path) else { continue };
            let mut samples = Vec::new();
            for f in 0..=frames {
                let time = f as f64 / s.fps;
                let env =
                    Env { time, fps: s.fps, duration: s.duration, index: i + 1, seed: layer.id.0, property: &prop };
                match eval(&e.source, &env) {
                    Ok(v) => samples.push((time, v)),
                    Err(_) => break,
                }
            }
            if let Some(p) = keys(samples) {
                let _ = layer.replace_property(e.path, p);
            }
        }
    }
    std::borrow::Cow::Owned(out)
}

fn keys(samples: Vec<(f64, Value)>) -> Option<AnyProperty> {
    use xenify_doc::Keyframe;
    fn build<T: xenify_doc::Animatable>(k: Vec<(f64, T)>) -> Option<Property<T>> {
        Property::from_keyframes(
            k.into_iter().map(|(time, value)| Keyframe { time, value, easing: xenify_doc::Easing::Linear }).collect(),
        )
    }
    Some(match samples.first()?.1 {
        Value::Scalar(_) => AnyProperty::Scalar(build(
            samples
                .into_iter()
                .filter_map(|(t, v)| if let Value::Scalar(x) = v { Some((t, x)) } else { None })
                .collect(),
        )?),
        Value::Vec2(_) => AnyProperty::Vec2(build(
            samples.into_iter().filter_map(|(t, v)| if let Value::Vec2(x) = v { Some((t, x)) } else { None }).collect(),
        )?),
        Value::Color(_) => AnyProperty::Color(build(
            samples
                .into_iter()
                .filter_map(|(t, v)| if let Value::Color(x) = v { Some((t, x)) } else { None })
                .collect(),
        )?),
        _ => return None,
    })
}

fn constant(v: Value) -> Option<AnyProperty> {
    Some(match v {
        Value::Scalar(x) => AnyProperty::Scalar(Property::Static(x)),
        Value::Vec2(x) => AnyProperty::Vec2(Property::Static(x)),
        Value::Color(x) => AnyProperty::Color(Property::Static(x)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenify_doc::{Easing, Keyframe};

    fn scalar_keys() -> AnyProperty {
        AnyProperty::Scalar(
            Property::from_keyframes(vec![
                Keyframe { time: 0.0, value: 0.0, easing: Easing::Linear },
                Keyframe { time: 1.0, value: 100.0, easing: Easing::Linear },
            ])
            .unwrap(),
        )
    }

    fn run(src: &str, prop: &AnyProperty, time: f64) -> Result<Value, String> {
        eval(src, &Env { time, fps: 30.0, duration: 5.0, index: 1, seed: 7, property: prop })
    }

    #[test]
    fn arithmetic_names_and_precedence() {
        let p = AnyProperty::Scalar(Property::Static(10.0));
        assert_eq!(run("1 + 2 * 3", &p, 0.0), Ok(Value::Scalar(7.0)));
        assert_eq!(run("(1 + 2) * 3", &p, 0.0), Ok(Value::Scalar(9.0)));
        assert_eq!(run("value * 2 + time", &p, 1.5), Ok(Value::Scalar(21.5)));
        assert!(run("-2 ^ 2", &p, 0.0).is_err(), "no ^ operator");
        assert_eq!(run("time > 1 ? 5 : 3", &p, 2.0), Ok(Value::Scalar(5.0)));
        assert_eq!(run("max(1, 7, 3) + min(2, 4)", &p, 0.0), Ok(Value::Scalar(9.0)));
        assert_eq!(run("clamp(15, 0, 10);", &p, 0.0), Ok(Value::Scalar(10.0)));
        assert_eq!(run("frame", &p, 1.0), Ok(Value::Scalar(30.0)));
    }

    #[test]
    fn vectors_fields_and_kinds() {
        let p = AnyProperty::Vec2(Property::Static(Vec2::new(100.0, 200.0)));
        assert_eq!(run("value + [10, 20]", &p, 0.0), Ok(Value::Vec2(Vec2::new(110.0, 220.0))));
        assert_eq!(run("[value.x, value[1] * 2]", &p, 0.0), Ok(Value::Vec2(Vec2::new(100.0, 400.0))));
        assert_eq!(run("value * 0.5", &p, 0.0), Ok(Value::Vec2(Vec2::new(50.0, 100.0))));
        assert!(run("[1, 2, 3]", &p, 0.0).unwrap_err().contains("[x, y]"));
        let c = AnyProperty::Color(Property::Static(Color::WHITE));
        assert_eq!(run("[1, 0, 0]", &c, 0.0), Ok(Value::Color(Color::rgba(1.0, 0.0, 0.0, 1.0))));
    }

    #[test]
    fn loops_value_at_and_wiggle() {
        let p = scalar_keys();
        assert_eq!(run("value_at(0.5)", &p, 3.0), Ok(Value::Scalar(50.0)));
        assert_eq!(run("loop_out()", &p, 2.25), Ok(Value::Scalar(25.0)));
        assert_eq!(run("ping_pong()", &p, 1.75), Ok(Value::Scalar(25.0)));
        // Wiggle drifts around the keyframed value, deterministically.
        let a = run("wiggle(2, 10)", &p, 0.3).unwrap();
        assert_eq!(a, run("wiggle(2, 10)", &p, 0.3).unwrap());
        let Value::Scalar(x) = a else { panic!() };
        assert!((x - 30.0).abs() <= 10.0 && (x - 30.0).abs() > 1e-6);
        // random() is stable per frame and changes between frames.
        let r = |t| run("random(100)", &p, t).unwrap();
        assert_eq!(r(1.0), r(1.0));
        assert_ne!(r(1.0), r(1.0 + 1.0 / 30.0));
    }

    #[test]
    fn errors_are_readable_and_nothing_hangs() {
        let p = AnyProperty::Scalar(Property::Static(0.0));
        assert!(run("foo + 1", &p, 0.0).unwrap_err().contains("unknown name"));
        assert!(run("sin(1, 2)", &p, 0.0).unwrap_err().contains("1 argument"));
        assert!(run("1 +", &p, 0.0).is_err());
        assert!(run("1 / 0", &p, 0.0).unwrap_err().contains("finite"));
        assert!(run(&"(".repeat(200), &p, 0.0).unwrap_err().contains("nested"));
        assert!(check("value + wiggle(2, 5)").is_ok());
    }
}
