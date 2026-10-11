//! Why a session ended, in a form the app can act on (auto-reconnect only for network causes).

use std::fmt;
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("empty connection target")]
    EmptyTarget,
    #[error("invalid connection target: {0}")]
    InvalidTarget(String),
    #[error("invalid port in connection target: {0}")]
    InvalidPort(String),
    #[error("cannot determine local user name; use user@host")]
    NoLocalUser,
    #[error("invalid port forward: {0:?}")]
    InvalidForward(String),
}

/// Why a port forward could not be started.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ForwardError {
    /// The session has ended.
    #[error("session closed")]
    Closed,
    /// This machine could not listen on the address.
    #[error("cannot listen on {addr}: {reason}")]
    Listen { addr: String, reason: String },
    /// The server would not listen on the address (remote forwards).
    #[error("the server refused to listen on {0}")]
    Refused(String),
}

/// Why input could not be queued for a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InputError {
    /// The remote has stopped reading and the input queue is full; retry later.
    #[error("session is not accepting input right now")]
    Busy,
    /// The session has ended.
    #[error("session closed")]
    Closed,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Why a session ended before or during setup; rendered into `SessionEvent::Closed::error`.
/// Messages never include secrets.
#[derive(Debug)]
pub(crate) enum Failure {
    HostKeyChanged {
        line: usize,
    },
    HostKeyRejected,
    HostKeyUnsupported,
    HostKeyCheck(String),
    AuthFailed,
    AuthCancelled,
    Proxy(String),
    /// A `ProxyJump` entry that could not be read; the text is the entry as written.
    BadJump(String),
    /// A failure while reaching or logging in to one jump host (`host:port`).
    Jump(String, Box<Failure>),
    ConnectTimeout,
    /// The server could not be reached; the text already names where tern tried to go.
    Unreachable(String),
    /// The server stopped answering keep-alives.
    KeepaliveTimeout,
    Transport(String),
    /// A socket error, kept whole so the caller can word it with the host it was reaching.
    Io(std::io::Error),
    UiGone,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::HostKeyChanged { line } => write!(
                f,
                "host key changed: the key presented by the server does not match known_hosts line {line}; \
                 connection refused (possible man-in-the-middle attack)"
            ),
            Failure::HostKeyRejected => write!(f, "host key rejected by user"),
            Failure::HostKeyUnsupported => write!(f, "host certificates are not supported"),
            Failure::HostKeyCheck(e) => write!(f, "host key check failed: {e}"),
            Failure::AuthFailed => write!(
                f,
                "permission denied: the server accepted none of the passwords or keys tried"
            ),
            Failure::AuthCancelled => write!(f, "authentication cancelled"),
            Failure::BadJump(e) => write!(f, "invalid ProxyJump host {e:?}"),
            Failure::Jump(hop, e) => write!(f, "{e} (while going through jump host {hop})"),
            Failure::Proxy(e) => write!(f, "ProxyCommand failed: {e}"),
            Failure::ConnectTimeout => write!(f, "no answer within 15 seconds"),
            Failure::Unreachable(e) | Failure::Transport(e) => write!(f, "{e}"),
            Failure::KeepaliveTimeout => {
                write!(f, "the server stopped answering; connection timed out")
            }
            Failure::Io(e) => write!(f, "{e}"),
            Failure::UiGone => write!(f, "event receiver dropped"),
        }
    }
}

impl From<russh::Error> for Failure {
    fn from(e: russh::Error) -> Self {
        match e {
            russh::Error::IO(io) => Failure::Io(io),
            russh::Error::KeepaliveTimeout | russh::Error::InactivityTimeout => {
                Failure::KeepaliveTimeout
            }
            other => Failure::Transport(other.to_string()),
        }
    }
}

/// Why a session ended. Carried by [`SessionEvent::Closed`](crate::SessionEvent::Closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disconnect {
    /// The remote shell ended with this exit status, e.g. the user typed `exit`.
    Exited(u32),
    /// The remote shell was killed by a signal.
    Signaled(String),
    /// This side closed the session ([`SessionHandle::close`](crate::SessionHandle::close)).
    Local,
    /// The server stopped answering keep-alives, or the connection went silent.
    Timeout,
    /// The connection was reset or broke mid-session (network drop, sleep, roaming).
    Reset,
    /// The server closed the connection, or the shell channel, without an exit status.
    ServerClosed,
    /// The server could not be reached while connecting (refused, no route, no answer).
    Unreachable,
    /// Setup failed for a reason retrying will not fix: host key, login, proxy, bad request.
    Failed,
}

impl Disconnect {
    /// True for causes a reconnect can cure. False after `exit`, a local close, or a refused
    /// login, where reconnecting would be wrong or pointless.
    #[must_use]
    pub fn is_network(&self) -> bool {
        matches!(
            self,
            Disconnect::Timeout
                | Disconnect::Reset
                | Disconnect::ServerClosed
                | Disconnect::Unreachable
        )
    }
}

/// What the russh connection task learned about its own death, shared with the session loop.
/// russh reports a dead link only to the handler, and the channel can go quiet just before.
#[derive(Debug, Clone)]
pub(crate) struct Cause(Arc<watch::Sender<Option<Disconnect>>>);

impl Default for Cause {
    fn default() -> Self {
        Cause(Arc::new(watch::channel(None).0))
    }
}

