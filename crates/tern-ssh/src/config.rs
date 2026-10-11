// Adapted from CrabPort crabport-core/src/ssh_import.rs (Apache-2.0): alias enumeration over
// ssh2-config (`get_hosts().skip(1)`, concrete-alias filter, case-insensitive dedupe).
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use ssh2_config::{ParseRule, SshConfig};

use crate::error::{Error, Result};
use crate::sshconf;
use crate::{
    ConnectSpec, DEFAULT_SERVER_ALIVE_COUNT_MAX, DEFAULT_SERVER_ALIVE_INTERVAL, Forward, HostEntry,
    JumpHop,
};

const DEFAULT_IDENTITIES: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

/// Effective parameters for one host after applying `~/.ssh/config`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Resolved {
    pub host_name: String,
    pub port: u16,
    pub user: Option<String>,
    pub identity_files: Vec<PathBuf>,
    pub proxy_command: Option<String>,
    pub proxy_jump: Vec<JumpHop>,
    pub forwards: Vec<Forward>,
    pub forward_agent: bool,
    pub agent_socket: Option<PathBuf>,
    pub server_alive_interval: Option<Duration>,
    pub server_alive_count_max: Option<u32>,
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::home_dir()
}

/// The parsed `~/.ssh/config` plus its text, which [`sshconf`] reads for line order.
pub(crate) struct Loaded {
    pub config: SshConfig,
    pub text: String,
}

impl Loaded {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let rules = ParseRule::ALLOW_UNKNOWN_FIELDS | ParseRule::ALLOW_UNSUPPORTED_FIELDS;
        match SshConfig::default().parse(&mut text.as_bytes(), rules) {
            Ok(config) => Some(Loaded {
                config,
                text: text.to_string(),
            }),
            Err(e) => {
                tracing::warn!(error = %e, "ssh_config_parse_failed");
                None
            }
        }
    }
}

pub(crate) fn load_config() -> Option<Loaded> {
    let path = home_dir()?.join(".ssh").join("config");
    Loaded::parse(&std::fs::read_to_string(path).ok()?)
}

/// Host, port, user and identities of `alias`, without following its own ProxyJump.
fn resolve_base(config: Option<&Loaded>, alias: &str) -> Resolved {
    let Some(config) = config else {
        return Resolved {
            host_name: alias.to_string(),
            port: 22,
            ..Default::default()
        };
    };
    let p = config.config.query(alias);
    Resolved {
        host_name: p.host_name.unwrap_or_else(|| alias.to_string()),
        port: p.port.unwrap_or(22),
        user: p.user,
        identity_files: p.identity_file.unwrap_or_default(),
        proxy_command: None,
        proxy_jump: Vec::new(),
        forwards: Vec::new(),
        forward_agent: false,
        agent_socket: None,
        server_alive_interval: p.server_alive_interval,
        server_alive_count_max: p
            .unsupported_fields
            .get("serveralivecountmax")
            .and_then(|a| a.first())
            .and_then(|n| n.parse().ok()),
    }
}

pub(crate) fn resolve(config: Option<&Loaded>, alias: &str) -> Resolved {
    let mut r = resolve_base(config, alias);
    let Some(config) = config else {
        return r;
    };
    // man ssh_config, ProxyJump: it "will compete with the ProxyCommand option - whichever is
    // specified first will prevent later instances of the other from taking effect".
    let d = sshconf::directives(&config.text, alias);
    // Forwards accumulate, in file order (man ssh_config: LocalForward, RemoteForward).
    r.forwards = d
        .iter()
        .filter_map(|x| {
            let parsed = match x.keyword.as_str() {
                "localforward" => Forward::parse_local(&x.args),
                "remoteforward" => Forward::parse_remote(&x.args),
                "dynamicforward" => Forward::parse_dynamic(&x.args),
                _ => return None,
            };
            parsed
                .map_err(|e| tracing::warn!(error = %e, keyword = %x.keyword, "ssh_config_forward_skipped"))
                .ok()
        })
        .collect();
    if let Some(v) = sshconf::first(&d, "forwardagent") {
        (r.forward_agent, r.agent_socket) = crate::authn::parse_forward_agent(v);
    }
    let claimed = d
        .iter()
        .find(|x| matches!(x.keyword.as_str(), "proxyjump" | "proxycommand"));
    if let Some(x) = claimed.filter(|x| !x.args.trim().eq_ignore_ascii_case("none")) {
        if x.keyword == "proxyjump" {
            r.proxy_jump = x
                .args
                .split(',')
                .map(str::trim)
                .filter(|h| !h.is_empty())
                .map(|h| parse_hop(h, Some(config)))
                .collect();
        } else if !x.args.trim().is_empty() {
            r.proxy_command = Some(x.args.clone());
        }
    }
    r
}

