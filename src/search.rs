//! Search queries: plain words match anywhere inside words ("affe" finds "Kaffekannor"),
//! `*` / `?` are wildcards, `/…/` is a full regular expression. Always case-insensitive.

use regex::{Regex, RegexBuilder};
use std::ops::Range;

pub enum Query {
    Empty,
    /// Every pattern must match (name or text).
    All(Vec<Regex>),
    /// The pattern did not compile; the message is shown under the search field.
    Bad(String),
}

pub fn parse(q: &str) -> Query {
    let q = q.trim();
    if q.is_empty() {
        return Query::Empty;
    }
    if q.len() >= 2 && q.starts_with('/') && q.ends_with('/') {
        return match build(&q[1..q.len() - 1]) {
            Ok(r) => Query::All(vec![r]),
            Err(e) => Query::Bad(e),
        };
    }
    if q.starts_with('/') {
        // Typing a regex: wait for the closing slash instead of searching for "/…".
        return Query::Empty;
    }
    let pats: Result<Vec<_>, _> = q.split_whitespace().map(|w| build(&wildcard(w))).collect();
    match pats {
        Ok(p) => Query::All(p),
        Err(e) => Query::Bad(e),
    }
}

/// `kaffe*kanna` → `kaffe\S*?kanna`, `f?rg` → `f\Srg`; everything else literal.
fn wildcard(w: &str) -> String {
    let mut out = String::new();
    for part in w.split_inclusive(['*', '?']) {
        let (lit, wild) = match part.chars().last() {
            Some(c @ ('*' | '?')) => (&part[..part.len() - 1], Some(c)),
            _ => (part, None),
        };
        out.push_str(&regex::escape(lit));
        match wild {
            Some('*') => out.push_str(r"\S*?"),
            Some('?') => out.push_str(r"\S"),
            _ => {}
        }
    }
    out
}

fn build(pat: &str) -> Result<Regex, String> {
    RegexBuilder::new(pat)
        .case_insensitive(true)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| e.to_string().lines().last().unwrap_or("invalid pattern").trim().trim_start_matches("error: ").to_string())
}

pub struct Hit {
    /// Higher is better.
    pub score: i64,
    /// One line around the first text match, and the match ranges inside it.
    pub snippet: String,
    pub marks: Vec<Range<usize>>,
}

/// Scores an entry; `None` when a pattern is missing from both name and text.
pub fn hit(pats: &[Regex], name: &str, body: &str) -> Option<Hit> {
    let mut score = 0i64;
    let mut first: Option<Range<usize>> = None;
    for p in pats {
        let in_name = p.is_match(name);
        let mut n = 0;
        for m in p.find_iter(body).take(50) {
            n += 1;
            let word_start = m.start() == 0 || !body[..m.start()].chars().next_back().is_some_and(char::is_alphanumeric);
            score += if word_start { 3 } else { 1 };
            if first.is_none() && !m.range().is_empty() {
                first = Some(m.range());
            }
        }
        if !in_name && n == 0 {
            return None;
        }
        if in_name {
            score += 20;
        }
    }
    let (snippet, marks) = match first {
        Some(r) => snippet(pats, body, r),
        None => (String::new(), Vec::new()),
    };
    Some(Hit { score, snippet, marks })
}

/// The line holding `r`, cut to ~90 chars around it, with every pattern match marked.
fn snippet(pats: &[Regex], body: &str, r: Range<usize>) -> (String, Vec<Range<usize>>) {
    let ls = body[..r.start].rfind('\n').map_or(0, |i| i + 1);
    let le = body[r.end..].find('\n').map_or(body.len(), |i| r.end + i);
    let line = &body[ls..le];
    let (a, b) = (r.start - ls, r.end - ls);
    // Window of about 30 chars before and 60 after, on char boundaries.
    let mut s = line[..a].char_indices().rev().nth(30).map_or(0, |(i, _)| i);
    let mut e = line[b..].char_indices().nth(60).map_or(line.len(), |(i, _)| b + i);
    while !line.is_char_boundary(s) {
        s -= 1;
    }
    while !line.is_char_boundary(e) {
        e += 1;
    }
    let lead = if s > 0 { "…" } else { "" };
    let tail = if e < line.len() { "…" } else { "" };
    let text = format!("{lead}{}{tail}", line[s..e].trim_end());
    let core = &text[lead.len()..text.len() - tail.len()];
    let mut marks: Vec<Range<usize>> = pats
        .iter()
        .flat_map(|p| p.find_iter(core).map(|m| m.start() + lead.len()..m.end() + lead.len()).collect::<Vec<_>>())
        .filter(|m| !m.is_empty())
        .collect();
    marks.sort_by_key(|m| m.start);
    (text, marks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(q: &str) -> Vec<Regex> {
        match parse(q) {
            Query::All(p) => p,
            _ => panic!("{q}"),
        }
    }

    #[test]
    fn infix_wildcards_regex() {
        let body = "Ny kaffekanna från Moccamaster, färg: svart";
        assert!(hit(&all("affe"), "", body).is_some());
        assert!(hit(&all("KAFFE moccA"), "", body).is_some());
        assert!(hit(&all("affe zz"), "", body).is_none());
        assert!(hit(&all("kaffe*kanna"), "", body).is_some());
        assert!(hit(&all("f?rg"), "", body).is_some());
        assert!(hit(&all("/m.cca\\w+/"), "", body).is_some());
        assert!(hit(&all("shopping"), "shopping-list", "nothing").is_some());
        assert!(matches!(parse("/(unclosed/"), Query::Bad(_)));
        assert!(matches!(parse("/typing"), Query::Empty));
        let h = hit(&all("kanna"), "", body).unwrap();
        assert_eq!(&h.snippet[h.marks[0].clone()], "kanna");
    }
}
