//! Light Markdown highlighting for the editor, plus link detection.
//! The text itself is never changed: every byte is laid out exactly once, as a TextEdit
//! layouter requires, so the cursor and selection stay exact.

use crate::theme;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId, Stroke};
use std::ops::Range;

const SCHEMES: [&str; 7] = ["https://", "http://", "ftps://", "ftp://", "file://", "mailto:", "www."];

pub struct Link {
    /// Byte range in the text.
    pub range: Range<usize>,
    /// What to open (`www.` gets `https://`).
    pub url: String,
}

/// Bare links: http(s)://, ftp(s)://, file://, mailto: and www.
pub fn find_links(text: &str) -> Vec<Link> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let boundary = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || matches!(b[i - 1], b'/' | b'.' | b'@' | b'-' | b'_'));
        let scheme = boundary
            .then(|| SCHEMES.iter().find(|s| b.len() - i >= s.len() && b[i..i + s.len()].eq_ignore_ascii_case(s.as_bytes())))
            .flatten();
        let Some(s) = scheme else {
            i += 1;
            continue;
        };
        // Stops only at ASCII bytes, so `end` is always a char boundary.
        let mut end = i + s.len();
        while end < b.len() && !b[end].is_ascii_whitespace() && !matches!(b[end], b'<' | b'>' | b'"' | b'`') {
            end += 1;
        }
        // Trailing punctuation belongs to the sentence; unbalanced closing brackets too.
        while end > i + s.len() {
            let span = &b[i..end];
            let unbalanced = |o: u8, c: u8| span.iter().filter(|&&x| x == c).count() > span.iter().filter(|&&x| x == o).count();
            match b[end - 1] {
                b'.' | b',' | b';' | b':' | b'!' | b'?' | b'\'' | b'*' | b'_' => end -= 1,
                b')' if unbalanced(b'(', b')') => end -= 1,
                b']' if unbalanced(b'[', b']') => end -= 1,
                _ => break,
            }
        }
        let body = &text[i + s.len()..end];
        let ok = !body.is_empty() && (*s != "mailto:" || body.contains('@')) && (*s != "www." || body.contains('.'));
        if ok {
            let raw = &text[i..end];
            let url = if *s == "www." { format!("https://{raw}") } else { raw.to_string() };
            out.push(Link { range: i..end, url });
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// Link under byte offset `at`, if any.
pub fn link_at(text: &str, at: usize) -> Option<Link> {
    find_links(text).into_iter().find(|l| l.range.contains(&at))
}

/// Body with bare links wrapped as CommonMark autolinks (for the rendered preview).
/// Links already inside `<…>` or `](…)` are left alone.
pub fn autolink(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut last = 0;
    for l in find_links(text) {
        let before = &text[..l.range.start];
        if before.ends_with('<') || before.ends_with("](") {
            continue;
        }
        out.push_str(&text[last..l.range.start]);
        let raw = &text[l.range.clone()];
        if raw == l.url {
            out.push_str(&format!("<{raw}>"));
        } else {
            out.push_str(&format!("[{raw}]({})", l.url));
        }
        last = l.range.end;
    }
    out.push_str(&text[last..]);
    out
}

// ---------- highlighting ----------

#[derive(Clone, Copy, PartialEq, Default)]
struct St {
    head: u8,
    bold: bool,
    italic: bool,
    code: bool,
    /// Markdown syntax characters (#, **, `, - …): faint.
    mark: bool,
    list: bool,
    quote: bool,
    /// Ticked task `- [x]`.
    done: bool,
    link: bool,
}

pub fn bold_family() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// Layout job for the editor: headings, **bold**, *italic*, `code`, fenced blocks, lists,
/// task boxes, quotes and links. `base` is the editor font.
pub fn job(text: &str, base: &FontId) -> LayoutJob {
    let mut st = vec![St::default(); text.len()];
    let set = |st: &mut [St], r: Range<usize>, f: &dyn Fn(&mut St)| st[r].iter_mut().for_each(f);

    let mut in_fence = false;
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        let start = pos;
        pos += line.len();
        let l = line.trim_end_matches(['\n', '\r']);
        let trimmed = l.trim_start();
        let indent = l.len() - trimmed.len();
        let end = start + l.len();
        if trimmed.starts_with("```") {
            set(&mut st, start..end, &|s| {
                s.code = true;
                s.mark = true;
            });
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            set(&mut st, start..end, &|s| s.code = true);
            continue;
        }
        let mut body = start + indent;
        let tb = trimmed.as_bytes();
        let hashes = tb.iter().take_while(|&&c| c == b'#').count();
        if indent == 0 && (1..=6).contains(&hashes) && tb.get(hashes) == Some(&b' ') {
            let lvl = hashes as u8;
            set(&mut st, start..end, &|s| s.head = lvl);
            set(&mut st, start..start + hashes + 1, &|s| s.mark = true);
            body = start + hashes + 1;
        } else if trimmed.starts_with('>') {
            set(&mut st, body..end, &|s| s.quote = true);
            set(&mut st, body..body + 1, &|s| s.mark = true);
            body += 1;
        } else if let Some(n) = list_marker(tb) {
            set(&mut st, body..body + n, &|s| s.list = true);
            body += n;
            let rest = &text.as_bytes()[body..end];
            if rest.len() >= 3 && rest[0] == b'[' && rest[2] == b']' && matches!(rest[1], b' ' | b'x' | b'X') {
                let ticked = rest[1] != b' ';
                set(&mut st, body..body + 3, &|s| s.list = true);
                if ticked {
                    set(&mut st, body + 3..end, &|s| s.done = true);
                }
                body += 3;
            }
        }
        inline(text.as_bytes(), body..end, &mut st);
    }
    for l in find_links(text) {
        if !st[l.range.start].code {
            set(&mut st, l.range, &|s| {
                s.link = true;
                s.mark = false;
            });
        }
    }

    let mut job = LayoutJob::default();
    let mut i = 0;
    while i < text.len() {
        let cur = st[i];
        let mut j = i + 1;
        while j < text.len() && (st[j] == cur || !text.is_char_boundary(j)) {
            j += 1;
        }
        job.append(&text[i..j], 0.0, format(cur, base));
        i = j;
    }
    job
}