/// One `ProxyJump` entry: `[user@]host[:port]` or `ssh://[user@]host[:port]`, where `host` may
/// be an alias from the same config. An entry that cannot be read keeps its text with port 0, so
/// the connection fails with that text rather than skipping the hop and going direct.
fn parse_hop(raw: &str, config: Option<&Loaded>) -> JumpHop {
    let t = raw
        .strip_prefix("ssh://")
        .unwrap_or(raw)
        .trim_end_matches('/');
    match split_target(t) {
        Ok((user, host, port)) => {
            let r = resolve_base(config, &host);
            JumpHop {
                host: r.host_name,
                port: port.unwrap_or(r.port),
                user: user.or(r.user),
                identity_files: r.identity_files,
            }
        }
        Err(_) => JumpHop {
            host: raw.to_string(),
            port: 0,
            user: None,
            identity_files: Vec::new(),
        },
    }
}

fn is_concrete_alias(alias: &str) -> bool {
    !alias.is_empty()
        && !alias.starts_with('!')
        && !alias
            .chars()
            .any(|c| matches!(c, '*' | '?' | '[' | ']' | '\\'))
}

pub(crate) fn hosts_from(config: &Loaded) -> Vec<HostEntry> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for block in config.config.get_hosts().iter().skip(1) {
        for clause in &block.pattern {
            let alias = clause.pattern.trim();
            if clause.negated || !is_concrete_alias(alias) {
                continue;
            }
            if !seen.insert(alias.to_ascii_lowercase()) {
                continue;
            }
            let r = resolve(Some(config), alias);
            out.push(HostEntry {
                alias: alias.to_string(),
                host_name: r.host_name,
                port: r.port,
                user: r.user,
                identity_files: r.identity_files,
                proxy_command: r.proxy_command,
                proxy_jump: r.proxy_jump,
                forwards: r.forwards,
                forward_agent: r.forward_agent,
                agent_socket: r.agent_socket,
                server_alive_interval: r.server_alive_interval,
                server_alive_count_max: r.server_alive_count_max,
            });
        }
    }
    out
}

/// Hosts declared in `~/.ssh/config`, wildcard patterns skipped.
pub fn load_ssh_config_hosts() -> Vec<HostEntry> {
    load_config().map(|c| hosts_from(&c)).unwrap_or_default()
}

