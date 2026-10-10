//! What kind of text is in a note: a PowerShell, shell or cmd snippet, a path or a web address.
//! Cheap on purpose (first few KB, a handful of patterns); used for the "send to" suggestion.

use regex::Regex;
use std::sync::OnceLock;

/// Only this much of a note is looked at.
const SCAN: usize = 4096;
/// A language needs this score, and a clear lead over the runner-up, to be suggested.
const MIN_SCORE: u32 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Pwsh,
    Bash,
    Cmd,
    /// A Windows path: `C:\…`, `\\server\share\…`.
    WinPath,
    /// A Linux path: `/mnt/c/…`, `/home/…`, `~/…`.
    UnixPath,
    Url,
}

struct Rules {
    pwsh: Vec<(Regex, u32)>,
    bash: Vec<(Regex, u32)>,
    cmd: Vec<(Regex, u32)>,
    win_path: Regex,
    unix_path: Regex,
    url: Regex,
}

fn rules() -> &'static Rules {
    static R: OnceLock<Rules> = OnceLock::new();
    R.get_or_init(|| {
        let set = |v: &[(&str, u32)]| v.iter().map(|(p, w)| (Regex::new(p).expect("detect pattern"), *w)).collect();
        Rules {
            pwsh: set(&[
                (r"(?m)^#!.*\bpwsh\b", 5),
                (r"\b(?:Get|Set|New|Remove|Start|Stop|Invoke|Test|Write|Read|Import|Export|Select|Where|ForEach|Out|Add|Copy|Move|Rename|Join|Split|Convert|ConvertTo|ConvertFrom|Format|Measure|Sort|Group|Update|Install|Uninstall|Enable|Disable|Register|Resolve|Expand|Compress|Wait|Push|Pop)-[A-Z][A-Za-z]+", 3),
                (r"\$env:\w+", 3),
                (r"\$(?:_|PSItem|PSScriptRoot|null|true|false|LASTEXITCODE)\b", 2),
                (r"\[(?:System\.|string\]|int\]|switch\]|bool\]|Parameter\(|CmdletBinding)", 2),
                (r"(?i)\s-(?:ErrorAction|Force|Recurse|Path|LiteralPath|Filter|ExecutionPolicy|NoProfile)\b", 1),
                (r"(?mi)^\s*(?:param\s*\(|function\s+[\w-]+\s*\{|\$\w+\s*=)", 1),
                (r"\s-(?:eq|ne|gt|lt|ge|le|like|match|and|or|not)\s", 1),
                (r"(?m)^\s*winget\s", 1),
            ]),
            bash: set(&[
                (r"(?m)^#!.*\b(?:bash|sh|zsh|dash)\b", 5),
                (r"(?m)(?:^|[;&|]\s*)sudo\s", 3),
                (r"(?m)(?:^|[;&|]\s*)(?:apt|apt-get|dnf|yum|pacman|brew|snap|systemctl|journalctl|chmod|chown|ln|grep|sed|awk|curl|wget|tar|make|cargo|git|docker|ssh|export|source)\s", 1),
                (r"\$\(|\$\{\w+|\$[A-Z_]{2,}\b", 1),
                (r"(?m)\b(?:fi|done|esac)\s*$|\bthen\s*$|;\s*do\s*$", 3),
                (r"(?m)(?:^|\s)(?:~|/usr|/etc|/home|/opt|/var|/mnt/[a-z])/", 1),
                (r"\|\s*(?:grep|xargs|sort|uniq|head|tail|wc|tee|less)\b", 2),
                (r"(?m)(?:^|\s)-{1,2}[a-z][\w-]*", 0),
            ]),
            cmd: set(&[
                (r"(?mi)^\s*@echo\s+off\b", 5),
                (r"%~?[A-Za-z_]\w*%|%%~?\w", 3),
                (r"(?mi)^\s*(?:set\s+/[ap]|set\s+\w+=|rem\s|goto\s|:\w+\s*$|call\s|if\s+(?:not\s+)?(?:exist|errorlevel|defined)\b)", 3),
                (r"(?mi)\b(?:dir|copy|xcopy|robocopy|del|ren|mkdir|rmdir|cls|start|title)\s+/", 1),
            ]),
            win_path: Regex::new(r#"^"?(?:[A-Za-z]:[\\/]|\\\\[^\\/\s]+[\\/])[^<>|?*\r\n]*"?$"#).expect("win path"),
            unix_path: Regex::new(r"^(?:~|/)[^\x00\r\n]*$").expect("unix path"),
            url: Regex::new(r"^(?:https?|ftp)://\S+$").expect("url"),
        }
    })
}

