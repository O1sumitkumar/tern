//! Connections made in tern, kept in `hosts.json` next to the settings. tern never writes
//! `~/.ssh/config`: it is often generated (assh, Nix, chezmoi) and an edit there would be lost.
//! A connection with the same name as a `~/.ssh/config` host takes its place in the list.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tern_ssh::{Forward, HostEntry, JumpHop};

use crate::forward_spec;

const FILE_NAME: &str = "hosts.json";
const VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// A private key path; `None` tries ssh-agent and the default keys, as `ssh` does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    /// The folder it is listed under in the sidebar; `None` lists it with the ungrouped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Free labels the sidebar search also matches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Copied from `~/.ssh/config` on import; the form does not edit it, so an edit keeps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_command: Option<String>,
    /// Name of a private key kept in the vault, offered at login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_key: Option<String>,
    /// A shell command whose stdout is the login password. It runs on this Mac, and one that
    /// arrived through sync waits for approval first (see `password_command`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_command: Option<String>,
    /// A shell command whose stdout is the password for `sudo` (and other prompts after
    /// login), filled on the "Fill password" shortcut. Falls back to `password_command`, then
    /// to the vault entry for this host. Approved like `password_command`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sudo_password_command: Option<String>,
    /// Hosts to hop through, comma separated, each a saved connection's name or
    /// `[user@]host[:port]`, in connection order. Empty connects directly.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub proxy_jump: String,
    /// Offer the local ssh-agent to the server, so keys stay here but work there.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub forward_agent: bool,
    /// Copied from `~/.ssh/config` on import (`IdentityAgent`); the form does not edit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_socket: Option<String>,
    /// Seconds between keep-alive probes; `None` is [`DEFAULT_KEEP_ALIVE`], `0` turns them off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_alive: Option<u32>,
    /// Port forwards started once connected, each as `-L`/`-R`/`-D` text (see `forward_spec`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forwards: Vec<String>,
}

/// What the form shows for keep-alive when the connection sets none; matches tern-ssh.
pub const DEFAULT_KEEP_ALIVE: u32 = 30;

impl Connection {
    /// The connection as a host entry, so it connects and lists like a `~/.ssh/config` host.
    /// A jump host named after another connection is not resolved here; see [`Self::entry_in`].
    pub fn entry(&self) -> HostEntry {
        self.entry_in(&[])
    }

    /// Like [`Self::entry`], with jump hosts that name a saved connection in `all` resolved to
    /// that connection's address, user and key.
    pub fn entry_in(&self, all: &[Connection]) -> HostEntry {
        // A hop that cannot be read keeps its text with port 0, so the connection fails naming
        // it rather than skipping the hop and going direct (as tern-ssh does for ssh_config).
        let proxy_jump = split_list(&self.proxy_jump)
            .map(|t| {
                hop(t, all).unwrap_or_else(|_| JumpHop {
                    host: t.to_owned(),
                    port: 0,
                    ..JumpHop::default()
                })
            })
            .collect();
        HostEntry {
            proxy_jump,
            forwards: self
                .forwards
                .iter()
                .filter_map(|f| forward_spec::parse(f).ok())
                .collect(),
            forward_agent: self.forward_agent,
            agent_socket: self.agent_socket.as_deref().map(expand_home),
            server_alive_interval: Some(std::time::Duration::from_secs(u64::from(
                self.keep_alive.unwrap_or(DEFAULT_KEEP_ALIVE),
            ))),
            alias: self.name.clone(),
            host_name: self.host.clone(),
            port: self.port,
            user: Some(self.user.clone()),
            identity_files: self.identity_file.iter().map(|p| expand_home(p)).collect(),
            proxy_command: self.proxy_command.clone(),
            ..Default::default()
        }
    }

