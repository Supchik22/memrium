//! What a piece of copied text is, for the clipboard history (Super+V): a link, an e-mail address, a phone number, a
//! colour, a sum, a path, a block of code; each kind with the one thing one most often wants to do with it.

/// The kind of a copied text, and what it holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Link(String),
    Email(String),
    Phone(String),
    /// the colour (0..1) and how it was written
    Colour([f32; 3], String),
    /// an arithmetic expression and its value
    Sum(f64),
    Path(String),
    /// several lines with indentation or trailing spaces (code, a pasted table)
    Block,
    Plain,
}

/// One action for the clipboard's row: its label and what to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Open(String),
    Copy(String),
}

impl Kind {
    /// A short name for the row's badge.
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Link(_) => "Link",
            Kind::Email(_) => "E-mail",
            Kind::Phone(_) => "Phone",
            Kind::Colour(..) => "Colour",
            Kind::Sum(_) => "Sum",
            Kind::Path(_) => "Path",
            Kind::Block => "Text block",
            Kind::Plain => "",
        }
    }

    /// The Lucide icon of the badge.
    pub fn icon(&self) -> &'static str {
        match self {
            Kind::Link(_) => "link",
            Kind::Email(_) => "mail",
            Kind::Phone(_) => "smartphone",
            Kind::Colour(..) => "palette",
            Kind::Sum(_) => "equal",
            Kind::Path(_) => "folder",
            Kind::Block => "code",
            Kind::Plain => "type",
        }
    }

    /// The action offered for it: (label, what it does).
    pub fn action(&self, text: &str) -> Option<(String, Action)> {
        Some(match self {
            Kind::Link(u) => ("Open".into(), Action::Open(u.clone())),
            Kind::Email(m) => ("Write".into(), Action::Open(format!("mailto:{m}"))),
            Kind::Phone(p) => ("Copy digits".into(), Action::Copy(p.clone())),
            Kind::Colour(c, written) => {
                let hex = format!("#{:02x}{:02x}{:02x}", (c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8);
                if written.starts_with('#') {
                    let rgb = format!("rgb({}, {}, {})", (c[0] * 255.0).round(), (c[1] * 255.0).round(), (c[2] * 255.0).round());
                    ("Copy as rgb()".into(), Action::Copy(rgb))
                } else {
                    ("Copy as hex".into(), Action::Copy(hex))
                }
            }
            Kind::Sum(v) => (format!("= {}   ·   Copy", fmt_value(*v)), Action::Copy(fmt_value(*v))),
            Kind::Path(p) => {
                let dir = std::path::Path::new(p).parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_else(|| p.clone());
                ("Show in folder".into(), Action::Open(dir))
            }
            Kind::Block => ("Copy as plain".into(), Action::Copy(plain(text))),
            Kind::Plain => return None,
        })
    }
}

fn fmt_value(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 && v.abs() < 1e15 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.6}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// What kind of text this is.
pub fn classify(text: &str) -> Kind {
    let t = text.trim();
    if t.is_empty() || t.len() > 4000 {
        return Kind::Plain;
    }
    let one_line = !t.contains('\n');
    if one_line && !t.contains(' ') {
        if (t.starts_with("http://") || t.starts_with("https://") || t.starts_with("www.")) && t.len() > 7 {
            let u = if t.starts_with("www.") { format!("https://{t}") } else { t.to_string() };
            return Kind::Link(u);
        }
        if let Some(at) = t.find('@') {
            let (user, host) = (&t[..at], &t[at + 1..]);
            if !user.is_empty() && host.contains('.') && !host.starts_with('.') && !host.ends_with('.') && !host.contains('@') {
                return Kind::Email(t.to_string());
            }
        }
        if t.starts_with('/') || t.starts_with("~/") {
            return Kind::Path(t.to_string());
        }
    }
    if one_line {
        if let Some(c) = colour(t) {
            return Kind::Colour(c, t.to_string());
        }
        if let Some(p) = phone(t) {
            return Kind::Phone(p);
        }
        if let Some(v) = sum(t) {
            return Kind::Sum(v);
        }
    }
    // several lines with indentation or trailing spaces: code, or a table from a web page
    if !one_line && t.lines().count() >= 2 && text.lines().any(|l| l.starts_with("  ") || l.starts_with('\t') || l.ends_with(' ')) {
        return Kind::Block;
    }
    Kind::Plain
}

/// `#rgb`, `#rrggbb`, `rgb(r, g, b)`.
fn colour(t: &str) -> Option<[f32; 3]> {
    let t = t.trim_end_matches(';');
    if let Some(h) = t.strip_prefix('#') {
        let n = |s: &str| u8::from_str_radix(s, 16).ok().map(|v| v as f32 / 255.0);
        return match h.len() {
            3 => {
                let d: Vec<f32> = h.chars().map(|c| u8::from_str_radix(&c.to_string(), 16).ok().map(|v| (v * 17) as f32 / 255.0)).collect::<Option<_>>()?;
                Some([d[0], d[1], d[2]])
            }
            6 => Some([n(&h[0..2])?, n(&h[2..4])?, n(&h[4..6])?]),
            _ => None,
        };
    }
    let inner = t.strip_prefix("rgb(").or_else(|| t.strip_prefix("rgba("))?.strip_suffix(')')?;
    let v: Vec<f32> = inner.split(',').take(3).map(|x| x.trim().parse::<f32>().ok().map(|v| (v / 255.0).clamp(0.0, 1.0))).collect::<Option<_>>()?;
    (v.len() == 3).then(|| [v[0], v[1], v[2]])
}

