# On-disk format compatibility (1.0)

Design for [#28]. The question for 1.0 is not "add version fields" — most formats already
carry one. It is: make a written compatibility contract, give the version check a migration
path, and close the two places where a *newer* file meets an *older* tern and data is lost.

After 1.0 the "a minor version may break things" caveat is retired, so these formats become
permanent contracts the moment a stranger trusts the app with their data — and, because tern
syncs, the moment two Macs on different tern versions share one Git repo.

## What exists today (measured in the code, 2026-10-11)

| Format | File | Versioned? | On version/parse mismatch |
| --- | --- | --- | --- |
| Connections | `hosts.json` | `version:1`, hard `==` check | `Unreadable` → **refuses to overwrite** (fail-closed, safe) |
| Snippets | `snippets.json` | `version:1`, hard `==` check | `Unreadable` → refuses to overwrite (fail-closed, safe) |
| Vault | `.age` (JSON inside) | `version:1`, hard `==` check | `Corrupt` → refuses (fail-closed, safe) |
| Settings | `settings.json` | **no version field** | parse error → **silently reset to defaults and saved back** |
| Sync bundle | per-file + `tern-meta.json` | wrapper `version:1` | merge **hardcodes `version:1` on output, ignores input version** |

Evolution so far has been additive: new optional fields with `#[serde(default)]`, version kept
at 1. `hosts.json` has round-trip tests proving files written before groups/tags and before
jump-hosts/forwards still load. That strategy is sound — it just isn't written down, and the
version field is currently a dead-end (a bump produces a file every shipped reader rejects),
not a migration hook.

## The two real data-loss paths

1. **`settings.json`, newer file meets older tern.** A newer tern writes a field or enum value
   an older tern can't parse. The older tern's `load()` hits the parse error, **resets to
   defaults, and saves that back** (`settings.rs:176-182`). If settings sync, the next sync
   pushes the wiped defaults and the newer Mac loses its settings. This is the only path that
   *actively destroys* data rather than refusing.

2. **Sync merge hardcodes `version:1`** (`merge.rs:134`) and never reads the input version. If
   `hosts.json` ever goes to v2, the merge stamps a v1 tag on a v2-shaped body, and an older
   Mac pulling the bundle has no signal to refuse. Cross-version sync is the highest-risk area
   because two versions sharing one repo is a *designed* state, not an edge case.

The other three (hosts, snippets, vault) are already safe: a higher version is refused without
overwrite. The cost there is UX (the user is stuck with "Unreadable"), not lost data.

## The contract (proposed)

**Rule, written in CRAFT.md.** Every on-disk format tern owns carries `version: u32`. Within a
version, changes are **additive only**: new fields are optional with `#[serde(default)]`; no
field is removed, renamed, or has its meaning changed; no required field is added; readers
ignore unknown fields (so never `deny_unknown_fields`). Additive-only is what lets an old and a
new tern share a file, and a sync repo, without a version bump.

**Three-case version policy** (replaces today's `== else error`), per format:

- `version == CURRENT` — normal.
- `version < CURRENT` — run `migrate(from → CURRENT)`, a pure per-format function, then proceed.
  For the additive history this is the identity; the hook must still exist so the first
  non-additive change (a rename, a semantic change, a vault re-encrypt) has a home instead of
  being unshippable.
- `version > CURRENT` — **older tern, newer file.** Read what is safe, **never write over it**,
  and surface "written by a newer tern; update tern." This is already how hosts/snippets/vault
  behave; the job is to make settings and sync behave the same.

**Fixes to land for 1.0:**

- **`settings.json`:** add `version: u32`; on an unparseable or higher-version file, stop
  silently resetting-and-saving — load defaults in memory for the run and **refuse to save over
  the file** (match `hosts.json`), or back it up to `settings.json.bak` first. (Owner decision
  below.)
- **Sync:** stop hardcoding `version:1`; preserve each file's own version through the merge, and
  record a **min-reader-version** on the bundle so an out-of-date Mac refuses the pull with
  "update tern to keep syncing" instead of corrupting the repo or looping.
- **Vault:** document that the age/scrypt envelope and `WORK_FACTOR = 18` are part of the
  contract; changing them is a re-encrypt migration, not an in-place bump.

**Tests (mutation-checked).** Per format: an old golden file round-trips (hosts already has
these — add for settings, snippets, vault); a higher-version file is refused *without
overwrite* (all four); a cross-version sync (v2 bundle vs v1 Mac) refuses rather than corrupts.
Mutation check: flip each `<` / `>` / `==` in the version policy and confirm a test goes red —
a version gate that no test can fail is decoration.

## Decisions (settled 2026-10-11)

1. **Settings on a higher/corrupt version: refuse-to-save.** Consistent with `hosts.json`. The
   older/corrupt file is kept, tern runs on in-memory defaults, and never overwrites it. This
   changes today's behaviour (silent reset-and-save): when a settings file is genuinely corrupt
   (not a version skew), the user keeps the broken file and runs on defaults until they fix or
   move it.

2. **Sync downgrade policy: hard-refuse with a min-reader-version gate.** An out-of-date Mac
   pulling a newer bundle stops syncing and says "update tern to keep syncing", rather than
   merging best-effort. Never corrupts the repo; the cost is that the old Mac can't sync until
   updated.

3. **Per-format versions plus one documented "format epoch".** Keep the per-format `version`
   field already present; document a single epoch for reasoning about cross-version compat.

[#28]: https://github.com/your-moon/tern/issues/28
