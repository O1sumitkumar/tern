// Adapted from zeron crates/ui/src/settings.rs (UiSettings load/save/clamped, sidebar bounds) (MIT).
//! UI preferences in `~/Library/Application Support/tern/settings.json`.

use std::io;
use std::path::{Path, PathBuf};

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const SIDEBAR_MIN: f32 = 224.0;
pub const SIDEBAR_MAX: f32 = 400.0;
pub const SIDEBAR_DEFAULT: f32 = 256.0;
/// zeron `typography::FONT_SIZE_MIN/MAX` and `TERMINAL_FONT_SIZE_DEFAULT`.
pub const FONT_MIN: f32 = 8.0;
pub const FONT_MAX: f32 = 32.0;
pub const FONT_DEFAULT: f32 = 13.0;
/// How strongly the wallpaper hero shows at the top of the empty view.
pub const WALLPAPER_HERO_OPACITY_MIN: f32 = 0.1;
pub const WALLPAPER_HERO_OPACITY_MAX: f32 = 1.0;
pub const WALLPAPER_HERO_OPACITY_DEFAULT: f32 = 1.0;
/// The idle lock choices in minutes; 0 is off.
pub const VAULT_LOCK_CHOICES: [u32; 5] = [0, 5, 15, 30, 60];
pub const VAULT_LOCK_DEFAULT: u32 = 15;
/// Where [`VAULT_LOCK_DEFAULT`] sits in [`VAULT_LOCK_CHOICES`].
pub const VAULT_LOCK_DEFAULT_INDEX: usize = 2;
const FILE_NAME: &str = "settings.json";

/// Which palette the window wears: macOS's own, or one of tern's two.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppearanceMode {
    #[default]
    System,
    Light,
    Dark,
}

impl AppearanceMode {
    /// Whether the light palette applies, given whether the system is light.
    pub fn is_light(self, system_light: bool) -> bool {
        match self {
            Self::System => system_light,
            Self::Light => true,
            Self::Dark => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub appearance: AppearanceMode,
    /// The strip under the terminal: host, state, session time.
    pub show_status_line: bool,
    /// On a standard-density (1x) display: whole-pixel sizes, 1 px icon strokes, an opaque
    /// window fill and stronger muted text, since the text there is grayscale-only and thin.
    pub sharp_text: bool,
    pub sidebar_width: f32,
    pub sidebar_collapsed: bool,
    pub terminal_font_size: f32,
    /// Terminal font family; `None` is the bundled Geist Mono.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_font_family: Option<String>,
    /// Hold looping animations still. macOS's Reduce motion does the same on its own.
    pub reduce_motion: bool,
    /// Option sends Meta (ESC-prefixed keys) to the remote, as most terminals offer.
    pub option_as_meta: bool,
    /// Open the tabs of the last run again at launch.
    pub reopen_tabs: bool,
    /// Write every session's output to a log file.
    pub log_sessions: bool,
    /// Bundled scheme for every host without its own; `None` is zeron's palette.
    pub terminal_theme: Option<String>,
    /// Per-host scheme by host alias, as Termius does per host.
    pub host_themes: BTreeMap<String, String>,
    /// Image (png, jpeg, webp) shown as the hero of the empty view and tinting the window's
    /// colours; `None` is the plain frost.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<String>,
    pub wallpaper_hero_opacity: f32,
    /// Artwork treatment applied to the wallpaper once and cached.
    pub wallpaper_effect: crate::wallpaper_fx::Effect,
    /// Recent wallpapers (copies in tern's folder), newest first.
    pub wallpaper_history: Vec<String>,
    /// Take the window's surfaces, accent and washes from the wallpaper's dominant colour.
    pub wallpaper_theme_colors: bool,
    /// The picture covers the whole window, with translucent panels carrying the text; off
    /// keeps it to the top of the empty view.
    pub wallpaper_fills_window: bool,
    /// A git remote to sync through (any host git can reach); `None` uses a GitHub gist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_remote: Option<String>,
    /// The cursor until the remote picks its own with DECSCUSR.
    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    /// Lines of output kept per terminal.
    pub scrollback_lines: usize,
    /// Finishing a mouse selection copies it, as on Linux desktops.
    pub copy_on_select: bool,
    pub middle_click_paste: bool,
    /// Flash the terminal on BEL.
    pub visual_bell: bool,
    /// Bounce the Dock icon on BEL while tern is in the background.
    pub bell_bounces_dock: bool,
    /// Ask GitHub once a day whether a newer release exists; tern only says so, never installs.
    pub check_for_updates: bool,
    /// Sync at launch and after local changes, once a remote is set up.
    pub sync_auto: bool,
    /// Minutes without key or mouse input before the open vault is locked; 0 is never.
    pub vault_lock_minutes: u32,
    /// The vault passphrase is kept in the macOS login Keychain and the vault opens at launch.
    pub vault_keychain: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    #[default]
    Block,
    Bar,
    Underline,
}

pub const SCROLLBACK_MIN: usize = 1_000;
pub const SCROLLBACK_MAX: usize = 100_000;
pub const SCROLLBACK_DEFAULT: usize = 10_000;
/// The stops the Settings stepper moves between.
pub const SCROLLBACK_STEPS: [usize; 5] = [1_000, 5_000, 10_000, 50_000, 100_000];

impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: AppearanceMode::System,
            show_status_line: true,
            sharp_text: true,
            sidebar_width: SIDEBAR_DEFAULT,
            sidebar_collapsed: false,
            terminal_font_size: FONT_DEFAULT,
            terminal_font_family: None,
            reduce_motion: false,
            option_as_meta: true,
            reopen_tabs: true,
            log_sessions: false,
            terminal_theme: None,
            host_themes: BTreeMap::new(),
            wallpaper: None,
            wallpaper_hero_opacity: WALLPAPER_HERO_OPACITY_DEFAULT,
            wallpaper_effect: crate::wallpaper_fx::Effect::None,
            wallpaper_history: Vec::new(),
            wallpaper_theme_colors: true,
            wallpaper_fills_window: true,
            sync_remote: None,
            cursor_style: CursorStyle::Block,
            cursor_blink: false,
            scrollback_lines: SCROLLBACK_DEFAULT,
            copy_on_select: false,
            middle_click_paste: false,
            visual_bell: false,
            bell_bounces_dock: true,
            check_for_updates: true,
            sync_auto: true,
            vault_lock_minutes: VAULT_LOCK_DEFAULT,
            vault_keychain: false,
        }
    }
}