    /// A draft copied from a `~/.ssh/config` host, for "Duplicate to edit".
    pub fn from_entry(entry: &HostEntry) -> Self {
        Self {
            name: entry.alias.clone(),
            host: entry.host_name.clone(),
            port: entry.port,
            user: entry.user.clone().unwrap_or_default(),
            identity_file: entry
                .identity_files
                .first()
                .map(|p| p.display().to_string()),
            group: None,
            tags: Vec::new(),
            proxy_command: entry.proxy_command.clone(),
            vault_key: None,
            password_command: None,
            sudo_password_command: None,
            proxy_jump: entry
                .proxy_jump
                .iter()
                .map(hop_text)
                .collect::<Vec<_>>()
                .join(", "),
            forward_agent: entry.forward_agent,
            agent_socket: entry.agent_socket.as_ref().map(|p| p.display().to_string()),
            keep_alive: entry
                .server_alive_interval
                .map(|d| u32::try_from(d.as_secs()).unwrap_or(u32::MAX)),
            forwards: entry.forwards.iter().map(Forward::to_string).collect(),
        }
    }
}

fn split_list(text: &str) -> impl Iterator<Item = &str> {
    text.split(',').map(str::trim).filter(|t| !t.is_empty())
}

/// One jump host: the name of a saved connection, or `[user@]host[:port]`.
fn hop(token: &str, all: &[Connection]) -> Result<JumpHop, String> {
    if let Some(c) = all.iter().find(|c| c.name == token) {
        return Ok(JumpHop {
            host: c.host.clone(),
            port: c.port,
            user: Some(c.user.clone()).filter(|u| !u.is_empty()),
            identity_files: c.identity_file.iter().map(|p| expand_home(p)).collect(),
        });
    }
    let bad = |why: &str| format!("Jump host \"{token}\": {why}");
    let t = token.strip_prefix("ssh://").unwrap_or(token);
    let (user, target) = match t.split_once('@') {
        Some((u, rest)) if !u.is_empty() => (Some(u.to_owned()), rest),
        Some(_) => return Err(bad("empty user.")),
        None => (None, t),
    };
    let (host, port) = match target.strip_prefix('[') {
        Some(rest) => {
            let (h, after) = rest.split_once(']').ok_or_else(|| bad("unclosed [."))?;
            (h, after.strip_prefix(':'))
        }
        None => match target.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') => (h, Some(p)),
            _ => (target, None),
        },
    };
    if host.is_empty() || host.contains(char::is_whitespace) {
        return Err(bad("not a saved connection or a host."));
    }
    let port = match port {
        None => 22,
        Some(p) => p
            .parse::<u16>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| bad("port must be 1 to 65535."))?,
    };
    Ok(JumpHop {
        host: host.to_owned(),
        port,
        user,
        identity_files: Vec::new(),
    })
}

/// A hop read from `~/.ssh/config`, as the text the form shows.
fn hop_text(h: &JumpHop) -> String {
    let host = if h.host.contains(':') {
        format!("[{}]", h.host)
    } else {
        h.host.clone()
    };
    let user = h.user.as_ref().map(|u| format!("{u}@")).unwrap_or_default();
    match h.port {
        0 | 22 => format!("{user}{host}"),
        p => format!("{user}{host}:{p}"),
    }
}

fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    connections: Vec<Connection>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("hosts.json could not be read ({0}); fix or move it, tern will not overwrite it")]
    Unreadable(String),
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// Reads `hosts.json`. A missing file is an empty list; a file that does not parse is an
/// error, so the caller can refuse to save over it rather than lose the user's connections.
///
/// # Errors
/// [`StoreError::Unreadable`] when the file exists but is not valid; [`StoreError::Io`] when it
/// cannot be read.
pub fn load(dir: &Path) -> Result<Vec<Connection>, StoreError> {
    let text = match std::fs::read_to_string(dir.join(FILE_NAME)) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let file: File =
        serde_json::from_str(&text).map_err(|e| StoreError::Unreadable(e.to_string()))?;
    if file.version != VERSION {
        return Err(StoreError::Unreadable(format!("version {}", file.version)));
    }
    Ok(file.connections)
}

