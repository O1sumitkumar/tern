//! A small reader for `~/.ssh/config` that keeps what ssh2-config drops: the order of lines and
//! repeated keywords. ProxyJump against ProxyCommand is decided by which line comes first, and
//! `LocalForward`/`RemoteForward`/`DynamicForward` may repeat; ssh2-config keeps only one of each.
//!
//! Semantics follow `man ssh_config`: the first value of a keyword wins, except for the
//! forwarding keywords, which accumulate. `Host` patterns are matched against the name typed by
//! the user (`*`, `?`, `!negation`); `Match all` applies, any other `Match` is skipped.
use std::path::PathBuf;

/// One keyword line that applies to the host, in file order. The keyword is lower-cased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Directive {
    pub keyword: String,
    pub args: String,
}

/// How deep `Include` may nest before it is ignored (OpenSSH allows 16).
const MAX_INCLUDE_DEPTH: usize = 8;

/// Every directive that applies to `alias`, in the order the file gives them.
pub(crate) fn directives(text: &str, alias: &str) -> Vec<Directive> {
    let mut out = Vec::new();
    scan(text, alias, true, 0, &mut out);
    out
}

fn scan(text: &str, alias: &str, mut active: bool, depth: usize, out: &mut Vec<Directive>) {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((keyword, args)) = split_line(line) else {
            continue;
        };
        match keyword.as_str() {
            "host" => active = host_matches(&args, alias),
            "match" => active = args.trim().eq_ignore_ascii_case("all"),
            "include" if active && depth < MAX_INCLUDE_DEPTH => {
                for path in include_paths(&args) {
                    if let Ok(body) = std::fs::read_to_string(&path) {
                        scan(&body, alias, active, depth + 1, out);
                    }
                }
            }
            _ if active => out.push(Directive { keyword, args }),
            _ => {}
        }
    }
}

/// `Keyword args` or `Keyword=args`; the keyword is lower-cased, the args keep their case.
fn split_line(line: &str) -> Option<(String, String)> {
    let end = line.find(|c: char| c.is_whitespace() || c == '=')?;
    let keyword = line[..end].to_ascii_lowercase();
    let args = line[end..].trim_start_matches(|c: char| c.is_whitespace() || c == '=');
    Some((keyword, args.trim().to_string()))
}

fn host_matches(patterns: &str, alias: &str) -> bool {
    let mut matched = false;
    for p in patterns.split_whitespace() {
        let p = p.trim_matches('"');
        match p.strip_prefix('!') {
            // A negated pattern that matches rules the whole line out.
            Some(neg) if glob(neg, alias) => return false,
            Some(_) => {}
            None => matched |= glob(p, alias),
        }
    }
    matched
}

/// `*` and `?` wildcards, ASCII case-insensitive as host names are.
pub(crate) fn glob(pattern: &str, text: &str) -> bool {
    fn go(p: &[u8], t: &[u8]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some((b'*', rest)) => (0..=t.len()).any(|i| go(rest, &t[i..])),
            Some((b'?', rest)) => !t.is_empty() && go(rest, &t[1..]),
            Some((c, rest)) => t
                .split_first()
                .is_some_and(|(x, tr)| c.eq_ignore_ascii_case(x) && go(rest, tr)),
        }
    }
    go(pattern.as_bytes(), text.as_bytes())
}

/// Files an `Include` names: `~` and relative paths resolve against `~/.ssh`, and `*`/`?` work
/// in the file name (not in directory names).
fn include_paths(args: &str) -> Vec<PathBuf> {
    let Some(home) = crate::config::home_dir() else {
        return Vec::new();
    };
    let ssh_dir = home.join(".ssh");
    let mut out = Vec::new();
    for raw in args.split_whitespace() {
        let raw = raw.trim_matches('"');
        let path = match raw.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => ssh_dir.join(raw),
        };
        let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str()))
        else {
            continue;
        };
        if !name.contains(['*', '?']) {
            out.push(path);
            continue;
        }
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_str().is_some_and(|n| glob(name, n)))
            .map(|e| e.path())
            .collect();
        found.sort();
        out.extend(found);
    }
    out
}

/// First value of `keyword`.
pub(crate) fn first<'a>(d: &'a [Directive], keyword: &str) -> Option<&'a str> {
    d.iter()
        .find(|x| x.keyword == keyword)
        .map(|x| x.args.as_str())
}

/// Every value of `keyword`, in order.
#[cfg(test)]
pub(crate) fn all<'a>(d: &'a [Directive], keyword: &str) -> Vec<&'a str> {
    d.iter()
        .filter(|x| x.keyword == keyword)
        .map(|x| x.args.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_blocks_apply_in_file_order_and_stop_at_the_next_host() {
        let t = "Host a\n  LocalForward 1 x:1\nHost b\n  LocalForward 2 x:2\nHost *\n  LocalForward 3 x:3\n";
        assert_eq!(all(&directives(t, "a"), "localforward"), ["1 x:1", "3 x:3"]);
        assert_eq!(all(&directives(t, "b"), "localforward"), ["2 x:2", "3 x:3"]);
        assert_eq!(all(&directives(t, "c"), "localforward"), ["3 x:3"]);
    }

    #[test]
    fn negation_wildcards_equals_and_case() {
        let t = "Host *.corp !skip.corp\n  User=bob\nHOST Web?\n  port 2200\n";
        assert_eq!(first(&directives(t, "db.corp"), "user"), Some("bob"));
        assert_eq!(first(&directives(t, "skip.corp"), "user"), None);
        assert_eq!(first(&directives(t, "web1"), "port"), Some("2200"));
        assert_eq!(first(&directives(t, "web12"), "port"), None);
    }

    #[test]
    fn global_lines_before_any_host_apply_and_match_blocks_are_skipped() {
        let t = "ServerAliveInterval 9\nMatch host x\n  User no\nMatch all\n  User yes\n";
        let d = directives(t, "x");
        assert_eq!(first(&d, "serveraliveinterval"), Some("9"));
        assert_eq!(all(&d, "user"), ["yes"]);
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let t = "# Host a\n\nHost a\n  # User x\n  User y\n";
        assert_eq!(all(&directives(t, "a"), "user"), ["y"]);
    }

    #[test]
    fn one_host_line_with_several_names_applies_to_each_and_negation_excludes() {
        let t = "Host alpha beta *.lan !gamma\n  User ops\n";
        for name in ["alpha", "beta", "x.lan"] {
            assert_eq!(first(&directives(t, name), "user"), Some("ops"), "{name}");
        }
        assert_eq!(first(&directives(t, "gamma"), "user"), None);
        assert_eq!(first(&directives(t, "delta"), "user"), None);
    }

    #[test]
    fn keyword_separators_are_equals_or_tab_and_arguments_keep_their_case() {
        let t = "Host a\n  Port = 2200\n  IdentityFile\t/Keys/Web\n  User=carol\n";
        let d = directives(t, "a");
        assert_eq!(first(&d, "port"), Some("2200"));
        assert_eq!(first(&d, "identityfile"), Some("/Keys/Web"));
        assert_eq!(first(&d, "user"), Some("carol"));
    }

    #[test]
    fn bare_keyword_line_is_skipped_without_ending_its_block() {
        let t = "Host a\n  Compression\n  User after\n";
        assert_eq!(all(&directives(t, "a"), "user"), ["after"]);
    }
}