impl Settings {
    /// Missing file → defaults. A file that does not parse is logged and replaced by defaults
    /// rather than stopping the app; fields it lacks take their defaults.
    pub fn load(dir: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(dir.join(FILE_NAME)) else {
            return Self::default();
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(settings) => settings.clamped(),
            Err(e) => {
                tracing::warn!(error = %e, "settings_corrupt_using_defaults");
                Self::default()
            }
        }
    }

    /// Temp file + rename, so a crash mid-write never leaves a half-written file.
    pub fn save(&self, dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(FILE_NAME);
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)
    }

    /// Widths into their legal range; NaN or infinity back to the default.
    pub fn clamped(mut self) -> Self {
        // Whole points are whole device pixels at 1x and 2x; the shell snaps the rest.
        self.sidebar_width = clamp_or(
            self.sidebar_width,
            SIDEBAR_MIN,
            SIDEBAR_MAX,
            SIDEBAR_DEFAULT,
        )
        .round();
        self.terminal_font_size =
            clamp_or(self.terminal_font_size, FONT_MIN, FONT_MAX, FONT_DEFAULT);
        self.scrollback_lines = self.scrollback_lines.clamp(SCROLLBACK_MIN, SCROLLBACK_MAX);
        self.wallpaper_history
            .truncate(crate::wallpaper::HISTORY_LIMIT);
        self.wallpaper_hero_opacity = clamp_or(
            self.wallpaper_hero_opacity,
            WALLPAPER_HERO_OPACITY_MIN,
            WALLPAPER_HERO_OPACITY_MAX,
            WALLPAPER_HERO_OPACITY_DEFAULT,
        );
        if !VAULT_LOCK_CHOICES.contains(&self.vault_lock_minutes) {
            self.vault_lock_minutes = VAULT_LOCK_DEFAULT;
        }
        self
    }

    /// The next scrollback stop up or down from the current value.
    pub fn step_scrollback(&mut self, up: bool) {
        let now = self.scrollback_lines;
        let next = if up {
            SCROLLBACK_STEPS.into_iter().find(|&s| s > now)
        } else {
            SCROLLBACK_STEPS.into_iter().rev().find(|&s| s < now)
        };
        if let Some(next) = next {
            self.scrollback_lines = next;
        }
    }

    /// One point larger or smaller, held inside the legal range.
    pub fn step_font(&mut self, delta: f32) {
        self.terminal_font_size = (self.terminal_font_size + delta).clamp(FONT_MIN, FONT_MAX);
    }
}