fn score(rules: &[(Regex, u32)], text: &str) -> u32 {
    rules.iter().map(|(re, w)| if re.is_match(text) { *w } else { 0 }).sum()
}

/// What `text` most likely is, or `None` when nothing stands out.
pub fn detect(text: &str) -> Option<Kind> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let mut end = t.len().min(SCAN);
    while !t.is_char_boundary(end) {
        end -= 1;
    }
    let t = &t[..end];
    let r = rules();
    // A single line can be a location rather than code.
    if !t.contains('\n') {
        if r.url.is_match(t) {
            return Some(Kind::Url);
        }
        if r.win_path.is_match(t) {
            return Some(Kind::WinPath);
        }
        if r.unix_path.is_match(t) && (!t.contains(' ') || t.starts_with("/mnt/")) {
            return Some(Kind::UnixPath);
        }
    }
    let mut scores = [(Kind::Pwsh, score(&r.pwsh, t)), (Kind::Bash, score(&r.bash, t)), (Kind::Cmd, score(&r.cmd, t))];
    scores.sort_by_key(|s| std::cmp::Reverse(s.1));
    let (best, top) = scores[0];
    let runner_up = scores[1].1;
    (top >= MIN_SCORE && top >= runner_up + 2).then_some(best)
}

#[cfg(test)]
mod tests {
    use super::{Kind, detect};

    #[test]
    fn powershell() {
        assert_eq!(detect("Get-ChildItem -Recurse | Where-Object { $_.Length -gt 1MB }"), Some(Kind::Pwsh));
        assert_eq!(detect("$env:PATH += ';C:\\tools'\nWrite-Host 'done'"), Some(Kind::Pwsh));
    }

    #[test]
    fn bash() {
        assert_eq!(detect("sudo apt update && sudo apt install -y ripgrep"), Some(Kind::Bash));
        assert_eq!(detect("#!/bin/bash\nfor f in *.png; do\n  echo $f\ndone"), Some(Kind::Bash));
        assert_eq!(detect("ps aux | grep cargo | head"), Some(Kind::Bash));
    }

    #[test]
    fn cmd() {
        assert_eq!(detect("@echo off\nset NAME=world\necho Hello %NAME%"), Some(Kind::Cmd));
    }

    #[test]
    fn locations() {
        assert_eq!(detect("C:\\Users\\lunal\\Downloads"), Some(Kind::WinPath));
        assert_eq!(detect("\"K:\\VFSH\\Quick Create\""), Some(Kind::WinPath));
        assert_eq!(detect("\\\\nas\\media\\films"), Some(Kind::WinPath));
        assert_eq!(detect("/mnt/k/VFSH/omfile"), Some(Kind::UnixPath));
        assert_eq!(detect("~/projects/omniaware"), Some(Kind::UnixPath));
        assert_eq!(detect("https://voidflow.tech/"), Some(Kind::Url));
    }

    #[test]
    fn prose_is_nothing() {
        assert_eq!(detect("Remember to buy coffee tomorrow, and call the dentist."), None);
        assert_eq!(detect("# Meeting notes\n- budget\n- hiring"), None);
        assert_eq!(detect(""), None);
    }
}