pub(crate) fn local_user() -> Option<String> {
    ["USER", "USERNAME", "LOGNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
}

/// Default identity files that exist on disk, used when none are configured.
pub(crate) fn default_identity_files() -> Vec<PathBuf> {
    let Some(dir) = home_dir().map(|h| h.join(".ssh")) else {
        return Vec::new();
    };
    DEFAULT_IDENTITIES
        .iter()
        .map(|n| dir.join(n))
        .filter(|p| p.is_file())
        .collect()
}

/// Splits `[user@]host[:port]` (`[v6]:port` for bracketed IPv6).
fn split_target(target: &str) -> Result<(Option<String>, String, Option<u16>)> {
    let t = target.trim();
    if t.is_empty() {
        return Err(Error::EmptyTarget);
    }
    let (user, rest) = match t.rfind('@') {
        Some(i) => {
            let u = &t[..i];
            if u.is_empty() {
                return Err(Error::InvalidTarget(t.to_string()));
            }
            (Some(u.to_string()), &t[i + 1..])
        }
        None => (None, t),
    };
    let parse_port = |p: &str| {
        p.parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or_else(|| Error::InvalidPort(p.to_string()))
    };
    let (host, port) = if let Some(r) = rest.strip_prefix('[') {
        let end = r
            .find(']')
            .ok_or_else(|| Error::InvalidTarget(t.to_string()))?;
        let after = &r[end + 1..];
        let port = match after {
            "" => None,
            a => Some(parse_port(
                a.strip_prefix(':')
                    .ok_or_else(|| Error::InvalidTarget(t.to_string()))?,
            )?),
        };
        (r[..end].to_string(), port)
    } else if rest.matches(':').count() == 1 {
        let (h, p) = rest.split_once(':').unwrap_or((rest, ""));
        (h.to_string(), Some(parse_port(p)?))
    } else {
        (rest.to_string(), None)
    };
    if host.is_empty() {
        return Err(Error::InvalidTarget(t.to_string()));
    }
    Ok((user, host, port))
}

pub(crate) fn parse_target(target: &str, config: Option<&Loaded>) -> Result<ConnectSpec> {
    let (user, host, port) = split_target(target)?;
    let r = resolve(config, &host);
    let user = user
        .or(r.user)
        .or_else(local_user)
        .ok_or(Error::NoLocalUser)?;
    Ok(ConnectSpec {
        host: r.host_name,
        port: port.unwrap_or(r.port),
        user,
        identity_files: r.identity_files,
        proxy_command: r.proxy_command,
        proxy_jump: r.proxy_jump,
        forwards: r.forwards,
        forward_agent: r.forward_agent,
        agent_socket: r.agent_socket,
        known_hosts: None,
        memory_keys: Vec::new(),
        server_alive_interval: r
            .server_alive_interval
            .unwrap_or(DEFAULT_SERVER_ALIVE_INTERVAL),
        server_alive_count_max: r
            .server_alive_count_max
            .unwrap_or(DEFAULT_SERVER_ALIVE_COUNT_MAX),
    })
}

/// Expands `%h`, `%p`, `%r`, `%%` in a ProxyCommand, as OpenSSH does.
pub(crate) fn expand_proxy_command(cmd: &str, host: &str, port: u16, user: &str) -> String {
    let mut out = String::with_capacity(cmd.len());
    let mut chars = cmd.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('h') => out.push_str(host),
            Some('p') => out.push_str(&port.to_string()),
            Some('r') => out.push_str(user),
            Some('%') => out.push('%'),
            Some(o) => {
                out.push('%');
                out.push(o);
            }
            None => out.push('%'),
        }
    }
    out
}

impl ConnectSpec {
    /// Builds a connection target from a `~/.ssh/config` host. With no `User`, the local
    /// user name is used, as OpenSSH does.
    ///
    /// # Errors
    ///
    /// [`Error::NoLocalUser`] when the entry has no `User` and the local user name is unknown.
    /// There is deliberately no fallback such as `root`.
    pub fn from_host_entry(e: &HostEntry) -> Result<Self> {
        Ok(ConnectSpec {
            host: e.host_name.clone(),
            port: e.port,
            user: e
                .user
                .clone()
                .or_else(local_user)
                .ok_or(Error::NoLocalUser)?,
            identity_files: e.identity_files.clone(),
            proxy_command: e.proxy_command.clone(),
            proxy_jump: e.proxy_jump.clone(),
            forwards: e.forwards.clone(),
            forward_agent: e.forward_agent,
            agent_socket: e.agent_socket.clone(),
            known_hosts: None,
            memory_keys: Vec::new(),
            server_alive_interval: e
                .server_alive_interval
                .unwrap_or(DEFAULT_SERVER_ALIVE_INTERVAL),
            server_alive_count_max: e
                .server_alive_count_max
                .unwrap_or(DEFAULT_SERVER_ALIVE_COUNT_MAX),
        })
    }

