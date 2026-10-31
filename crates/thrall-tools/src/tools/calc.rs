//! `calc`: exact arithmetic without the shell. Lines of expressions, optionally named
//! (`stack = 2 * 1.25`) so later lines can use them. No variables survive the call, nothing runs.

use std::collections::BTreeMap;

use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::constants::{
    CALC_MAX_DEPTH, CALC_MAX_INPUT_CHARS, CALC_MAX_STATEMENTS, CALC_SIGNIFICANT_DIGITS,
};
use crate::tool::{Tool, ToolContext, ToolOutput, arg_str};

/// Work out arithmetic.
pub struct Calc;

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    Name(String),
    Op(char),
}

fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == '_')
            {
                i += 1;
            }
            // Exponent: 1e5, 2.5E-3.
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    i = j;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let raw: String = chars[start..i].iter().filter(|c| **c != '_').collect();
            let n = raw
                .parse::<f64>()
                .map_err(|_| format!("'{raw}' isn't a number"))?;
            out.push(Token::Num(n));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Token::Name(chars[start..i].iter().collect()));
        } else if "+-*/%^(),=".contains(c) {
            out.push(Token::Op(c));
            i += 1;
        } else if c == '×' {
            out.push(Token::Op('*'));
            i += 1;
        } else if c == '÷' {
            out.push(Token::Op('/'));
            i += 1;
        } else {
            return Err(format!("'{c}' isn't something calc understands"));
        }
    }
    Ok(out)
}

