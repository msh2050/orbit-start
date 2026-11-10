//! Small recursive-descent expression evaluator used by the search box and
//! the calculator widget.

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i] == b' ' || self.s[self.i] == b',') {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        loop {
            if self.eat(b'+') {
                v += self.term()?;
            } else if self.eat(b'-') {
                v -= self.term()?;
            } else {
                return Some(v);
            }
        }
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        loop {
            if self.eat(b'*') || self.eat(b'x') {
                v *= self.unary()?;
            } else if self.eat(b'/') {
                v /= self.unary()?;
            } else if self.eat(b'%') {
                // "50%" is a percentage, "7 % 3" is a remainder.
                match self.peek() {
                    Some(c) if c.is_ascii_digit() || c == b'(' || c == b'.' => v %= self.unary()?,
                    _ => v /= 100.0,
                }
            } else {
                return Some(v);
            }
        }
    }
    fn unary(&mut self) -> Option<f64> {
        if self.eat(b'-') {
            return Some(-self.unary()?);
        }
        if self.eat(b'+') {
            return self.unary();
        }
        let base = self.atom()?;
        if self.eat(b'^') {
            return Some(base.powf(self.unary()?));
        }
        Some(base)
    }
    fn atom(&mut self) -> Option<f64> {
        let c = self.peek()?;
        if c == b'(' {
            self.i += 1;
            let v = self.expr()?;
            if !self.eat(b')') {
                return None;
            }
            return Some(v);
        }
        if c.is_ascii_digit() || c == b'.' {
            let start = self.i;
            while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.') {
                self.i += 1;
            }
            return std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok();
        }
        if c.is_ascii_alphabetic() {
            let start = self.i;
            while self.i < self.s.len() && self.s[self.i].is_ascii_alphanumeric() {
                self.i += 1;
            }
            let name = std::str::from_utf8(&self.s[start..self.i]).ok()?.to_ascii_lowercase();
            match name.as_str() {
                "pi" => return Some(std::f64::consts::PI),
                "e" => return Some(std::f64::consts::E),
                _ => {}
            }
            if !self.eat(b'(') {
                return None;
            }
            let a = self.expr()?;
            if !self.eat(b')') {
                return None;
            }
            return Some(match name.as_str() {
                "sqrt" => a.sqrt(),
                "sin" => a.sin(),
                "cos" => a.cos(),
                "tan" => a.tan(),
                "ln" => a.ln(),
                "log" => a.log10(),
                "abs" => a.abs(),
                "round" => a.round(),
                "floor" => a.floor(),
                "ceil" => a.ceil(),
                _ => return None,
            });
        }
        None
    }
}

pub fn eval(s: &str) -> Option<f64> {
    let cleaned = s.replace('×', "*").replace('÷', "/");
    let mut p = P { s: cleaned.as_bytes(), i: 0 };
    let v = p.expr()?;
    if p.peek().is_some() {
        return None;
    }
    Some(v)
}

/// True when the query is an actual calculation rather than a plain number or word.
pub fn is_math(s: &str) -> bool {
    let has_digit = s.bytes().any(|b| b.is_ascii_digit());
    let has_op = s.bytes().any(|b| b"+-*/^%(".contains(&b)) || s.contains("pi");
    has_digit && has_op
}

pub fn fmt(v: f64) -> String {
    if v.is_nan() {
        return "undefined".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "∞".into() } else { "-∞".into() };
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        let n = v as i64;
        let digits = n.abs().to_string();
        let mut out = String::new();
        for (i, ch) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(ch);
        }
        return if n < 0 { format!("-{out}") } else { out };
    }
    let s = format!("{:.10}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        assert_eq!(eval("2+3*4"), Some(14.0));
        assert_eq!(eval("(2+3)*4"), Some(20.0));
        assert_eq!(eval("2^3^2"), Some(512.0));
        assert_eq!(eval("-2^2"), Some(-4.0));
        assert_eq!(eval("7 % 3"), Some(1.0));
        assert_eq!(eval("200*15%"), Some(30.0));
        assert_eq!(eval("sqrt(16)+abs(-2)"), Some(6.0));
        assert_eq!(eval("2+"), None);
        assert_eq!(eval("hello"), None);
    }

    #[test]
    fn formatting() {
        assert_eq!(fmt(1234567.0), "1,234,567");
        assert_eq!(fmt(-1000.0), "-1,000");
        assert_eq!(fmt(0.5), "0.5");
        assert_eq!(fmt(1.0 / 3.0), "0.3333333333");
    }
}
