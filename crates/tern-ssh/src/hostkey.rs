// Adapted from russh russh/src/keys/known_hosts.rs (Apache-2.0): uses its known-host parsing and
// learn helpers, with our own match policy on top — OpenSSH semantics (known if any recorded key
// matches, changed only if none does; see `verdict`) plus the prompt / hard-fail layering.
use std::path::{Path, PathBuf};

use futures::channel::oneshot;
use russh::client;
use russh::keys::{Algorithm, HashAlg, PublicKey, PublicKeyOrCertificate, known_hosts};

use crate::error::{Cause, Disconnect, Failure, classify};
use crate::forward::{self, RemoteTargets};
use crate::{Prompt, SessionEvent};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Known,
    Unknown,
    Changed { line: usize },
    Error(String),
}

pub(crate) fn verdict(host: &str, port: u16, key: &PublicKey, path: Option<&Path>) -> Verdict {
    let recorded = match path {
        Some(p) => known_hosts::known_host_keys_path(host, port, p),
        None => known_hosts::known_host_keys(host, port),
    };
    let recorded = match recorded {
        Ok(entries) => entries,
        Err(russh::keys::Error::IO(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return Verdict::Unknown;
        }
        Err(e) => return Verdict::Error(e.to_string()),
    };
    // OpenSSH's rule: the host is known if ANY recorded key matches; it is a changed key only
    // when none matches but a key of the same type is on file; otherwise it is unknown. russh's
    // own `check_known_hosts_path` instead stops at the first same-type entry that differs, so a
    // stale line before the current one would wrongly read as "changed" and refuse the host.
    if recorded.iter().any(|(_, recorded)| recorded == key) {
        return Verdict::Known;
    }
    match recorded
        .iter()
        .find(|(_, recorded)| recorded.algorithm() == key.algorithm())
    {
        Some(&(line, _)) => Verdict::Changed {
            line: match path {
                Some(p) => physical_line(p, line),
                None => line,
            },
        },
        None => Verdict::Unknown,
    }
}

/// `known_host_keys_path` numbers its entries by counting only non-comment lines (it `continue`s
/// past a `#` line before incrementing), so the line it hands back is short by the number of
/// comment lines above the entry. This walks the file the same way to recover the physical line
/// an editor would show, counting comments and blanks.
fn physical_line(path: &Path, counted: usize) -> usize {
    let Ok(text) = std::fs::read_to_string(path) else {
        return counted;
    };
    let mut non_comment = 1usize;
    for (index, line) in text.lines().enumerate() {
        if line.as_bytes().first() == Some(&b'#') {
            continue;
        }
        if non_comment == counted {
            return index + 1;
        }
        non_comment += 1;
    }
    counted
}

/// Algorithms already recorded for this host, so the server is asked for those first
/// (otherwise a server offering a different key type would look "unknown", not "changed").
pub(crate) fn known_algorithms(host: &str, port: u16, path: Option<&Path>) -> Vec<Algorithm> {
    let keys = match path {
        Some(p) => known_hosts::known_host_keys_path(host, port, p),
        None => known_hosts::known_host_keys(host, port),
    };
    let mut out: Vec<Algorithm> = Vec::new();
    for (_, k) in keys.unwrap_or_default() {
        let a = k.algorithm();
        if !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

pub(crate) struct Handler {
    pub host: String,
    pub port: u16,
    pub known_hosts: Option<PathBuf>,
    pub events: async_channel::Sender<SessionEvent>,
    /// Filled when russh reports the connection dead; read by the session loop.
    pub cause: Cause,
    /// Where the server's forwarded-tcpip channels go.
    pub remote: RemoteTargets,
    /// The local agent socket to wire server-opened agent channels to; `None` refuses them.
    pub agent_socket: Option<PathBuf>,
}

impl client::Handler for Handler {
    /// russh returns this from `connect` unchanged (it requires only `From<russh::Error>`),
    /// so a refused host key reaches the session as the specific [`Failure`].
    type Error = Failure;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return Err(Failure::HostKeyUnsupported);
        };
        match verdict(&self.host, self.port, key, self.known_hosts.as_deref()) {
            Verdict::Known => {
                tracing::debug!(host = %self.host, port = self.port, "ssh_host_key_known");
                Ok(true)
            }
            Verdict::Changed { line } => {
                tracing::error!(host = %self.host, port = self.port, known_hosts_line = line, "ssh_host_key_changed");
                Err(Failure::HostKeyChanged { line })
            }
            Verdict::Error(e) => {
                tracing::error!(host = %self.host, port = self.port, error = %e, "ssh_host_key_check_failed");
                Err(Failure::HostKeyCheck(e))
            }
            Verdict::Unknown => {
                tracing::info!(host = %self.host, port = self.port, "ssh_host_key_unknown");
                let (tx, rx) = oneshot::channel();
                let prompt = Prompt::UnknownHostKey {
                    host: self.host.clone(),
                    port: self.port,
                    algorithm: key.algorithm().to_string(),
                    fingerprint_sha256: key.fingerprint(HashAlg::Sha256).to_string(),
                    reply: tx,
                };
                if self
                    .events
                    .send(SessionEvent::Prompt(prompt))
                    .await
                    .is_err()
                {
                    return Err(Failure::UiGone);
                }
                if !rx.await.unwrap_or(false) {
                    return Err(Failure::HostKeyRejected);
                }
                let learned = match self.known_hosts.as_deref() {
                    Some(p) => known_hosts::learn_known_hosts_path(&self.host, self.port, key, p),
                    None => known_hosts::learn_known_hosts(&self.host, self.port, key),
                };
                if let Err(e) = learned {
                    // Same as OpenSSH: the user accepted, so continue, but say we could not persist it.
                    tracing::warn!(host = %self.host, port = self.port, error = %e, "ssh_known_hosts_write_failed");
                }
                Ok(true)
            }
        }
    }

    /// The server opening an agent channel: only honoured when forwarding was asked for.
    async fn server_channel_open_agent_forward(
        &mut self,
        channel: russh::Channel<client::Msg>,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        match self.agent_socket.clone() {
            Some(socket) => {
                reply.accept().await;
                tokio::spawn(crate::authn::bridge(channel, socket));
            }
            None => reply.reject(forward::NOT_FORWARDED).await,
        }
        Ok(())
    }

    /// A connection to a port the server is listening on for a `-R` forward.
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        match self.remote.lookup(connected_address, connected_port) {
            Some((host, port)) => {
                reply.accept().await;
                tokio::spawn(forward::bridge_remote(channel, host, port));
            }
            None => reply.reject(forward::NOT_FORWARDED).await,
        }
        Ok(())
    }

    /// russh calls this once the handshake is done and the link dies; the channel alone only
    /// goes quiet, so this is where a keep-alive timeout or reset becomes visible.
    async fn disconnected(
        &mut self,
        reason: client::DisconnectReason<Self::Error>,
    ) -> Result<(), Self::Error> {
        match reason {
            client::DisconnectReason::ReceivedDisconnect(_) => {
                self.cause.set(Disconnect::ServerClosed);
                Ok(())
            }
            client::DisconnectReason::Error(e) => {
                self.cause.set(classify(&e, true));
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const KEY_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFgetNZuhJ8rf7QTXKOWMELOSgDO5tuRDWi5bRS9UNAC";
    const KEY_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHt6DH7o7tXSqrdZsggH9EARU3XRi2kqRpl9pHdUyBec";

    fn tmp(name: &str, body: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("tern-ssh-{}-{name}", std::process::id()));
        std::fs::write(&p, body).unwrap();
        p
    }

    fn key(s: &str) -> PublicKey {
        PublicKey::from_openssh(s).unwrap()
    }

    #[test]
    fn matching_key_is_known() {
        let p = tmp("known", &format!("strong-b {KEY_A}\n"));
        assert_eq!(
            verdict("strong-b", 22, &key(KEY_A), Some(&p)),
            Verdict::Known
        );
    }

    #[test]
    fn different_key_for_same_host_is_changed() {
        let p = tmp("changed", &format!("other {KEY_B}\nstrong-b {KEY_B}\n"));
        assert_eq!(
            verdict("strong-b", 22, &key(KEY_A), Some(&p)),
            Verdict::Changed { line: 2 }
        );
    }

    #[test]
    fn unlisted_host_and_missing_file_are_unknown() {
        let p = tmp("unlisted", &format!("other {KEY_B}\n"));
        assert_eq!(
            verdict("strong-b", 22, &key(KEY_A), Some(&p)),
            Verdict::Unknown
        );
        let missing = std::env::temp_dir().join("tern-ssh-does-not-exist");
        assert_eq!(
            verdict("strong-b", 22, &key(KEY_A), Some(&missing)),
            Verdict::Unknown
        );
    }

    #[test]
    fn non_default_port_uses_bracket_form() {
        let p = tmp("port", &format!("[strong-b]:2222 {KEY_B}\n"));
        assert_eq!(
            verdict("strong-b", 2222, &key(KEY_A), Some(&p)),
            Verdict::Changed { line: 1 }
        );
        assert_eq!(
            verdict("strong-b", 22, &key(KEY_A), Some(&p)),
            Verdict::Unknown
        );
    }

    /// `ssh-keygen -H` output for `strong-b` with KEY_A: the host name is HMAC-SHA1 hashed.
    const HASHED_STRONG_B: &str = "|1|Vtyj+xDWIGS+plsnabmvtfEd/R8=|ctFVvS3K0CoQBK3vze0cNhFp0FA= ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFgetNZuhJ8rf7QTXKOWMELOSgDO5tuRDWi5bRS9UNAC";

    const ECDSA: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBIQTDbXKX4X4fQAXgrXaqurGfi9GnEVfOaM4DyG4bO5oB/P9sCFUTj2hMxJ0DHWaXF+gkcqcceqOio5b205RMEs=";

    #[test]
    fn hashed_host_name_matches_its_host_and_still_guards_the_key() {
        let p = tmp("hashed", &format!("{HASHED_STRONG_B}\n"));
        assert_eq!(
            verdict("strong-b", 22, &key(KEY_A), Some(&p)),
            Verdict::Known
        );
        assert_eq!(
            verdict("strong-b", 22, &key(KEY_B), Some(&p)),
            Verdict::Changed { line: 1 }
        );
        assert_eq!(
            verdict("other", 22, &key(KEY_A), Some(&p)),
            Verdict::Unknown
        );
    }

    #[test]
    fn comma_separated_names_on_one_line_all_match() {
        let p = tmp("aliases", &format!("strong-b,10.0.0.5 {KEY_A}\n"));
        for host in ["strong-b", "10.0.0.5"] {
            assert_eq!(
                verdict(host, 22, &key(KEY_A), Some(&p)),
                Verdict::Known,
                "{host}"
            );
        }
        assert_eq!(
            verdict("10.0.0.6", 22, &key(KEY_A), Some(&p)),
            Verdict::Unknown
        );
    }

    /// Regression for #37: the reported line counts comment and blank lines, so it points at the
    /// entry as an editor shows it (the host line here is line 3).
    #[test]
    fn changed_line_number_counts_comments_and_blank_lines() {
        let p = tmp(
            "numbering",
            &format!("# kept by hand\n\nstrong-b {KEY_B}\n"),
        );
        assert_eq!(
            verdict("strong-b", 22, &key(KEY_A), Some(&p)),
            Verdict::Changed { line: 3 }
        );
    }

    #[test]
    fn key_of_another_algorithm_is_unknown_not_changed() {
        // A different key type for a recorded host is "not recorded yet" (prompt the user),
        // not a changed key: the session asks for the recorded algorithms first to avoid this.
        let p = tmp("algorithm", &format!("strong-b {KEY_A}\n"));
        assert_eq!(
            verdict("strong-b", 22, &key(ECDSA), Some(&p)),
            Verdict::Unknown
        );
    }

    #[test]
    fn known_algorithms_lists_each_recorded_type_once_in_file_order() {
        let p = tmp(
            "algs",
            &format!("strong-b {KEY_A}\nother {ECDSA}\nstrong-b {KEY_B}\nstrong-b {ECDSA}\n"),
        );
        let algs: Vec<String> = known_algorithms("strong-b", 22, Some(&p))
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(algs, ["ssh-ed25519", "ecdsa-sha2-nistp256"]);
    }

    #[test]
    fn known_algorithms_respects_port_and_is_empty_without_a_file() {
        let p = tmp("algs-port", &format!("[strong-b]:2222 {ECDSA}\n"));
        assert_eq!(
            known_algorithms("strong-b", 2222, Some(&p))
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["ecdsa-sha2-nistp256"]
        );
        assert!(known_algorithms("strong-b", 22, Some(&p)).is_empty());
        let missing = std::env::temp_dir().join("tern-ssh-algs-does-not-exist");
        assert!(known_algorithms("strong-b", 22, Some(&missing)).is_empty());
    }

    /// Regression for #36: OpenSSH accepts a host when any recorded key matches, even if a stale
    /// entry of the same type precedes it. `verdict` checks every entry rather than stopping at
    /// the first same-type mismatch the way russh's own check does.
    #[test]
    fn matching_key_is_known_even_when_a_stale_entry_precedes_it() {
        let p = tmp("stale", &format!("strong-b {KEY_B}\nstrong-b {KEY_A}\n"));
        assert_eq!(
            verdict("strong-b", 22, &key(KEY_A), Some(&p)),
            Verdict::Known
        );
    }
}
