//! Instant calculator shown at the top of the results while typing.
//!
//! A small recursive-descent parser: `+ - * / % ^`, parentheses, implicit
//! multiplication (`2pi`, `3(4+1)`), common functions and constants.

use std::f64::consts;

/// Evaluates `input` if it looks like arithmetic. Returns `None` for anything
/// else so plain searches ("firefox", "42") never show a useless result.
pub fn evaluate(input: &str) -> Option<f64> {
    let input = input.trim();
    if !looks_like_math(input) {
        return None;
    }
    let mut parser = Parser { tokens: tokenize(input)?, pos: 0 };
    let value = parser.expr()?;
    (parser.pos == parser.tokens.len() && value.is_finite()).then_some(value)
}

/// Formats a result without float noise (`0.1 + 0.2` → `0.3`).
pub fn format(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let rounded = format!("{:.10}", value);
    rounded.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn looks_like_math(input: &str) -> bool {
    let has_operator = input.chars().any(|c| "+-*/%^(×÷".contains(c))
        || FUNCTIONS.iter().any(|(name, _)| input.contains(&format!("{name}(")));
    let has_digit = input.chars().any(|c| c.is_ascii_digit());
    has_operator && (has_digit || input.contains("pi")) && input.len() < 200
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    Ident(String),
    Op(char),
}

fn tokenize(input: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() || c == '_' {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' || c == ',' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == ',') {
                i += 1;
            }
            // Accept the French decimal comma: "3,5 * 2".
            let text: String = chars[start..i].iter().map(|&c| if c == ',' { '.' } else { c }).collect();
            tokens.push(Token::Num(text.parse().ok()?));
        } else if c.is_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_alphanumeric() {
                i += 1;
            }
            tokens.push(Token::Ident(chars[start..i].iter().collect::<String>().to_lowercase()));
        } else if "+-*/%^()×÷".contains(c) {
            tokens.push(Token::Op(match c {
                '×' => '*',
                '÷' => '/',
                c => c,
            }));
            i += 1;
        } else {
            return None;
        }
    }
    Some(tokens)
}

type Function = (&'static str, fn(f64) -> f64);

const FUNCTIONS: &[Function] = &[
    ("sqrt", f64::sqrt),
    ("abs", f64::abs),
    ("round", f64::round),
    ("floor", f64::floor),
    ("ceil", f64::ceil),
    ("ln", f64::ln),
    ("log", f64::log10),
    ("sin", f64::sin),
    ("cos", f64::cos),
    ("tan", f64::tan),
];

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
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

    // expr := term (('+' | '-') term)*
    fn expr(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        loop {
            if self.eat('+') {
                value += self.term()?;
            } else if self.eat('-') {
                value -= self.term()?;
            } else {
                return Some(value);
            }
        }
    }

    // term := unary (('*' | '/' | '%' | implicit) unary)*
    fn term(&mut self) -> Option<f64> {
        let mut value = self.unary()?;
        loop {
            if self.eat('*') {
                value *= self.unary()?;
            } else if self.eat('/') {
                value /= self.unary()?;
            } else if self.eat('%') {
                value %= self.unary()?;
            } else if matches!(self.peek(), Some(Token::Num(_) | Token::Ident(_) | Token::Op('('))) {
                value *= self.unary()?;
            } else {
                return Some(value);
            }
        }
    }

    // unary := '-' unary | power
    fn unary(&mut self) -> Option<f64> {
        if self.eat('-') {
            return Some(-self.unary()?);
        }
        if self.eat('+') {
            return self.unary();
        }
        self.power()
    }

    // power := atom ('^' unary)?   (right associative)
    fn power(&mut self) -> Option<f64> {
        let base = self.atom()?;
        if self.eat('^') {
            return Some(base.powf(self.unary()?));
        }
        Some(base)
    }

    fn atom(&mut self) -> Option<f64> {
        match self.tokens.get(self.pos)?.clone() {
            Token::Num(n) => {
                self.pos += 1;
                Some(n)
            }
            Token::Op('(') => {
                self.pos += 1;
                let value = self.expr()?;
                // Tolerate a missing closing parenthesis at the end while typing.
                if !self.eat(')') && self.pos != self.tokens.len() {
                    return None;
                }
                Some(value)
            }
            Token::Ident(name) => {
                self.pos += 1;
                match name.as_str() {
                    "pi" | "π" => return Some(consts::PI),
                    "e" => return Some(consts::E),
                    "tau" => return Some(consts::TAU),
                    _ => {}
                }
                let (_, f) = FUNCTIONS.iter().find(|(n, _)| *n == name)?;
                if !matches!(self.peek(), Some(Token::Op('('))) {
                    return None;
                }
                Some(f(self.atom()?))
            }
            Token::Op(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calc(s: &str) -> Option<String> {
        evaluate(s).map(format)
    }

    #[test]
    fn arithmetic() {
        assert_eq!(calc("1 + 2 * 3").as_deref(), Some("7"));
        assert_eq!(calc("(1 + 2) * 3").as_deref(), Some("9"));
        assert_eq!(calc("2 ^ 3 ^ 2").as_deref(), Some("512"));
        assert_eq!(calc("-2^2").as_deref(), Some("-4"));
        assert_eq!(calc("10 % 4").as_deref(), Some("2"));
        assert_eq!(calc("0.1 + 0.2").as_deref(), Some("0.3"));
        assert_eq!(calc("3,5 * 2").as_deref(), Some("7"));
        assert_eq!(calc("7 ÷ 2").as_deref(), Some("3.5"));
    }

    #[test]
    fn functions_constants_and_implicit_multiplication() {
        assert_eq!(calc("sqrt(16) + 1").as_deref(), Some("5"));
        assert_eq!(calc("2pi + 0").as_deref(), Some("6.2831853072"));
        assert_eq!(calc("3(4+1)").as_deref(), Some("15"));
        assert_eq!(calc("(2+3").as_deref(), Some("5"));
    }

    #[test]
    fn ignores_non_math() {
        for s in ["firefox", "42", "", "1 / 0", "foo(2)", "2 +", "rm -rf", "v1.2-beta"] {
            assert_eq!(calc(s), None, "{s:?}");
        }
    }
}