/// Writes `hosts.json` through a temp file and rename.
///
/// # Errors
/// [`StoreError::Io`] when the directory or file cannot be written.
pub fn save(dir: &Path, connections: &[Connection]) -> Result<(), StoreError> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(FILE_NAME);
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(&File {
        version: VERSION,
        connections: connections.to_vec(),
    })
    .map_err(io::Error::other)?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// The form's text, as typed.
#[derive(Debug, Default, Clone)]
pub struct Draft {
    pub name: String,
    pub host: String,
    pub port: String,
    pub user: String,
    pub identity_file: String,
    pub group: String,
    /// Comma separated, as typed.
    pub tags: String,
    pub vault_key: String,
    pub password_command: String,
    pub sudo_password_command: String,
    pub proxy_jump: String,
    pub forward_agent: bool,
    /// Seconds, as typed; empty means the default.
    pub keep_alive: String,
    /// One spec per row, as typed; blank rows are ignored.
    pub forwards: Vec<String>,
}

/// `"prod, web ,,prod"` → `["prod", "web"]`: trimmed, no blanks, no repeats, first spelling kept.
pub fn parse_tags(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for tag in text.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        if !tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
            tags.push(tag.to_owned());
        }
    }
    tags
}

/// Checks a draft. `others` are the names of the other tern connections (not the one being
/// edited); reusing a `~/.ssh/config` alias is allowed and replaces that host.
///
/// # Errors
/// A message for the first field that is wrong, worded for the form.
pub fn validate(draft: &Draft, others: &[&str], all: &[Connection]) -> Result<Connection, String> {
    let name = draft.name.trim();
    let host = draft.host.trim();
    let user = draft.user.trim();
    if name.is_empty() {
        return Err("Name is required.".into());
    }
    if others.contains(&name) {
        return Err(format!("A connection named \"{name}\" already exists."));
    }
    if host.is_empty() {
        return Err("Host is required.".into());
    }
    if host.contains(char::is_whitespace) {
        return Err("Host cannot contain spaces.".into());
    }
    let port = match draft.port.trim() {
        "" => 22,
        p => match p.parse::<u16>() {
            Ok(n) if n > 0 => n,
            _ => return Err("Port must be a number from 1 to 65535.".into()),
        },
    };
    if user.is_empty() {
        return Err("User is required.".into());
    }
    let identity_file = Some(draft.identity_file.trim())
        .filter(|p| !p.is_empty())
        .map(str::to_owned);
    let vault_key = Some(draft.vault_key.trim())
        .filter(|k| !k.is_empty())
        .map(str::to_owned);
    let password_command = Some(draft.password_command.trim())
        .filter(|c| !c.is_empty())
        .map(str::to_owned);
    let sudo_password_command = Some(draft.sudo_password_command.trim())
        .filter(|c| !c.is_empty())
        .map(str::to_owned);
    let proxy_jump = split_list(&draft.proxy_jump).collect::<Vec<_>>();
    for t in &proxy_jump {
        hop(t, all)?;
    }
    let keep_alive = match draft.keep_alive.trim() {
        "" => None,
        s => Some(
            s.parse::<u32>()
                .map_err(|_| "Keep-alive must be a number of seconds (0 turns it off).")?,
        ),
    };
    let mut forwards = Vec::new();
    for spec in draft
        .forwards
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        forwards.push(forward_spec::parse(spec)?.to_string());
    }
    Ok(Connection {
        name: name.to_owned(),
        host: host.to_owned(),
        port,
        user: user.to_owned(),
        identity_file,
        group: Some(draft.group.trim())
            .filter(|g| !g.is_empty())
            .map(str::to_owned),
        tags: parse_tags(&draft.tags),
        proxy_command: None,
        vault_key,
        password_command,
        sudo_password_command,
        proxy_jump: proxy_jump.join(", "),
        forward_agent: draft.forward_agent,
        agent_socket: None,
        keep_alive,
        forwards,
    })
}

#[cfg(test)]
#[path = "connections_tests.rs"]
mod tests;
