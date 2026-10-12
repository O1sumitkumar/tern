<p align="center">
  <img src="packaging/macos/icon.svg" width="128" alt="tern icon: a white bird in flight on near-black">
</p>

<h1 align="center">tern</h1>

<p align="center">
  A native SSH client for macOS, written in Rust on <a href="https://github.com/zed-industries/zed/tree/main/crates/gpui">GPUI</a>.<br>
  Saved hosts, SFTP, splits and snippets, with passwords from your own password manager.
<br>
  <a href="https://tern.carrot-soft.tech">Website: https://tern.carrot-soft.tech</a>
</p>

<p align="center">
  <img src="docs/screenshots/demo.gif" alt="⌘K finds web-01, tern connects, runs docker ps, then ⌘\ fills the sudo password" width="900">
</p>

## Why tern

tern does what Termius and Tabby do for a list of servers: saved hosts, ⌘K to any of them, SFTP, splits, snippets and port forwards. It runs as one native process, and one connected tab takes 76 MB where Tabby takes 580 MB ([Memory](#memory)).

Passwords come from your SSH key, from a password-manager command such as `gopass show -o work/ssh/web-01`, or from tern's age-encrypted vault. At a `sudo` prompt, ⌘\\ types the password. It never types one on its own.

Hosts, snippets and settings sync to a private Git repository you own. They are encrypted on your Mac first, so the repository only holds `.age` files.

<p align="center">
  <img src="docs/screenshots/hero.jpg" alt="tern: hosts grouped in a frosted sidebar over Hokusai's Great Wave" width="900">
</p>

## A closer look

| | |
| --- | --- |
| <img src="docs/screenshots/docker.jpg" alt="sudo docker ps with the sudo password filled by ⌘\"><br>**⌘\\ fills `sudo`**: the password comes from your password manager, typed only when you ask and only at a password prompt. | <img src="docs/screenshots/snippets.jpg" alt="The snippets picker filtering docker commands"><br>**Snippets (⇧⌘S)**: saved commands. ↵ marks the ones that run when picked; the rest are typed for you to finish. |
| <img src="docs/screenshots/split.jpg" alt="Two panes side by side with find in scrollback"><br>**Splits and find**: ⌘D and ⌘⇧D split a tab; ⌘F searches the scrollback. | <img src="docs/screenshots/sftp.jpg" alt="The SFTP side panel listing a remote home folder"><br>**Files (⇧⌘O)**: browse the remote side over SFTP; download, or drop files to upload. |
| <img src="docs/screenshots/settings.jpg" alt="Settings, Appearance, with the wallpaper gallery"><br>**Wallpapers and fonts**: seven public-domain artworks built in, or any image; light, dark or follow macOS; any monospace font. | <img src="docs/screenshots/light.jpg" alt="tern in light appearance over Van Gogh's Wheat Field"><br>**Light and dark**: the window takes its colours from the wallpaper, and every text colour stays at least 4.5:1. |

## Features

**Connections**
- Saved connections with groups and tags, search (⇧⌘F), and a ⌘K picker ordered by what you used last.
- Import from `~/.ssh/config` as a one-time copy; tern keeps its own list.
- ProxyJump chains, local, remote and dynamic (SOCKS) port forwards, agent forwarding, and keep-alive per host.
- Automatic reconnect after a network drop, with backoff. It never reconnects after you type `exit`.
- Two-factor (keyboard-interactive) logins, and a hard stop on changed host keys, with the exact `ssh-keygen -R` line to fix one you trust.

**Terminal**
- Tabs you can rename, reorder and reopen on launch; split panes; broadcast input to chosen tabs.
- Find in scrollback, ⌘-click links (including OSC 8), cursor shape and blink, scrollback length, copy on select, and visual bell.
- SFTP browser, session logging to a file, and a status line showing host, state, latency and session time.

**Passwords and keys**
- **Password command** per connection, such as `gopass show -o work/ssh/web-01`. tern runs it when the server asks and never stores the result. A command that arrives through sync must be approved once on each Mac.
- **Vault**: an age-encrypted file for passwords and SSH keys (generate ed25519, import, copy the public key). It unlocks with a PIN or through the macOS Keychain, and locks after you've been idle.
- **⌘\\ Fill password** at `sudo` and other password prompts. It never types on its own.

**Sync**
- To a private Git repository through your own `git` and SSH key (or a GitHub gist). Everything is encrypted with your vault passphrase before it leaves the Mac.
- It runs on its own when something changes and every few minutes. When both sides changed, it merges per file and per host, and only asks about a real conflict.

**Look**
- Panels are frosted glass over your wallpaper and take their tint from it; the picture shows sharp when no session is open. The layout follows [zeron](https://github.com/zeronsh/zeron).
- Light, dark or system appearance. A "sharper text" mode for 1× monitors. Reduce motion follows macOS.

## Memory

tern against the other two SSH clients, on the same Mac, the same day (2026-10-06), the same way. Each app was launched fresh and measured every 15 s for two minutes with macOS `footprint` (the number Activity Monitor shows), summed over all of its processes. The table gives the settled value at two minutes. "Connected" means one tab logged in to the same server.

| App | Idle | Connected | Processes |
| --- | --- | --- | --- |
| **tern** | **86 MB** (108 MB with a wallpaper) | **76 MB** (125 MB with a wallpaper) | 1 |
| Tabby | 471 MB | 580 MB | 5 (Electron) |
| Termius | 583 MB | not measured | 7 (Electron) |

Connected, tern uses 76 MB against Tabby's 580 MB, or 125 MB with a wallpaper. The lead is over Electron apps: OxideTerm, another GPUI SSH client, reports 81 MB idle for itself (not measured here). The method, raw samples and the plain terminals (Terminal, iTerm2, Ghostty) are in [docs/perf.md](docs/perf.md).

## Keyboard

| Action | Keys | Action | Keys |
| --- | --- | --- | --- |
| Host picker | ⌘K | Split right / down | ⌘D / ⌘⇧D |
| Search hosts | ⇧⌘F | Move between panes | ⌥⌘ + arrows |
| New connection | ⌘N | Find in scrollback | ⌘F |
| Snippets | ⇧⌘S | Fill password | ⌘\\ |
| Browse files | ⇧⌘O | Close pane / tab | ⌘W |
| Settings | ⌘, | Next / previous tab | ⇧⌘] / ⇧⌘[ |
| Toggle sidebar | ⌘B | Text size | ⌘= / ⌘− / ⌘0 |

Every shortcut can be rebound in Settings → Shortcuts.

## Install

**Download:** get `tern-<version>-macos-arm64.dmg` from [Releases](https://github.com/your-moon/tern/releases), open it, and drag tern to Applications. Releases are signed with a Developer ID and notarised by Apple, so it opens without a warning.

**Windows and Linux (preview):** `tern-<version>-windows-x86_64.zip` and `tern-<version>-linux-<arch>.tar.gz` are on the same Releases page. They build and pass the tests in CI but have not been run on a real Windows or Linux desktop yet; reports are welcome. Shortcuts there use Ctrl+Shift / Ctrl+Alt in place of ⌘ (table in `crates/tern/src/keymap.rs`).

**Build from source** (Rust 1.99+, macOS on Apple silicon):

```sh
git clone https://github.com/your-moon/tern && cd tern
scripts/make-local-identity.sh   # once: a stable local signature, so Keychain access survives updates
scripts/bundle-app.sh            # builds target/release/bundle/tern.app
cp -R target/release/bundle/tern.app /Applications/
cargo install --locked --path crates/tern   # the `tern` command (tern <host>, tern vault init)
```

## Security

- Host keys are checked against `~/.ssh/known_hosts`. A changed key refuses the connection.
- Secrets live in the vault (age, scrypt) or in your password manager. They are never written to logs, and never typed into the terminal unless you press ⌘\\.
- Sync data is encrypted before it leaves your Mac. The repository only ever sees `.age` files.
- Report vulnerabilities privately, as described in [SECURITY.md](SECURITY.md).

Losing a Mac doesn't lose your setup: see [docs/recovery.md](docs/recovery.md).

## Architecture

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/architecture-dark.svg">
    <img src="docs/architecture.svg" alt="tern architecture: the GPUI app (Shell, Session, Keeper) over the tern-term, tern-ssh, tern-vault and tern-sync crates, with config, password manager, Keychain, Git repo and SSH servers around it" width="900">
  </picture>
</p>

Editable source: [docs/architecture.excalidraw](docs/architecture.excalidraw) (open it at excalidraw.com).

## Design and development

- [Architecture](docs/architecture.html), [UI spec](docs/ui.md), [performance](docs/perf.md), and the [craft rules](CRAFT.md) every change passes.
- `cargo test --workspace` runs the suite, including SSH tests against an in-process server. `cargo run -p tern-ssh --example password_server -- --demo` starts the demo server used for these screenshots.

## Credits

tern copies and adapts code from well-maintained projects rather than reinventing it, and each copied file names its origin. The full list is in [NOTICE.md](NOTICE.md).
- **Code:** [zeron](https://github.com/zeronsh/zeron)'s UI (MIT), [russh](https://github.com/Eugeny/russh) (Apache-2.0), [alacritty_terminal](https://github.com/alacritty/alacritty) (Apache-2.0) and Zed's GPUI.
- **Assets:**
  - icons from Solar by 480 Design (CC BY 4.0) and Geist fonts (OFL);
  - terminal themes from [iTerm2-Color-Schemes](https://github.com/mbadolato/iTerm2-Color-Schemes);
  - built-in wallpapers from [The Met Open Access](https://www.metmuseum.org/about-the-met/policies-and-documents/open-access) (CC0): Hokusai, Hiroshige, Church, Van Gogh and Rousseau.

## License

[GPL-3.0](LICENSE).