/// `$TERN_CONFIG_DIR` when set (test runs keep their settings apart from the real ones), else
/// the platform's config directory: `~/Library/Application Support/tern` on macOS,
/// `%APPDATA%\tern` on Windows, `$XDG_CONFIG_HOME/tern` (default `~/.config/tern`) on Linux.
/// `None` when there is no such directory.
pub fn dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("TERN_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    platform_dir()
}

#[cfg(target_os = "macos")]
fn platform_dir() -> Option<PathBuf> {
    std::env::home_dir().map(|home| home.join("Library/Application Support/tern"))
}

#[cfg(target_os = "windows")]
fn platform_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join("AppData").join("Roaming")))
        .map(|base| base.join("tern"))
}

/// Where tern's logs go: `~/Library/Logs/tern` on macOS, `%LOCALAPPDATA%\tern\logs` on
/// Windows, `$XDG_STATE_HOME/tern` (default `~/.local/state/tern`) on Linux.
pub fn log_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::home_dir().map(|home| home.join("Library/Logs/tern"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::home_dir().map(|home| home.join("AppData").join("Local")))
            .map(|base| base.join("tern").join("logs"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::home_dir().map(|home| home.join(".local").join("state")))
            .map(|base| base.join("tern"))
    }
}

/// [`log_dir`] as the settings page shows it.
pub fn log_dir_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "~/Library/Logs/tern"
    } else if cfg!(target_os = "windows") {
        "%LOCALAPPDATA%\\tern\\logs"
    } else {
        "~/.local/state/tern"
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_dir() -> Option<PathBuf> {
    // The XDG spec: a relative $XDG_CONFIG_HOME is invalid and ignored.
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::home_dir().map(|home| home.join(".config")))
        .map(|base| base.join("tern"))
}