/// A phone number: digits with the usual separators, 9 to 15 digits; the digits (with a leading +) are returned.
fn phone(t: &str) -> Option<String> {
    if !t.chars().all(|c| c.is_ascii_digit() || " +-()".contains(c)) {
        return None;
    }
    let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
    if !(9..=15).contains(&digits.len()) || !t.chars().any(|c| " -()+".contains(c)) {
        return None;
    }
    Some(if t.trim_start().starts_with('+') { format!("+{digits}") } else { digits })
}

/// An arithmetic expression with at least one operator: + - * / ^ %, brackets, decimals (a comma counts as a point).
pub fn sum(t: &str) -> Option<f64> {
    let s: Vec<char> = t.chars().filter(|c| !c.is_whitespace()).map(|c| match c {
        ',' => '.',
        '×' | 'x' | 'X' => '*',
        '÷' | ':' => '/',
        c => c,
    }).collect();
    if s.is_empty() || !s.iter().all(|c| c.is_ascii_digit() || "+-*/^%().".contains(*c)) {
        return None;
    }
    // something to work out, not just a number (or a date like 2026-11-06)
    let ops = s.iter().skip(1).filter(|c| "+-*/^%".contains(**c)).count();
    if ops == 0 || (s.iter().filter(|c| **c == '-').count() == ops && s.iter().all(|c| c.is_ascii_digit() || *c == '-')) {
        return None;
    }
    let mut p = Parser { s: &s, i: 0 };
    let v = p.expr()?;
    (p.i == s.len() && v.is_finite()).then_some(v)
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(c) = self.peek().filter(|c| *c == '+' || *c == '-') {
            self.i += 1;
            let r = self.term()?;
            v = if c == '+' { v + r } else { v - r };
        }
        Some(v)
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.power()?;
        while let Some(c) = self.peek().filter(|c| *c == '*' || *c == '/' || *c == '%') {
            self.i += 1;
            if c == '%' && !self.peek().is_some_and(|n| n.is_ascii_digit() || n == '(') {
                // 50% = 0.5
                v /= 100.0;
                continue;
            }
            let r = self.power()?;
            v = match c {
                '*' => v * r,
                '/' => v / r,
                _ => v % r,
            };
        }
        Some(v)
    }
    fn power(&mut self) -> Option<f64> {
        let b = self.unary()?;
        if self.peek() == Some('^') {
            self.i += 1;
            let e = self.power()?;
            return Some(b.powf(e));
        }
        Some(b)
    }
    fn unary(&mut self) -> Option<f64> {
        if self.peek() == Some('-') {
            self.i += 1;
            return Some(-self.unary()?);
        }
        if self.peek() == Some('(') {
            self.i += 1;
            let v = self.expr()?;
            if self.peek() != Some(')') {
                return None;
            }
            self.i += 1;
            return Some(v);
        }
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.') {
            self.i += 1;
        }
        self.s[start..self.i].iter().collect::<String>().parse().ok()
    }
}

/// Text without the indentation all its lines share, the spaces at the line ends and blank lines at the ends.
pub fn plain(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let indent = lines.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    let out: Vec<String> = lines.iter().map(|l| if l.len() >= indent { l[indent..].trim_end().to_string() } else { l.trim().to_string() }).collect();
    out.join("\n").trim_matches('\n').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert!(matches!(classify("https://github.com/Supchik22"), Kind::Link(_)));
        assert!(matches!(classify("www.example.com"), Kind::Link(u) if u == "https://www.example.com"));
        assert!(matches!(classify("19demodov19@gmail.com"), Kind::Email(_)));
        assert!(matches!(classify("+38 (067) 123-45-67"), Kind::Phone(p) if p == "+380671234567"));
        assert!(matches!(classify("#ff8800"), Kind::Colour(_, _)));
        assert!(matches!(classify("rgb(255, 136, 0)"), Kind::Colour(_, _)));
        assert!(matches!(classify("12*7+3"), Kind::Sum(v) if v == 87.0));
        assert!(matches!(classify("~/Pictures/Wallpapers/a.png"), Kind::Path(_)));
        assert!(matches!(classify("fn main() {\n    println!();\n}"), Kind::Block));
        assert_eq!(classify("hello world"), Kind::Plain);
        // a date or a plain number is not a sum
        assert_eq!(classify("2026-11-06"), Kind::Plain);
        assert_eq!(classify("42"), Kind::Plain);
    }

    #[test]
    fn sums() {
        assert_eq!(sum("2^10"), Some(1024.0));
        assert_eq!(sum("(1,5 + 2.5) × 3"), Some(12.0));
        assert_eq!(sum("200*15%"), Some(30.0));
        assert_eq!(sum("10/4"), Some(2.5));
        assert_eq!(sum("1+"), None);
    }

    #[test]
    fn actions() {
        assert_eq!(classify("#ff8800").action("#ff8800").map(|a| a.1), Some(Action::Copy("rgb(255, 136, 0)".into())));
        assert_eq!(classify("rgb(255, 136, 0)").action("").map(|a| a.1), Some(Action::Copy("#ff8800".into())));
        assert_eq!(plain("    a  \n      b\n"), "a\n  b");
    }
}