    /// Parses `[user@]host[:port]`; `host` may be an alias from `~/.ssh/config`
    /// (HostName, Port, User, IdentityFile are taken from it; explicit user/port win).
    ///
    /// # Errors
    ///
    /// [`Error::EmptyTarget`], [`Error::InvalidTarget`] or [`Error::InvalidPort`] for a malformed
    /// target, and [`Error::NoLocalUser`] when no user is given anywhere and the local user name
    /// is unknown.
    pub fn parse(target: &str) -> Result<Self> {
        parse_target(target, load_config().as_ref())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn cfg(s: &str) -> Loaded {
        Loaded::parse(s).unwrap()
    }

    const CONF: &str = "Host web\n  HostName 10.1.2.3\n  Port 2222\n  User deploy\n  IdentityFile /keys/web\n\n\
                        Host plain\n  User bob\n\nHost *\n  ProxyCommand nc %h %p\n";

    const JUMP: &str =
        "Host inner\n  HostName 10.9.9.9\n  User me\n  ProxyCommand ssh -W %h:%p bastion\n";

    #[test]
    fn alias_proxy_command_survives_host_name_resolution() {
        let s = parse_target("inner", Some(&cfg(JUMP))).unwrap();
        assert_eq!(s.host, "10.9.9.9");
        assert_eq!(s.proxy_command.as_deref(), Some("ssh -W %h:%p bastion"));
    }

    #[test]
    fn host_entry_carries_its_proxy_command_into_the_spec() {
        let entry = hosts_from(&cfg(JUMP)).remove(0);
        let s = ConnectSpec::from_host_entry(&entry).unwrap();
        assert_eq!(s.proxy_command.as_deref(), Some("ssh -W %h:%p bastion"));
    }

    #[test]
    fn full_target_without_config() {
        let s = parse_target("alice@example.com:2200", None).unwrap();
        assert_eq!(
            (s.user.as_str(), s.host.as_str(), s.port),
            ("alice", "example.com", 2200)
        );
    }

    #[test]
    fn default_port_and_user_from_env() {
        let s = parse_target("example.com", None).unwrap();
        assert_eq!((s.host.as_str(), s.port), ("example.com", 22));
    }

    #[test]
    fn alias_resolves_through_config() {
        let c = cfg(CONF);
        let s = parse_target("web", Some(&c)).unwrap();
        assert_eq!(
            (s.user.as_str(), s.host.as_str(), s.port),
            ("deploy", "10.1.2.3", 2222)
        );
        assert_eq!(s.identity_files, vec![PathBuf::from("/keys/web")]);
    }

    #[test]
    fn explicit_user_and_port_override_alias() {
        let c = cfg(CONF);
        let s = parse_target("root@web:2022", Some(&c)).unwrap();
        assert_eq!(
            (s.user.as_str(), s.host.as_str(), s.port),
            ("root", "10.1.2.3", 2022)
        );
    }

    #[test]
    fn alias_without_hostname_keeps_alias() {
        let c = cfg(CONF);
        let s = parse_target("plain", Some(&c)).unwrap();
        assert_eq!(
            (s.user.as_str(), s.host.as_str(), s.port),
            ("bob", "plain", 22)
        );
    }

    #[test]
    fn bracketed_ipv6() {
        let s = parse_target("u@[::1]:2200", None).unwrap();
        assert_eq!((s.host.as_str(), s.port), ("::1", 2200));
        let s = parse_target("u@::1", None).unwrap();
        assert_eq!((s.host.as_str(), s.port), ("::1", 22));
    }

    #[test]
    fn rejects_bad_targets() {
        assert!(matches!(parse_target("  ", None), Err(Error::EmptyTarget)));
        assert!(matches!(
            parse_target("h:abc", None),
            Err(Error::InvalidPort(_))
        ));
        assert!(matches!(
            parse_target("h:0", None),
            Err(Error::InvalidPort(_))
        ));
        assert!(matches!(
            parse_target("h:70000", None),
            Err(Error::InvalidPort(_))
        ));
        assert!(matches!(
            parse_target("@h", None),
            Err(Error::InvalidTarget(_))
        ));
        assert!(matches!(
            parse_target("u@", None),
            Err(Error::InvalidTarget(_))
        ));
    }

    #[test]
    fn hosts_skip_wildcards_and_dedupe() {
        let c = cfg(&format!("{CONF}\nHost WEB\n  Port 1\n"));
        let h = hosts_from(&c);
        let aliases: Vec<_> = h.iter().map(|e| e.alias.as_str()).collect();
        assert_eq!(aliases, ["web", "plain"]);
        assert_eq!(h[0].host_name, "10.1.2.3");
    }

    #[test]
    fn proxy_command_is_picked_up_and_expanded() {
        let c = cfg(CONF);
        let r = resolve(Some(&c), "web");
        let cmd = r.proxy_command.unwrap();
        assert_eq!(expand_proxy_command(&cmd, "h", 22, "u"), "nc h 22");
        assert_eq!(expand_proxy_command("a%%b%x", "h", 1, "u"), "a%b%x");
    }

    #[test]
    fn forwards_accumulate_across_lines_and_blocks_in_file_order() {
        let c = cfg(
            "Host db\n  LocalForward 8080 localhost:80\n  LocalForward 127.0.0.1:8081 db:5432\n  \
                     LocalForward [::1]:8082 [::1]:80\n  RemoteForward 9000 localhost:3000\n  DynamicForward 1080\n\
                     Host *\n  LocalForward 7000 shared:7000\n",
        );
        let s = parse_target("u@db", Some(&c)).unwrap();
        let shown: Vec<_> = s.forwards.iter().map(ToString::to_string).collect();
        assert_eq!(
            shown,
            [
                "-L 127.0.0.1:8080:localhost:80",
                "-L 127.0.0.1:8081:db:5432",
                "-L [::1]:8082:[::1]:80",
                "-R localhost:9000:localhost:3000",
                "-D 127.0.0.1:1080",
                "-L 127.0.0.1:7000:shared:7000",
            ]
        );
        let other = parse_target("u@other", Some(&c)).unwrap();
        assert_eq!(other.forwards.len(), 1);
    }

    #[test]
    fn bad_forward_lines_are_skipped_not_fatal() {
        let c = cfg("Host a\n  LocalForward 8080\n  LocalForward x db:80\n  DynamicForward 1080\n");
        let entry = hosts_from(&c).remove(0);
        assert_eq!(entry.forwards, [Forward::parse_dynamic("1080").unwrap()]);
        assert_eq!(
            ConnectSpec::from_host_entry(&entry).unwrap().forwards,
            entry.forwards
        );
    }

    #[test]
    fn forward_agent_in_config_reaches_the_spec_first_value_winning() {
        let c = cfg("Host a\n  ForwardAgent yes\nHost b\n  ForwardAgent yes\n\
                     Host c\n  ForwardAgent no\nHost *\n  ForwardAgent yes\n");
        let a = parse_target("u@a", Some(&c)).unwrap();
        assert_eq!((a.forward_agent, a.agent_socket), (true, None));
        assert!(parse_target("u@b", Some(&c)).unwrap().forward_agent);
        // `no` in the specific block beats `yes` in `Host *`.
        assert!(!parse_target("u@c", Some(&c)).unwrap().forward_agent);
        assert!(parse_target("u@d", Some(&c)).unwrap().forward_agent);
        let entries = hosts_from(&c);
        assert!(entries[0].forward_agent && !entries[2].forward_agent);
        assert!(
            ConnectSpec::from_host_entry(&entries[0])
                .unwrap()
                .forward_agent
        );
        // Unset means off.
        assert!(
            !parse_target("u@x", Some(&cfg("Host a\n  User q\n")))
                .unwrap()
                .forward_agent
        );
    }

    #[test]
    fn proxy_jump_single_hop_takes_the_hop_alias_settings() {
        let c = cfg(
            "Host inner\n  HostName 10.9.9.9\n  User me\n  ProxyJump bastion\n\
                     Host bastion\n  HostName 203.0.113.5\n  Port 2200\n  User jump\n  IdentityFile /keys/jump\n",
        );
        let r = resolve(Some(&c), "inner");
        assert_eq!(
            r.proxy_jump,
            [JumpHop {
                host: "203.0.113.5".into(),
                port: 2200,
                user: Some("jump".into()),
                identity_files: vec![PathBuf::from("/keys/jump")],
            }]
        );
        assert_eq!(r.proxy_command, None);
    }

    #[test]
    fn proxy_jump_chain_keeps_hop_order_and_each_hops_own_port_and_user() {
        let c = cfg("Host edge\n  HostName 198.51.100.7\n  User gw\n\
                     Host core\n  HostName 10.0.0.9\n  Port 2300\n\
                     Host inner\n  HostName 10.9.9.9\n  ProxyJump edge,alice@10.0.0.1:2022,ssh://core\n");
        let hops = resolve(Some(&c), "inner").proxy_jump;
        let shown: Vec<_> = hops
            .iter()
            .map(|h| (h.user.as_deref(), h.host.as_str(), h.port))
            .collect();
        assert_eq!(
            shown,
            [
                (Some("gw"), "198.51.100.7", 22),
                (Some("alice"), "10.0.0.1", 2022),
                (None, "10.0.0.9", 2300),
            ]
        );
    }

    #[test]
    fn proxy_jump_none_is_no_hop_and_an_unreadable_hop_keeps_its_text() {
        let none = cfg("Host a\n  ProxyJump none\n");
        assert!(resolve(Some(&none), "a").proxy_jump.is_empty());
        let bad = cfg("Host a\n  ProxyJump bob@\n");
        assert_eq!(
            resolve(Some(&bad), "a").proxy_jump,
            [JumpHop {
                host: "bob@".into(),
                port: 0,
                user: None,
                identity_files: Vec::new(),
            }]
        );
    }

    #[test]
    fn whichever_of_proxyjump_and_proxycommand_comes_first_wins() {
        let cmd_first = cfg("Host a\n  ProxyCommand nc gw 22\n  ProxyJump bastion\n");
        let r = resolve(Some(&cmd_first), "a");
        assert_eq!(
            (r.proxy_command.as_deref(), r.proxy_jump.len()),
            (Some("nc gw 22"), 0)
        );
        let jump_first = cfg("Host a\n  ProxyJump bastion\n  ProxyCommand nc gw 22\n");
        let r = resolve(Some(&jump_first), "a");
        assert_eq!((r.proxy_command, r.proxy_jump.len()), (None, 1));
    }

    #[test]
    fn keep_alive_settings_come_from_config_or_the_defaults() {
        let c =
            cfg("Host a\n  ServerAliveInterval 10\n  ServerAliveCountMax 5\nHost b\n  User x\n");
        let a = parse_target("u@a", Some(&c)).unwrap();
        assert_eq!(
            (a.server_alive_interval, a.server_alive_count_max),
            (Duration::from_secs(10), 5)
        );
        let b = parse_target("u@b", Some(&c)).unwrap();
        assert_eq!(
            (b.server_alive_interval, b.server_alive_count_max),
            (
                DEFAULT_SERVER_ALIVE_INTERVAL,
                DEFAULT_SERVER_ALIVE_COUNT_MAX
            )
        );
    }

    #[test]
    fn host_star_supplies_defaults_and_the_first_value_wins() {
        let c =
            cfg("Host web\n  HostName 10.1.1.1\n  Port 2200\nHost *\n  User deploy\n  Port 2022\n");
        let s = parse_target("web", Some(&c)).unwrap();
        assert_eq!(
            (s.host.as_str(), s.port, s.user.as_str()),
            ("10.1.1.1", 2200, "deploy")
        );
        let s = parse_target("other", Some(&c)).unwrap();
        assert_eq!((s.host.as_str(), s.port), ("other", 2022));
    }

    #[test]
    fn identity_files_from_every_matching_block_are_offered_in_order() {
        let c = cfg("Host web\n  IdentityFile /keys/web\nHost *\n  IdentityFile /keys/shared\n");
        assert_eq!(
            parse_target("web", Some(&c)).unwrap().identity_files,
            [PathBuf::from("/keys/web"), PathBuf::from("/keys/shared")]
        );
    }

    #[test]
    fn a_host_line_lists_each_concrete_name_and_skips_patterns() {
        let c = cfg("Host alpha beta *.lan !gamma\n  User ops\n");
        let names: Vec<_> = hosts_from(&c).into_iter().map(|h| h.alias).collect();
        assert_eq!(names, ["alpha", "beta"]);
    }

    #[test]
    fn proxy_command_expands_user_repeated_port_and_keeps_a_trailing_percent() {
        assert_eq!(
            expand_proxy_command("ssh -l %r %h", "bastion.example", 2200, "bob"),
            "ssh -l bob bastion.example"
        );
        assert_eq!(expand_proxy_command("%p-%p", "h", 2200, "u"), "2200-2200");
        assert_eq!(expand_proxy_command("echo 100%", "h", 22, "u"), "echo 100%");
    }
}
