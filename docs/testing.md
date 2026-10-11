# Testing tern

How tern decides a test is worth keeping, built from how the most-tested open-source
systems judge their own suites — [SQLite][sqlite], and Google's Testing on the Toilet
([change-detector tests][cdt], [test behavior not implementation][tbni]).

## The one principle

**A passing test is not evidence. A test earns its place only if it can fail when the code
is wrong.** The same thinking that writes a bug writes a test that can't catch it: four
identical streams in a four-stream test, a palindrome fed to a reverser, a golden that omits
the field being added. Every technique below exists to make "can this test fail?" a question
we answer with a tool, not a hope.

SQLite states the bar plainly: it reaches 100% branch and MC/DC coverage *and then mutation-
tests the binary* — rewriting branches in the compiled code — because coverage alone only
proves a line ran, not that any assertion would have noticed it go wrong. We do the same with
`cargo-mutants`.

## What a good tern test looks like

A test we keep:

1. **Can fail.** A surviving `cargo-mutants` mutant in the code it covers means it cannot;
   fix the test, not the mutant. On money/state/wire code, also hand-mutate once (flip a sign,
   swap two args, drop a branch) and watch it go red.
2. **Tests behaviour through the public API, not the implementation.** It asserts what a
   caller observes, not which private function ran or in what order. Google calls the opposite
   a *change-detector test* — "a transformation of the same information in the code under
   test… it breaks in response to any change without verifying correct behaviour" and has
   "negative value". A test that must change when you refactor without changing behaviour is
   one of these; rewrite or delete it.
3. **Is DAMP, not clever.** Descriptive and self-contained beats de-duplicated. A reader
   should see the input and the expected output in the test body, not chase helpers.
4. **Exercises the axis under test with structured input.** Random *bytes* only re-test the
   rejection path. Generate well-formed domain objects (a real `hosts.json`, a real escape
   sequence) and vary them along the one axis the test is about.
5. **Pins a real bug, if it was born from one.** A fix is not done until a test reproduces the
   bug and is red before the fix. SQLite credits this rule with "thousands and thousands" of
   its tests.

## The techniques, and where they apply in tern

### 1. Mutation testing — the meta-check (`cargo-mutants`)

The tool that tells us which tests are decoration. It flips operators, deletes statements and
replaces return values, then runs the suite; a **surviving mutant** is a change no test
noticed.

```sh
cargo mutants -p tern-vault -p tern-sync        # the risk crates first
cargo mutants --in-diff <(git diff main)        # only what a branch changed (CI-sized)
```

Run it on the crates that carry secrets, state and wire formats: **`tern-vault`,
`tern-sync`** (the merge especially), and the version gates in `tern`. Survivors are the
to-do list. It is wired non-blocking in CI (`.github/workflows`) — triage survivors, don't
gate on the number yet.

### 2. Property testing (`proptest`)

For functions where the interesting inputs are too many to enumerate. Assert an *invariant*
over generated inputs, not one example.

- **`tern-sync::merge::merge_hosts`** — the data-loss path. Properties: a connection changed
  on only one side is never dropped; the merge is deterministic and order-stable; identical
  inputs produce no conflict; the output carries each file's real version (not a hardcoded 1 —
  see [#28]). This is tern's money path: a silent merge bug eats a user's hosts on every Mac.
- **`tern-term::Terminal::process`** — the parser (adapted from alacritty). Property: feeding
  any structured stream of escape sequences never panics and leaves the cursor within the grid
  and the grid at its set size. This is tern's fuzzer-in-miniature; structured sequences, not
  random bytes.
- **`tern-vault`** — round-trip: any set of key/values encrypts then decrypts to exactly
  itself, and secrets are zeroized.

### 3. Fault injection (anomaly testing)

SQLite's highest-value layer: OOM on the Nth allocation, I/O errors from a custom VFS, crash
and power-loss mid-write that must leave the database fully committed or fully rolled back.
tern's equivalents, at tern's scale:

- **Crash mid-write.** Every store writes `foo.json.tmp` then renames (`settings.rs`,
  `connections.rs`, `snippets.rs`). Test the interrupted states — a stray `.tmp`, a rename
  that never happened — and assert `load()` returns the *old good* file or empty, never a
  corrupt or half-written one.
- **Hostile sync input.** Feed `merge`/load a truncated, malformed, or **higher-version**
  bundle and assert it refuses *without overwriting* local data. This is also the acceptance
  test for the min-reader-version gate in [#28].
- **Vault corruption.** Wrong passphrase, truncated `.age`, higher `version` → a clear error,
  and the file is never clobbered.
- **SSH failure paths.** Against the in-process test server (`cargo run -p tern-ssh --example
  password_server -- --demo`): auth failure, a changed host key (refuse + the `ssh-keygen -R`
  line), a drop mid-session (reconnect, but never after `exit`).

### 4. Coverage as a floor, not a trophy (`cargo-llvm-cov`)

```sh
cargo llvm-cov --workspace --branch
```

SQLite's caution applies: coverage only means something once mutation testing shows the
covered lines are actually checked. Use it to find *untouched* branches in `tern-vault`,
`tern-sync` and the version policy; do not chase a percentage elsewhere.

### 5. End-to-end is by eye, never by UI automation

The GUI is verified by running the real app and **looking at screenshots** (terminal over
wallpaper, Settings, 1×/2× sharper text, light/dark), measuring with `getComputedStyle` where
a number matters — not with brittle UI specs. A UI test that asserts which element rendered is
a change-detector test by another name. Unit, property and Go/Rust tests stay; UI automation
does not. See `docs/ui.md` and the project memory on verifying by eye.

## Conventions

- **Where tests live.** Small unit and property tests go inline in the module they cover
  (`#[cfg(test)] mod tests`), matching the repo (`merge.rs`, `connections.rs`). Larger
  scenario tests get their own file next to the module (`terminal_tests.rs`, `view_tests.rs`).
- **Lints in tests.** The workspace denies `unwrap`/`expect`/`panic`; test modules opt out
  with `#![allow(clippy::unwrap_used)]` (see `merge.rs`). A test must pass **both**
  `cargo test -p <crate>` and `cargo clippy -p <crate> --all-targets`.
- **No network, no real home dir.** Use `tempfile::TempDir`; never touch `~/.ssh` or a real
  gist in a test.

## Running everything

```sh
cargo test --workspace                       # the suite
cargo clippy --workspace --all-targets       # the craft gates
cargo mutants -p tern-vault -p tern-sync     # prove the critical tests can fail
cargo llvm-cov --workspace --branch          # branch coverage of the critical crates
```

## Per-crate priority

| Crate | First tests to add | Why |
| --- | --- | --- |
| `tern-sync` | `merge_hosts` properties; hostile-bundle refusal | silent data loss across Macs |
| `tern-vault` | round-trip property; corruption/version refusal | leaks or loses secrets |
| `tern` | store crash-mid-write recovery; version gates | corrupt config on a crash |
| `tern-term` | `process` no-panic / grid invariant fuzz | a parser panic kills the session |
| `tern-ssh` | fault paths on the in-process server | auth, host-key, reconnect edges |

[sqlite]: https://www.sqlite.org/testing.html
[cdt]: https://testing.googleblog.com/2015/01/testing-on-toilet-change-detector-tests.html
[tbni]: https://testing.googleblog.com/2013/08/testing-on-toilet-test-behavior-not.html
[#28]: https://github.com/your-moon/tern/issues/28
