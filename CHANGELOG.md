# Changelog

All notable changes to tern are recorded here. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Until 1.0, a minor version may break things.

## [Unreleased]

## [0.1.3] - 2026-10-11

### Fixed

- Host key verification now matches OpenSSH: tern accepts a server when any key recorded for it in `known_hosts` matches, so a stale earlier entry of the same type no longer reports the key as changed and refuses the connection (#36). The changed-key message's line number counts comment and blank lines, so it points at the offending entry as an editor shows it (#37).

## [0.1.2] - 2026-10-06

### Added

- Windows (x86_64) and Linux (x86_64, aarch64) builds, as a **preview**: they build and pass the test suite in CI, but have not yet been run on a real Windows or Linux desktop. On those systems saved secrets go to Windows Credential Manager or the Linux Secret Service, config lives in `%APPDATA%\tern` or `~/.config/tern`, password commands run through `cmd /C` on Windows, the Windows OpenSSH agent pipe is used, and shortcuts use Ctrl+Shift / Ctrl+Alt so plain Ctrl keys still reach the shell (table in `crates/tern/src/keymap.rs`).

### Fixed

- Skipping "Vault passphrase (Enter to skip)" holds for the tab. It was asked again at the server's password prompt, after each refused password, and by every automatic reconnect, which then waited on the question instead of reconnecting.
- Sync's `git` ignores `GIT_DIR` and related variables, so a sync started from inside a git hook cannot act on the wrong repository.

## [0.1.1] - 2026-10-06

### Changed

- New app icon: a flat white bird in flight on near-black, replacing the glossy dusk-sky tern.

## [0.1.0] - 2026-10-06

The first release: a daily-driver SSH client for macOS.

### Highlights

- Saved connections with groups, tags, search, a ⌘K picker and a one-time import from `~/.ssh/config`.
- Logins from SSH keys, a per-connection password command (for example gopass), or the vault. ⌘\ fills `sudo` and other password prompts on request.
- Vault (age, scrypt) for passwords and SSH keys, with PIN unlock, macOS Keychain unlock and an idle lock.
- Terminal: tabs (rename, reorder, restore), split panes, find, links, snippets, broadcast input, session logs, a status line with latency, an SFTP browser and port forwards.
- SSH: ProxyJump, agent forwarding, keep-alive per host, two-factor logins, automatic reconnect, and a hard stop on changed host keys.
- Encrypted sync to your own Git repository, merged per file and per host.
- zeron-style look: frosted panels over a wallpaper (seven public-domain artworks built in), light/dark/system, a sharper mode for 1× monitors, and the Arctic tern icon.
- Signed with a Developer ID and notarised.

### Added

- `tern`: vault PIN (Settings → Vault → Unlock with a PIN). A 4 to 8 digit PIN seals the vault passphrase (age scrypt) in the login Keychain item `tern vault pin`; every unlock prompt asks "Vault PIN" first, with Enter or "Use passphrase instead" for the passphrase. 5 wrong PINs in a row (counted in `vault-pin.json`, not synced) delete the item; changing the vault passphrase removes it.
- `tern`: "Fill password" (⌘\\ by default). Types the saved password plus Enter into the focused pane, only when the cursor line ends in a password prompt (`[sudo] password for me:`, `Password:`, a key passphrase), and only on the key press; a hint toast names the shortcut when such a prompt appears. The password comes from the connection's `sudoPasswordCommand`, else its `passwordCommand`, else the vault entry for the host; commands need the same one-time approval as the password command.

- `tern-ssh`: SSH sessions over russh with ssh-agent, key-file (with passphrase), password and keyboard-interactive login; `~/.ssh/config` hosts and `ProxyCommand`; `known_hosts` checking where a changed key refuses the connection and an unknown key asks first.
- `tern-ssh`: bounded output path. Output is coalesced into 64 KiB chunks over an 8-deep queue, and the session stops reading the socket when the UI falls behind; a 16.9 MB burst peaks at 6 MB RSS. Input reports `Busy` instead of queueing without limit.
- `tern-term`: a GPUI terminal view fed by any byte stream, with xterm-256color and truecolor, text attributes, wide characters, box drawing painted as paths, mouse reporting, selection and bracketed paste.
- `tern-ssh`: password, keyboard-interactive and encrypted-key logins are tested against an in-process russh server, including retries after a wrong answer and cancelling.
- `tern`: the app window, with zeron's chrome: transparent 38 pt titlebar, traffic lights at (14, 14), frosted shell over a blurred background, a 10 pt rounded main panel, bundled Geist fonts and structured JSON logs (`TERN_LOG`). Idle: 66 MB.
- `tern`: a host sidebar listing every concrete `Host` in `~/.ssh/config`; clicking one opens its SSH session in the main panel, with a status dot and the host in the titlebar. `tern <host>` connects at start.
- `tern`: login questions (unknown host key, password, key passphrase, keyboard-interactive) are asked inside the terminal with OpenSSH's wording; Backspace edits, Ctrl-C cancels.
- `tern`: session tabs in the titlebar with status dots; clicking a host with a live tab switches to it. ⌘W closes, ⌘1–9 and ⌘⇧[ / ⌘⇧] or ⌃Tab switch, middle-click closes.
- `tern`: a closed tab says "Press Enter to reconnect"; Enter dials the same host again in the same terminal, keeping its scrollback. Clicking a host whose tab is closed reconnects that tab instead of opening another.
- `tern`: ⌘K host picker: fuzzy search (nucleo, as zeron and helix) over host names and addresses; ↑/↓ or ⌃N/⌃P move, Enter connects, Esc or a click outside closes. Card, header, footer and list height are zeron's command palette values.
- `tern`: shortcuts work before the first tab is open and after the last one closes; the window itself now holds focus when no terminal does.
- `tern`: settings in `~/Library/Application Support/tern/settings.json` (or `$TERN_CONFIG_DIR`), written atomically; a corrupt file falls back to defaults.
- `tern`: ⌘B collapses and expands the sidebar over 200 ms; drag its edge to resize between 224 and 400 pt, double-click the edge to reset to 256. Width and collapsed state persist.
- `tern`: ⌘= / ⌘− / ⌘0 change the terminal font size for every tab (8–32 pt, default 13, as zeron); the grid and the remote PTY follow, and the size persists.
- `tern`: a connecting host's status dot breathes (zeron's 2.4 s pulse), synced between its tab and sidebar row; `"reduceMotion": true` in settings holds it still.
- Docs: idle memory baseline in `docs/perf.md`: 49 MB footprint (87 MB RSS) with one connected tab, about 5 MB over an empty window.
- `tern-vault`: one passphrase-encrypted file for SSH passwords (per `user@host:port`) and key passphrases (per key path), in the age format with an scrypt recipient. A wrong passphrase and a tampered file are both refused, saves are atomic, and plaintext is zeroed after use. Not wired into the app yet.
- `tern`: saved logins. After a password or key passphrase is typed and the login works, tern offers to keep it in the vault, creating one with a repeated passphrase if there is none. Later logins answer from the vault (unlocked once per run, Enter skips), try each saved secret once, and fall back to asking. Unlock and save run off the UI thread.
- `tern-ssh`: `password_server` example, a 127.0.0.1-only server that accepts one password, for checking login flows.
- Docs: README lists what the app does today and what is not done yet.
- `tern`: add, edit and remove connections in the app (⌘N or + in the sidebar). They are kept in `hosts.json`, never in `~/.ssh/config`, and a password typed in the form goes to the vault (created on first use). `~/.ssh/config` hosts stay listed, read-only, with Duplicate to edit; a connection with the same name takes their place.
- `tern`: text fields (adapted from gpui's input example) with a masked mode that never copies its secret.
- `tern`: a crash is written to `~/Library/Logs/tern/panic.log` with a backtrace, even when tern was started from Finder or the Dock.
- `tern`: Settings (⌘, or tern → Settings…) in zeron's layout: Appearance (font size, reduce motion), Terminal (Option as Meta), Connections (list, edit, remove, new), Vault (status, lock now) and About. Escape or Back returns.
- `tern`: 725 terminal colour schemes (iTerm2-Color-Schemes), a searchable picker with a colour strip per row and live preview (Enter keeps, Escape reverts), a default theme in Settings → Appearance, and a theme per host from the sidebar (◐). The panel around the terminal takes the scheme's background.
- `tern`: the offer to save a typed password waits for the login banner to finish and starts on its own line.
- `tern`: Settings → Shortcuts lists every app shortcut; click one and press the new keys (Esc cancels, ⌫ unbinds, Reset restores). A chord already in use is refused with the name of the action that has it. Overrides live in `keymap.json` and apply at once; keys without ⌘, ⌃ or ⌥ always reach the remote shell.
- `tern`: Settings → Sync keeps connections, settings, shortcuts and the vault the same on every Mac through one private GitHub gist. The token comes from `gh` or is pasted once into the macOS Keychain. Everything but the already-encrypted vault is sealed with the vault passphrase before upload. Sync now uploads or downloads, a new Mac just downloads, and when both sides changed it asks which one to keep.
- `tern`: keys keep working after the focused field disappears (for example the passphrase field after a sync).
- `tern`: snackbars in seed-design's style at the top of the window (4 s, paused while the pointer is on them, one at a time, queued): sync results, token saved or removed, connection saved, connection removed with Undo, password saved, vault locked, and a background tab that disconnects (with Show).
- `tern`: tabs shrink like a browser's and the strip scrolls once they no longer fit, with the active tab kept in view; long host names lose their middle, not their distinct end.
- `tern`: connection errors name the target and say what to check ("10.0.0.5:22 refused the connection (is an SSH server running there?)", "could not find host …", "no answer within 15 seconds"); a failing ProxyCommand reports its own first error line.
- `tern`: a refused password says "Permission denied, please try again." before asking again, as OpenSSH does, and running out of attempts reads "permission denied: the server accepted none of the passwords or keys tried".
- `tern`: an empty window shows what to do next, with the current shortcuts for finding a host, a new connection and settings.
- `tern`: logs are also written to `~/Library/Logs/tern/tern.log` (daily, a week kept).
- `tern`: `~/.ssh/config` hosts can be removed from tern's list (× on hover; the file is never written), with Undo and a "Removed from the list" section in Settings → Connections to bring them back.
- `tern`: right-click a host or a tab for its menu: connect, open in a new tab, edit or duplicate, theme, remove; on tabs reconnect, close and close others.
- `tern`: zeron's look: Solar icons throughout, one-line host rows with a status dot and faint address, collapsible Saved and ~/.ssh/config sections, a hover … that opens the row menu, a footer with Sync and Settings, sidebar toggle and + by the traffic lights, icons in menus and the settings nav.
- `tern`: sync through your own private git repository (any remote your git can reach; one click creates `tern-sync` on GitHub with `gh`). Each sync is a commit of the same encrypted files; the gist stays as the fallback when no repository is set.
- `tern`: text fields clip to their box and scroll to keep the cursor in view.
- `tern`: a macOS menu bar (Hide, Hide Others, Show All, Quit, Minimize, Zoom) with ⌘Q, ⌘H, ⌥⌘H and ⌘M.
- `tern-term`: `local_demo <command>` for visual checks; scripted runs never take keyboard focus.
- Project: craft gates (`clippy -D warnings`, file-size and attribution checks, `cargo deny`), CI, architecture diagram, UI spec and measured performance notes.

### Changed

- `tern-ssh`: keyboard-interactive rounds arrive as `Prompt::Challenge` with the server's name, instructions and each prompt's text and echo flag, so 2FA prompts such as "Verification code:" are shown as written (#9).
- `tern-ssh`: the known_hosts location is a `ConnectSpec::known_hosts` field instead of the process-wide `TERN_KNOWN_HOSTS` variable; the `connect` example still reads the variable.

### Fixed

- `tern-ssh`: a `~/.ssh/config` alias with both `HostName` and its own `ProxyCommand` now uses that proxy instead of dialling `HostName` directly; a proxy that dies during the handshake is reported with its exit status (#7).

### Security

- A changed host key is a hard stop with no override.
- Secrets are held as zeroizing `SecretString`s and never logged; session output is redacted from `Debug`.
- RUSTSEC-2023-0071 in `rsa` (via russh) is accepted for now; see [SECURITY.md](SECURITY.md) and #16.

[Unreleased]: https://github.com/your-moon/tern/compare/v0.1.3...main
[0.1.3]: https://github.com/your-moon/tern/releases/tag/v0.1.3
[0.1.2]: https://github.com/your-moon/tern/releases/tag/v0.1.2
[0.1.1]: https://github.com/your-moon/tern/releases/tag/v0.1.1
[0.1.0]: https://github.com/your-moon/tern/releases/tag/v0.1.0