/// "- ", "* ", "+ ", "12. " → marker length including the space.
fn list_marker(t: &[u8]) -> Option<usize> {
    match t {
        [b'-' | b'*' | b'+', b' ', ..] => Some(2),
        _ => {
            let d = t.iter().take_while(|c| c.is_ascii_digit()).count();
            (d > 0 && t.get(d) == Some(&b'.') && t.get(d + 1) == Some(&b' ')).then_some(d + 2)
        }
    }
}

/// `code`, **bold**, *italic* / _italic_ within one line.
fn inline(b: &[u8], r: Range<usize>, st: &mut [St]) {
    let mut i = r.start;
    let word = |k: usize| k < b.len() && b[k].is_ascii_alphanumeric();
    while i < r.end {
        match b[i] {
            b'`' => {
                if let Some(off) = b[i + 1..r.end].iter().position(|&c| c == b'`') {
                    let close = i + 1 + off;
                    st[i..=close].iter_mut().for_each(|s| s.code = true);
                    st[i].mark = true;
                    st[close].mark = true;
                    i = close + 1;
                    continue;
                }
            }
            b'*' if i + 1 < r.end && b[i + 1] == b'*' => {
                let from = i + 2;
                if from < r.end
                    && b[from] != b' '
                    && let Some(off) = b[from..r.end].windows(2).position(|w| w == b"**")
                    && off > 0
                    && b[from + off - 1] != b' '
                {
                    let close = from + off;
                    st[from..close].iter_mut().for_each(|s| s.bold = true);
                    for k in (i..from).chain(close..close + 2) {
                        st[k].mark = true;
                    }
                    i = close + 2;
                    continue;
                }
            }
            c @ (b'*' | b'_') if i + 1 < r.end && b[i + 1] != b' ' && (c == b'*' || i == 0 || !word(i - 1)) => {
                let from = i + 1;
                let close = (from..r.end).find(|&k| {
                    b[k] == c && b[k - 1] != b' ' && k > from && (c == b'*' || !word(k + 1)) && !(c == b'*' && b.get(k + 1) == Some(&b'*'))
                });
                if let Some(close) = close {
                    st[from..close].iter_mut().for_each(|s| s.italic = true);
                    st[i].mark = true;
                    st[close].mark = true;
                    i = close + 1;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
}

fn format(s: St, base: &FontId) -> TextFormat {
    let size = base.size
        + match s.head {
            1 => 4.0,
            2 => 2.0,
            3 => 1.0,
            _ => 0.0,
        };
    let family = if (s.head > 0 || s.bold) && !s.mark { bold_family() } else { base.family.clone() };
    let color = if s.link {
        theme::p().accent
    } else if s.mark {
        theme::p().faint
    } else if s.list {
        theme::p().accent
    } else if s.code {
        theme::p().code
    } else if s.done || s.quote {
        theme::p().weak
    } else if s.head > 0 {
        theme::p().heading
    } else {
        theme::p().text
    };
    TextFormat {
        font_id: FontId::new(size, family),
        color,
        background: if s.code && !s.mark { theme::p().bg_side } else { Color32::TRANSPARENT },
        italics: s.italic || s.quote,
        underline: if s.link { Stroke::new(1.0, theme::p().accent.gamma_multiply(0.55)) } else { Stroke::NONE },
        strikethrough: if s.done { Stroke::new(1.0, theme::p().weak) } else { Stroke::NONE },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links() {
        let t = "see https://voidflow.tech/a_(b). and www.example.com, mailto:me@x.se or ftp://f.org/x?";
        let l: Vec<_> = find_links(t).into_iter().map(|l| l.url).collect();
        assert_eq!(
            l,
            ["https://voidflow.tech/a_(b)", "https://www.example.com", "mailto:me@x.se", "ftp://f.org/x"]
        );
        assert!(find_links("mailto: nope, www. nope, xhttps://no").is_empty());
        assert_eq!(autolink("a https://x.se b <https://y.se> [z](https://z.se)"), "a <https://x.se> b <https://y.se> [z](https://z.se)");
        assert_eq!(autolink("www.x.se"), "[www.x.se](https://www.x.se)");
    }

    #[test]
    fn job_covers_every_byte() {
        let t = "# Rubrik åäö\n- [x] klar **fet** `kod` *kursiv* snake_case_name\n> citat https://a.se\n```\nkod\n```\n";
        let j = job(t, &FontId::monospace(14.0));
        assert_eq!(j.text, t);
        let covered: usize = j.sections.iter().map(|s| s.byte_range.end.0 - s.byte_range.start.0).sum();
        assert_eq!(covered, t.len());
    }
}