fn clamp_or(value: f32, min: f32, max: f32, default: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        default
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tern-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A settings file from before the terminal options keeps its values and gains defaults.
    #[test]
    fn older_file_gains_terminal_defaults() {
        let dir = temp_dir("older");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.json"),
            r#"{"terminalFontSize": 15, "optionAsMeta": false}"#,
        )
        .unwrap();
        let got = Settings::load(&dir);
        assert_eq!(got.terminal_font_size, 15.0);
        assert!(!got.option_as_meta);
        assert_eq!(got.cursor_style, CursorStyle::Block);
        assert_eq!(got.scrollback_lines, SCROLLBACK_DEFAULT);
        assert!(got.bell_bounces_dock);
    }

    #[test]
    fn scrollback_steps_between_stops_and_clamps() {
        let mut s = Settings::default();
        s.step_scrollback(true);
        assert_eq!(s.scrollback_lines, 50_000);
        s.step_scrollback(false);
        s.step_scrollback(false);
        assert_eq!(s.scrollback_lines, 5_000);
        // A hand-edited value between stops moves to the neighbouring stop.
        s.scrollback_lines = 7_000;
        s.step_scrollback(false);
        assert_eq!(s.scrollback_lines, 5_000);
        s.scrollback_lines = 100_000;
        s.step_scrollback(true);
        assert_eq!(s.scrollback_lines, 100_000, "stays at the top stop");
        s.scrollback_lines = 5;
        assert_eq!(s.clamped().scrollback_lines, SCROLLBACK_MIN);
    }

    #[test]
    fn missing_file_gives_defaults() {
        assert_eq!(Settings::load(&temp_dir("missing")), Settings::default());
    }

    #[test]
    fn wallpaper_history_is_capped_when_loaded() {
        let many = Settings {
            wallpaper_history: (0..20).map(|i| format!("/w/{i}.png")).collect(),
            ..Settings::default()
        };
        let kept = many.clamped().wallpaper_history;
        assert_eq!(kept.len(), crate::wallpaper::HISTORY_LIMIT);
        assert_eq!(kept[0], "/w/0.png");
    }

    #[test]
    fn wallpaper_hero_opacity_is_clamped() {
        let low = Settings {
            wallpaper_hero_opacity: 0.0,
            ..Settings::default()
        };
        assert_eq!(
            low.clamped().wallpaper_hero_opacity,
            WALLPAPER_HERO_OPACITY_MIN
        );
        let nan = Settings {
            wallpaper_hero_opacity: f32::NAN,
            ..Settings::default()
        };
        assert_eq!(
            nan.clamped().wallpaper_hero_opacity,
            WALLPAPER_HERO_OPACITY_DEFAULT
        );
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = temp_dir("roundtrip");
        let saved = Settings {
            appearance: AppearanceMode::Light,
            show_status_line: false,
            sharp_text: false,
            sidebar_width: 312.0,
            sidebar_collapsed: true,
            terminal_font_size: 15.0,
            terminal_font_family: Some("JetBrainsMono Nerd Font Mono".into()),
            reduce_motion: true,
            option_as_meta: false,
            reopen_tabs: false,
            log_sessions: true,
            terminal_theme: Some("Dracula".into()),
            host_themes: BTreeMap::from([("grape".into(), "Nord".into())]),
            wallpaper: Some("/tmp/w.png".into()),
            wallpaper_hero_opacity: 0.5,
            wallpaper_effect: crate::wallpaper_fx::Effect::Halftone,
            wallpaper_history: vec!["/tmp/w.png".into(), "/tmp/v.png".into()],
            wallpaper_theme_colors: false,
            wallpaper_fills_window: false,
            sync_remote: Some("git@github.com:me/tern-sync.git".into()),
            cursor_style: CursorStyle::Bar,
            cursor_blink: true,
            scrollback_lines: 50_000,
            copy_on_select: true,
            middle_click_paste: true,
            visual_bell: true,
            bell_bounces_dock: false,
            check_for_updates: false,
            sync_auto: false,
            vault_lock_minutes: 30,
            vault_keychain: true,
        };
        saved.save(&dir).unwrap();
        assert_eq!(Settings::load(&dir), saved);
        assert!(!dir.join("settings.json.tmp").exists());
    }

    /// A crash between the temp write and the rename leaves a garbage `.tmp` beside the good
    /// file; `load` reads the committed file and the next `save` replaces the stray.
    #[test]
    fn a_stray_tmp_beside_a_good_file_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let good = Settings {
            sidebar_width: 300.0,
            cursor_blink: true,
            ..Settings::default()
        };
        good.save(dir.path()).unwrap();
        std::fs::write(dir.path().join("settings.json.tmp"), "{ half writ").unwrap();
        assert_eq!(Settings::load(dir.path()), good);

        let next = Settings {
            sidebar_width: 280.0,
            ..good
        };
        next.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), next);
    }

    /// The rename never happened: only the `.tmp` exists. It is not the settings file.
    #[test]
    fn a_tmp_without_the_real_file_is_not_loaded() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("settings.json.tmp"),
            r#"{"sidebarWidth":333}"#,
        )
        .unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
    }

    /// CURRENT behaviour, pinned so a change to it is deliberate: a file that does not parse
    /// is replaced by defaults in memory (see `settings_corrupt_using_defaults`), with no
    /// version to tell "corrupt" from "written by a newer tern". Gap tracked in #28; the
    /// desired behaviour is the ignored test below.
    #[test]
    fn today_an_unparseable_file_silently_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), r#"{"sidebarWidth": "wide""#).unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
    }

    /// DESIRED behaviour (#28): like hosts.json and snippets.json, an unreadable or newer
    /// settings file is kept as it is. Saving what `load` returned must not overwrite it.
    #[test]
    #[ignore = "#28: settings should carry a version and refuse to save over an unreadable/newer file, not silently reset"]
    fn an_unreadable_or_newer_file_is_not_overwritten_by_a_save() {
        for original in [
            r#"{"sidebarWidth": "wide""#,
            r#"{"version": 99, "sidebarWidth": 300, "someFutureField": true}"#,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(FILE_NAME);
            std::fs::write(&path, original).unwrap();
            let _ = Settings::load(dir.path()).save(dir.path());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        }
    }

    #[test]
    fn corrupt_file_gives_defaults() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE_NAME), "{ not json").unwrap();
        assert_eq!(Settings::load(&dir), Settings::default());
    }

    #[test]
    fn absent_fields_take_defaults_and_unknown_fields_are_ignored() {
        let dir = temp_dir("partial");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(FILE_NAME),
            r#"{"sidebarCollapsed":true,"later":1}"#,
        )
        .unwrap();
        let loaded = Settings::load(&dir);
        assert!(loaded.sidebar_collapsed);
        assert_eq!(loaded.sidebar_width, SIDEBAR_DEFAULT);
    }

    #[test]
    fn appearance_follows_the_system_only_when_set_to_system() {
        for system_light in [true, false] {
            assert_eq!(AppearanceMode::System.is_light(system_light), system_light);
            assert!(AppearanceMode::Light.is_light(system_light));
            assert!(!AppearanceMode::Dark.is_light(system_light));
        }
    }

    #[test]
    fn an_older_file_gets_system_appearance_and_the_status_line() {
        let dir = temp_dir("appearance");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE_NAME), r#"{"sidebarWidth":300}"#).unwrap();
        let loaded = Settings::load(&dir);
        assert_eq!(loaded.appearance, AppearanceMode::System);
        assert!(loaded.show_status_line);
        assert!(
            loaded.sharp_text,
            "a file from before the toggle keeps sharp text on"
        );
        std::fs::write(dir.join(FILE_NAME), r#"{"sharpText":false}"#).unwrap();
        assert!(!Settings::load(&dir).sharp_text);
        std::fs::write(dir.join(FILE_NAME), r#"{"appearance":"light"}"#).unwrap();
        assert_eq!(Settings::load(&dir).appearance, AppearanceMode::Light);
    }

    /// `hiddenHosts` was the old "Remove from list" for `~/.ssh/config` hosts; files that still
    /// carry it must load, and saving drops it.
    #[test]
    fn a_settings_file_with_the_retired_hidden_hosts_still_loads() {
        let dir = temp_dir("hidden");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(FILE_NAME),
            r#"{"sidebarWidth":300,"hiddenHosts":["old-box"]}"#,
        )
        .unwrap();
        let loaded = Settings::load(&dir);
        assert_eq!(loaded.sidebar_width, 300.0);
        loaded.save(&dir).unwrap();
        let text = std::fs::read_to_string(dir.join(FILE_NAME)).unwrap();
        assert!(!text.contains("hiddenHosts"), "{text}");
    }

    #[test]
    fn width_is_clamped_on_load() {
        let at = |w: f32| {
            Settings {
                sidebar_width: w,
                ..Settings::default()
            }
            .clamped()
            .sidebar_width
        };
        assert_eq!(at(10.0), SIDEBAR_MIN);
        assert_eq!(at(9000.0), SIDEBAR_MAX);
        assert_eq!(at(300.0), 300.0);
        assert_eq!(at(f32::NAN), SIDEBAR_DEFAULT);
    }

    #[test]
    fn width_is_stored_in_whole_points() {
        let at = |w: f32| {
            Settings {
                sidebar_width: w,
                ..Settings::default()
            }
            .clamped()
            .sidebar_width
        };
        assert_eq!(at(300.4), 300.0);
        assert_eq!(at(300.6), 301.0);
        // Rounding must not push a width out of range.
        assert_eq!(at(SIDEBAR_MAX + 0.4), SIDEBAR_MAX);
    }

    #[test]
    fn font_steps_stop_at_the_bounds() {
        let mut s = Settings::default();
        s.step_font(1.0);
        assert_eq!(s.terminal_font_size, FONT_DEFAULT + 1.0);
        s.terminal_font_size = FONT_MAX;
        s.step_font(1.0);
        assert_eq!(s.terminal_font_size, FONT_MAX);
        s.terminal_font_size = FONT_MIN;
        s.step_font(-1.0);
        assert_eq!(s.terminal_font_size, FONT_MIN);
        s.terminal_font_size = 99.0;
        assert_eq!(s.clamped().terminal_font_size, FONT_MAX);
    }
}