impl Cause {
    /// Records the first cause only; later ones are consequences of it.
    pub fn set(&self, d: Disconnect) {
        self.0.send_if_modified(|slot| {
            let first = slot.is_none();
            if first {
                *slot = Some(d);
            }
            first
        });
    }

    /// The cause, waiting up to `grace` for russh to report it.
    pub async fn wait(&self, grace: Duration) -> Option<Disconnect> {
        let mut rx = self.0.subscribe();
        let seen = tokio::time::timeout(grace, rx.wait_for(Option::is_some)).await;
        match seen {
            Ok(Ok(v)) => v.clone(),
            _ => None,
        }
    }
}

/// Classifies a failure. `connected` is true once the SSH handshake had finished.
pub(crate) fn classify(f: &Failure, connected: bool) -> Disconnect {
    match f {
        Failure::Jump(_, inner) => classify(inner, connected),
        Failure::KeepaliveTimeout => Disconnect::Timeout,
        Failure::Unreachable(_) | Failure::ConnectTimeout => Disconnect::Unreachable,
        Failure::Io(e) => match e.kind() {
            ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::BrokenPipe
            | ErrorKind::UnexpectedEof => Disconnect::Reset,
            ErrorKind::TimedOut if connected => Disconnect::Timeout,
            _ if connected => Disconnect::Reset,
            _ => Disconnect::Unreachable,
        },
        Failure::Transport(_) if connected => Disconnect::ServerClosed,
        _ => Disconnect::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Error;

    #[test]
    fn only_network_causes_are_reconnectable() {
        for d in [
            Disconnect::Timeout,
            Disconnect::Reset,
            Disconnect::ServerClosed,
            Disconnect::Unreachable,
        ] {
            assert!(d.is_network(), "{d:?}");
        }
        for d in [
            Disconnect::Exited(0),
            Disconnect::Exited(1),
            Disconnect::Signaled("KILL".into()),
            Disconnect::Local,
            Disconnect::Failed,
        ] {
            assert!(!d.is_network(), "{d:?}");
        }
    }

    #[test]
    fn failures_map_to_their_cause() {
        let io = |k| Failure::Io(Error::from(k));
        assert_eq!(
            classify(&Failure::KeepaliveTimeout, true),
            Disconnect::Timeout
        );
        assert_eq!(
            classify(&io(ErrorKind::ConnectionReset), true),
            Disconnect::Reset
        );
        assert_eq!(
            classify(&io(ErrorKind::ConnectionRefused), false),
            Disconnect::Unreachable
        );
        assert_eq!(
            classify(&io(ErrorKind::ConnectionRefused), true),
            Disconnect::Reset
        );
        assert_eq!(classify(&Failure::AuthFailed, true), Disconnect::Failed);
        assert_eq!(
            classify(&Failure::HostKeyRejected, false),
            Disconnect::Failed
        );
        assert_eq!(
            classify(&Failure::Transport("x".into()), false),
            Disconnect::Failed
        );
        assert_eq!(
            classify(&Failure::Transport("x".into()), true),
            Disconnect::ServerClosed
        );
        assert_eq!(
            classify(&Failure::ConnectTimeout, false),
            Disconnect::Unreachable
        );
    }

    #[test]
    fn host_key_changed_message_names_the_known_hosts_line() {
        assert_eq!(
            Failure::HostKeyChanged { line: 7 }.to_string(),
            "host key changed: the key presented by the server does not match known_hosts line 7; \
             connection refused (possible man-in-the-middle attack)"
        );
    }

    #[test]
    fn failure_at_a_jump_host_names_the_hop_it_happened_at() {
        let f = Failure::Jump("bastion.example:2222".into(), Box::new(Failure::AuthFailed));
        assert_eq!(
            f.to_string(),
            "permission denied: the server accepted none of the passwords or keys tried \
             (while going through jump host bastion.example:2222)"
        );
    }

    #[test]
    fn bad_jump_entry_is_quoted_as_written() {
        assert_eq!(
            Failure::BadJump("bob@".into()).to_string(),
            r#"invalid ProxyJump host "bob@""#
        );
    }

    #[test]
    fn russh_timeouts_are_timeouts_and_socket_errors_keep_their_kind() {
        let keepalive = Failure::from(russh::Error::KeepaliveTimeout);
        assert_eq!(classify(&keepalive, true), Disconnect::Timeout);
        let inactivity = Failure::from(russh::Error::InactivityTimeout);
        assert_eq!(classify(&inactivity, true), Disconnect::Timeout);
        let reset = Failure::from(russh::Error::IO(Error::from(ErrorKind::ConnectionReset)));
        assert_eq!(classify(&reset, true), Disconnect::Reset);
    }

    #[test]
    fn a_failure_at_a_jump_host_is_classified_by_its_cause() {
        let at = |inner: Failure| Failure::Jump("bastion:22".into(), Box::new(inner));
        assert_eq!(
            classify(&at(Failure::KeepaliveTimeout), true),
            Disconnect::Timeout
        );
        assert_eq!(classify(&at(Failure::AuthFailed), true), Disconnect::Failed);
    }

    #[test]
    fn host_key_and_proxy_setup_failures_are_never_retried() {
        let setup = [
            Failure::HostKeyChanged { line: 1 },
            Failure::HostKeyCheck("unreadable".into()),
            Failure::Proxy("exit status 1".into()),
            Failure::BadJump("bob@".into()),
        ];
        for f in setup {
            assert_eq!(classify(&f, false), Disconnect::Failed, "{f}");
        }
    }
}