/// One expression, read left to right with the usual precedence: ^ (right to left), then
/// unary minus, then * / %, then + -.
struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    names: &'a BTreeMap<String, f64>,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn eat(&mut self, op: char) -> bool {
        if self.peek() == Some(&Token::Op(op)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn deeper(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > CALC_MAX_DEPTH {
            return Err("too deeply nested".to_owned());
        }
        Ok(())
    }

    fn expr(&mut self) -> Result<f64, String> {
        self.deeper()?;
        let mut v = self.term()?;
        loop {
            if self.eat('+') {
                v += self.term()?;
            } else if self.eat('-') {
                v -= self.term()?;
            } else {
                break;
            }
        }
        self.depth -= 1;
        Ok(v)
    }

    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.unary()?;
        loop {
            if self.eat('*') {
                v *= self.unary()?;
            } else if self.eat('/') {
                let d = self.unary()?;
                if d == 0.0 {
                    return Err("division by zero".to_owned());
                }
                v /= d;
            } else if self.eat('%') {
                let d = self.unary()?;
                if d == 0.0 {
                    return Err("remainder of division by zero".to_owned());
                }
                v %= d;
            } else {
                break;
            }
        }
        Ok(v)
    }

    fn unary(&mut self) -> Result<f64, String> {
        self.deeper()?;
        let v = if self.eat('-') {
            -self.unary()?
        } else if self.eat('+') {
            self.unary()?
        } else {
            self.power()?
        };
        self.depth -= 1;
        Ok(v)
    }

    fn power(&mut self) -> Result<f64, String> {
        let base = self.primary()?;
        if self.eat('^') {
            let exp = self.unary()?;
            return Ok(base.powf(exp));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<f64, String> {
        match self.peek().cloned() {
            Some(Token::Num(n)) => {
                self.pos += 1;
                Ok(n)
            }
            Some(Token::Op('(')) => {
                self.pos += 1;
                let v = self.expr()?;
                if !self.eat(')') {
                    return Err("a '(' isn't closed".to_owned());
                }
                Ok(v)
            }
            Some(Token::Name(name)) => {
                self.pos += 1;
                if self.eat('(') {
                    let mut args = Vec::new();
                    if !self.eat(')') {
                        loop {
                            args.push(self.expr()?);
                            if self.eat(',') {
                                continue;
                            }
                            if self.eat(')') {
                                break;
                            }
                            return Err(format!("expected ',' or ')' in {name}(...)"));
                        }
                    }
                    return call(&name, &args);
                }
                match name.to_lowercase().as_str() {
                    "pi" => Ok(std::f64::consts::PI),
                    "e" => Ok(std::f64::consts::E),
                    _ => self
                        .names
                        .get(&name)
                        .copied()
                        .ok_or_else(|| format!("'{name}' hasn't been given a value")),
                }
            }
            Some(Token::Op(c)) => Err(format!("unexpected '{c}'")),
            None => Err("the expression ends too early".to_owned()),
        }
    }
}

fn want(name: &str, args: &[f64], n: usize) -> Result<(), String> {
    if args.len() == n {
        Ok(())
    } else {
        Err(format!("{name} takes {n} value(s), got {}", args.len()))
    }
}

fn call(name: &str, args: &[f64]) -> Result<f64, String> {
    let lower = name.to_lowercase();
    let one = |f: fn(f64) -> f64| -> Result<f64, String> {
        want(&lower, args, 1)?;
        Ok(f(args[0]))
    };
    match lower.as_str() {
        "abs" => one(f64::abs),
        "sqrt" => one(f64::sqrt),
        "floor" => one(f64::floor),
        "ceil" => one(f64::ceil),
        "ln" => one(f64::ln),
        "log10" => one(f64::log10),
        "exp" => one(f64::exp),
        "round" => match args.len() {
            1 => Ok(args[0].round()),
            2 => {
                let scale = 10f64.powf(args[1].trunc());
                Ok((args[0] * scale).round() / scale)
            }
            n => Err(format!("round takes 1 or 2 values, got {n}")),
        },
        "pow" => {
            want(&lower, args, 2)?;
            Ok(args[0].powf(args[1]))
        }
        "log" => {
            want(&lower, args, 2)?;
            Ok(args[0].ln() / args[1].ln())
        }
        "min" | "max" | "sum" | "product" | "avg" if args.is_empty() => {
            Err(format!("{lower} needs at least one value"))
        }
        "min" => Ok(args.iter().copied().fold(f64::INFINITY, f64::min)),
        "max" => Ok(args.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
        "sum" => Ok(args.iter().sum()),
        "product" => Ok(args.iter().product()),
        #[allow(clippy::cast_precision_loss)]
        "avg" => Ok(args.iter().sum::<f64>() / args.len() as f64),
        _ => Err(format!(
            "no function '{name}' (have: abs, sqrt, round, floor, ceil, min, max, sum, product, \
             avg, pow, log, ln, log10, exp)"
        )),
    }
}

/// A number as people write it: up to the set significant digits, no trailing zeros.
fn shown(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{v:.0}");
    }
    let digits = CALC_SIGNIFICANT_DIGITS as i32;
    #[allow(clippy::cast_possible_truncation)]
    let magnitude = v.abs().log10().floor() as i32;
    let decimals = (digits - 1 - magnitude).clamp(0, 15);
    #[allow(clippy::cast_sign_loss)]
    let text = format!("{v:.prec$}", prec = decimals as usize);
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

/// Work through the lines; each answer is `line = value`. Stops at the first mistake.
pub fn evaluate(input: &str) -> Result<String, String> {
    if input.chars().count() > CALC_MAX_INPUT_CHARS {
        return Err(format!("more than {CALC_MAX_INPUT_CHARS} characters"));
    }
    let statements: Vec<&str> = input
        .split(['\n', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .collect();
    if statements.is_empty() {
        return Err("nothing to work out".to_owned());
    }
    if statements.len() > CALC_MAX_STATEMENTS {
        return Err(format!("more than {CALC_MAX_STATEMENTS} lines"));
    }
    let mut names: BTreeMap<String, f64> = BTreeMap::new();
    let mut out = String::new();
    for line in statements {
        let toks = tokens(line).map_err(|e| format!("{line}: {e}"))?;
        let (target, body) = match toks.as_slice() {
            [Token::Name(n), Token::Op('='), rest @ ..] => (Some(n.clone()), rest),
            _ => (None, toks.as_slice()),
        };
        let mut p = Parser {
            tokens: body,
            pos: 0,
            names: &names,
            depth: 0,
        };
        let v = p.expr().map_err(|e| format!("{line}: {e}"))?;
        if p.pos != body.len() {
            return Err(format!("{line}: unexpected text after the expression"));
        }
        if !v.is_finite() {
            return Err(format!("{line}: the result isn't a finite number"));
        }
        out.push_str(&format!("{line} = {}\n", shown(v)));
        if let Some(n) = target {
            names.insert(n, v);
        }
    }
    Ok(out)
}

impl Tool for Calc {
    fn name(&self) -> &'static str {
        "calc"
    }
    fn description(&self) -> &'static str {
        "Exact arithmetic - use this for every sum, product, percentage or conversion instead of \
         working numbers out in your head or in the shell. One expression per line (or separated \
         by ';'); name results to reuse them: \"stack = 2 * 1.25 * 1.15\" then \"stack * 1.5\". \
         Operators + - * / % ^ and brackets; functions abs, sqrt, round(x, digits), floor, ceil, \
         min, max, sum, product, avg, pow, log(x, base), ln, log10, exp; pi and e. Every line's \
         answer comes back."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "expression": { "type": "string" } },
            "required": ["expression"]
        })
    }
    fn title(&self, args: &Value) -> String {
        let first = arg_str(args, "expression")
            .lines()
            .next()
            .unwrap_or_default();
        format!("Calculating {first}")
    }
    fn run<'a>(&'a self, _ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            match evaluate(arg_str(&args, "expression")) {
                Ok(text) => {
                    let last = text.lines().last().unwrap_or_default().to_owned();
                    ToolOutput::ok(text, last)
                }
                Err(e) => ToolOutput::error(e),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::evaluate;

    fn last(input: &str) -> String {
        let out = evaluate(input).unwrap();
        out.lines()
            .last()
            .unwrap()
            .rsplit(" = ")
            .next()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn precedence_and_powers() {
        assert_eq!(last("2 + 3 * 4"), "14");
        assert_eq!(last("(2 + 3) * 4"), "20");
        assert_eq!(last("-2 ^ 2"), "-4");
        assert_eq!(last("2 ^ 3 ^ 2"), "512");
        assert_eq!(last("10 % 4"), "2");
        assert_eq!(last("1e3 / 8"), "125");
    }

    #[test]
    fn named_lines_carry_forward() {
        // The harvest stack Thrall once multiplied in its head and got wrong (6.36).
        let out = evaluate(
            "confirmed = 2.0 * 1.25 * 1.33 * 1.15 * 1.15 * 1.10 * 1.15\n\
             round(confirmed * 1.05, 2)\nround(confirmed * 1.25, 2)",
        )
        .unwrap();
        assert!(out.contains("round(confirmed * 1.05, 2) = 5.84"), "{out}");
        assert!(out.contains("round(confirmed * 1.25, 2) = 6.95"), "{out}");
    }

    #[test]
    fn functions() {
        assert_eq!(last("product(2, 1.25, 1.15)"), "2.875");
        assert_eq!(last("max(1, 7, 3) + min(4, -1)"), "6");
        assert_eq!(last("round(2 / 3, 3)"), "0.667");
        assert_eq!(last("log(8, 2)"), "3");
        assert_eq!(last("0.1 + 0.2"), "0.3");
    }

    #[test]
    fn mistakes_are_explained() {
        assert!(evaluate("1 / 0").unwrap_err().contains("division by zero"));
        assert!(
            evaluate("x + 1")
                .unwrap_err()
                .contains("hasn't been given a value")
        );
        assert!(evaluate("2 +").unwrap_err().contains("ends too early"));
        assert!(evaluate("(1 + 2").unwrap_err().contains("isn't closed"));
        assert!(evaluate("2 $ 3").unwrap_err().contains("isn't something"));
        assert!(evaluate("nope(1)").unwrap_err().contains("no function"));
        assert!(evaluate("").is_err());
    }
}
