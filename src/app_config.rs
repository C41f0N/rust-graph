use crate::config;
use crate::graph::processing::{
    param_values, set_param_values, ALPHA_COOLING_ENABLED, PARAM_KEYS, SHOW_FORCE_PANEL,
};
use std::path::{Path, PathBuf};

// One global key=value config file in the OS's appdata/config directory
// (via `dirs::config_dir()`: ~/.config on Linux, ~/Library/Application
// Support on macOS, %APPDATA% on Windows). Holds everything the app persists:
// window size, text zoom, selected font and the graph force values - one set
// shared by every folder opened. All graphs use the same values; nothing goes
// inside the graph directory anymore.

// Sub-folder and file name under the config dir. Kept short and stable so the
// file's identity never changes with the binary name.
const APP_DIR: &str = "rust-graph";
const CONFIG_FILE: &str = "config";

#[derive(Clone)]
pub struct AppConfig {
    pub window: (i32, i32),
    pub zoom: f32,
    pub font_name: String,
    pub graph: [f32; 9],
    pub cooling: bool,
    pub panel: bool,
}

impl AppConfig {
    // Snapshot the current live state (used as load defaults and as the save
    // payload). Reads statics only; no IO. Pure enough for tests to run
    // single-threaded alongside the rest.
    fn from_statics() -> AppConfig {
        let (w, h) = *config::SCREEN_SIZE.read().unwrap();
        AppConfig {
            window: (w, h),
            zoom: config::zoom(),
            font_name: current_font_name(),
            graph: param_values(),
            cooling: *ALPHA_COOLING_ENABLED.read().unwrap(),
            panel: *SHOW_FORCE_PANEL.read().unwrap(),
        }
    }

    pub fn apply(&self) {
        set_param_values(self.graph);
        *ALPHA_COOLING_ENABLED.write().unwrap() = self.cooling;
        *SHOW_FORCE_PANEL.write().unwrap() = self.panel;
        *config::TEXT_ZOOM.write().unwrap() =
            self.zoom.clamp(config::TEXT_ZOOM_MIN, config::TEXT_ZOOM_MAX);
        apply_font(self.font_name.clone());
    }
}

// Name of the font the font picker currently points at, or "" for the
// built-in default. The settings dialog only runs after FONTS is built, so
// save() reads a real name; load-time defaults get "" when FONTS is empty.
fn current_font_name() -> String {
    let fonts = crate::graph::settings::FONTS.read().unwrap();
    match *crate::graph::settings::SELECTED_FONT.read().unwrap() {
        None | Some(0) => String::new(),
        Some(i) => fonts.get(i).map(|f| f.name.clone()).unwrap_or_default(),
    }
}

// Reinstate a persisted font family: point the picker at it and queue the
// load request (main.rs owns the raylib handle and fulfils it on the GL
// thread). Plays no-op while FONTS isn't built yet, and when the family no
// longer exists on this system.
fn apply_font(name: String) {
    let fonts = crate::graph::settings::FONTS.read().unwrap();
    let request = if name.is_empty() {
        Some(0)
    } else {
        fonts.iter()
            .position(|f| f.name == name)
            .or(Some(0))
    };
    if let Some(idx) = request {
        if !fonts.is_empty() {
            *crate::graph::settings::SELECTED_FONT.write().unwrap() = Some(idx);
            *crate::graph::settings::REQUEST_LOAD_FONT.write().unwrap() = Some(idx);
        }
    }
}

/// Absolute path of the global config file. None when the OS can't tell us its
/// config directory (some embedded/sandboxed platforms) - the app then runs
/// without persistence.
pub fn config_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join(APP_DIR).join(CONFIG_FILE))
}

/// True when a global config file already exists (used to decide whether a
/// legacy per-folder .graph-params import should seed it on first run).
pub fn has_global_config() -> bool {
    config_path()
        .map(|p| p.is_file())
        .unwrap_or(false)
}

/// Read and parse the global config, falling back to the current statics when
/// the file is missing or unreadable.
pub fn load() -> AppConfig {
    let path = match config_path() {
        Some(p) => p,
        None => return AppConfig::from_statics(),
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text, AppConfig::from_statics()),
        Err(_) => AppConfig::from_statics(),
    }
}

/// Write the current live state to the global config file. Written to a temp
/// sibling then renamed, so a crash can't leave a half-written file. The
/// parent folder is created on demand.
pub fn save() {
    let Some(parent) = config_path().and_then(|p| p.parent().map(|pd| pd.to_path_buf())) else {
        return;
    };
    if std::fs::create_dir_all(&parent).is_err() {
        return;
    }
    let path = parent.join(CONFIG_FILE);
    let text = serialize(&AppConfig::from_statics());
    let tmp = parent.join(format!(".{}.tmp", CONFIG_FILE));
    if std::fs::write(&tmp, &text).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

// ---- Legacy migration -------------------------------------------------------
// Before this feature, force values lived in a `<folder>/.graph-params` file.
// On first run without a global config, those values are imported once and the
// file is removed so the folder goes back to being pure notes. This only fires
// when no global config exists yet.

const LEGACY_FILE: &str = ".graph-params";

/// Read a legacy `.graph-params` file (if any) and overlay its graph keys on
/// the passed config. Returns None when the folder has no such file.
pub fn import_legacy_graph_params(dir: &Path) -> Option<AppConfig> {
    let path = dir.join(LEGACY_FILE);
    let text = std::fs::read_to_string(&path).ok()?;
    let cfg = parse_graph_keys(&text, AppConfig::from_statics());
    // The file is consumed by the import: delete it so it can't be picked up
    // again next launch or confuse the folder.
    let _ = std::fs::remove_file(&path);
    Some(cfg)
}

/// Parse just the graph block of the config (force values + the two booleans)
/// out of arbitrary text over `base`, leaving every other key untouched. This
/// is the seam both the full config parser and the legacy import share.
fn parse_graph_keys(text: &str, mut base: AppConfig) -> AppConfig {
    for key in PARAM_KEYS {
        if let Some(v) = line_value(text, key).and_then(|v| v.trim().parse::<f32>().ok()) {
            let idx = PARAM_KEYS.iter().position(|&k| k == key).unwrap();
            base.graph[idx] = v;
        }
    }
    if line_value(text, "alpha_cooling") == Some("1") {
        base.cooling = true;
    } else if line_value(text, "alpha_cooling") == Some("0") {
        base.cooling = false;
    }
    if line_value(text, "show_force_panel") == Some("1") {
        base.panel = true;
    } else if line_value(text, "show_force_panel") == Some("0") {
        base.panel = false;
    }
    base
}

// ---- Pure (de)serialization -------------------------------------------------

/// Value of the first `key=` line in `text`, trimmed, or None.
fn line_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|raw| {
        let line = raw.trim();
        let (k, v) = line.split_once('=')?;
        if k == key {
            Some(v.trim())
        } else {
            None
        }
    })
}

/// Serialize the config to key=value text. Floats keep 4 decimals so every
/// config round-trips bit-stable.
pub fn serialize(cfg: &AppConfig) -> String {
    let mut out = String::new();
    for (i, key) in PARAM_KEYS.iter().enumerate() {
        out.push_str(&format!("{key}={:.4}\n", cfg.graph[i]));
    }
    out.push_str(&format!(
        "alpha_cooling={}\nshow_force_panel={}\n",
        if cfg.cooling { 1 } else { 0 },
        if cfg.panel { 1 } else { 0 },
    ));
    out.push_str(&format!("window_width={}\n", cfg.window.0));
    out.push_str(&format!("window_height={}\n", cfg.window.1));
    out.push_str(&format!("text_zoom={:.4}\n", cfg.zoom));
    out.push_str(&format!("font_name={}\n", cfg.font_name));
    out
}

/// Parse config text over `defaults`. Missing/unknown/junk keys keep the
/// default slot, so hand-edited files degrade gracefully.
pub fn parse(text: &str, defaults: AppConfig) -> AppConfig {
    let mut cfg = parse_graph_keys(text, defaults);
    if let Some(v) = line_value(text, "window_width").and_then(|v| v.trim().parse::<i32>().ok()) {
        cfg.window.0 = v.max(config::MIN_WINDOW_W);
    }
    if let Some(v) = line_value(text, "window_height").and_then(|v| v.trim().parse::<i32>().ok()) {
        cfg.window.1 = v.max(config::MIN_WINDOW_H);
    }
    if let Some(v) = line_value(text, "text_zoom").and_then(|v| v.trim().parse::<f32>().ok()) {
        cfg.zoom = v.clamp(config::TEXT_ZOOM_MIN, config::TEXT_ZOOM_MAX);
    }
    if let Some(v) = line_value(text, "font_name") {
        if !v.is_empty() {
            cfg.font_name = v.to_string();
        }
    }
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> AppConfig {
        AppConfig {
            window: (1920, 1080),
            zoom: 1.0,
            font_name: "Fira Code".to_string(),
            graph: [0.9, 0.95, 0.1, 360.0, 10000.0, 0.0228, 1.0, 0.0, 0.05],
            cooling: true,
            panel: true,
        }
    }

    #[test]
    fn round_trip_through_serialization() {
        let cfg = base();
        let text = serialize(&cfg);
        let got = parse(&text, AppConfig::from_statics());
        assert_eq!(got.window, cfg.window);
        assert_eq!(got.zoom, 1.0);
        assert_eq!(got.font_name, "Fira Code");
        assert_eq!(got.graph, cfg.graph);
        assert!(got.cooling);
        assert!(got.panel);
    }

    #[test]
    fn partial_file_keeps_defaults_and_ignores_stale_keys() {
        // A leftover legacy `rest_gap=` line plus junk keys must be ignored,
        // everything absent falls back to the defaults.
        let text = "spring_k=2.25\nnot-a-param=99\nbogus\nalpha_cooling=0\n      \n\
                    rest_gap=45.0\nwindow_width=640\nwindow_height=900\n";
        let defaults = base();
        let got = parse(text, defaults.clone());
        assert_eq!(got.graph[0], 2.25);
        assert_eq!(got.graph[5], defaults.graph[5], "alpha_decay untouched");
        // rest_gap is not a known key: slot 5 must not move.
        assert_eq!(got.graph[5], 0.0228);
        assert!(!got.cooling);
        assert!(got.panel);
        // window_height fits, the undersized width is clamped to the floor.
        assert_eq!(got.window.0, config::MIN_WINDOW_W);
        assert_eq!(got.window.1, 900);
        assert_eq!(got.font_name, "Fira Code");
    }

    #[test]
    fn missing_file_defaults_to_statics() {
        let got = parse("", AppConfig::from_statics());
        assert_eq!(got.window, *config::SCREEN_SIZE.read().unwrap());
        assert_eq!(got.zoom, config::zoom());
        assert_eq!(got.graph, param_values());
    }

    #[test]
    fn legacy_graph_params_import_only_touches_graph_block() {
        // Old .graph-params file: only graph keys present. App keys keep the
        // base (an existing global config's values, say).
        let text = "spring_k=2.25\ndamping=0.91\ncenter_pull=0.3\nrepulsion_radius=400.\n\
                    repulsion_k=12300.\nalpha_decay=0.02\nradius_scale=1.1\nradius_variation=0.2\n\
                    attraction=0.08\nalpha_cooling=0\nshow_force_panel=1\n";
        let got = parse_graph_keys(text, base());
        let expect = [2.25, 0.91, 0.3, 400.0, 12300.0, 0.02, 1.1, 0.2, 0.08];
        assert_eq!(got.graph, expect);
        assert!(!got.cooling);
        assert!(got.panel);
        assert_eq!(got.window, base().window, "app keys untouched");
        assert_eq!(got.font_name, "Fira Code");
    }

    #[test]
    fn config_path_resolves_within_fs() {
        let path = config_path().expect("config dir must resolve on this platform");
        assert_eq!(path.file_name().map(|n| n.to_string_lossy().into_owned()), Some("config".into()));
        assert_eq!(
            path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()),
            Some(APP_DIR.into())
        );
    }

    fn seed_fonts(names: &[&str]) {
        use crate::graph::settings::{FontFamily, FONTS};
        *FONTS.write().unwrap() = names
            .iter()
            .map(|n| FontFamily {
                name: n.to_string(),
                path: None,
                data: None,
            })
            .collect();
    }

    #[test]
    fn selected_font_persists_through_snapshot_and_apply() {
        use crate::graph::settings::{REQUEST_LOAD_FONT, SELECTED_FONT};
        seed_fonts(&["(Default)", "Fira Code", "DejaVu Sans"]);

        *SELECTED_FONT.write().unwrap() = Some(2);
        let snap = AppConfig::from_statics();
        assert_eq!(snap.font_name, "DejaVu Sans");

        *SELECTED_FONT.write().unwrap() = Some(0);
        *REQUEST_LOAD_FONT.write().unwrap() = None;
        snap.apply();
        assert_eq!(*SELECTED_FONT.read().unwrap(), Some(2));
        assert_eq!(*REQUEST_LOAD_FONT.read().unwrap(), Some(2));
    }

    #[test]
    fn apply_falls_back_to_default_for_missing_or_empty_font() {
        use crate::graph::settings::{REQUEST_LOAD_FONT, SELECTED_FONT};
        seed_fonts(&["(Default)", "Fira Code"]);

        // Family gone from this system: picker and load request revert to 0.
        let stale = AppConfig {
            font_name: "Nope".to_string(),
            ..AppConfig::from_statics()
        };
        stale.apply();
        assert_eq!(*SELECTED_FONT.read().unwrap(), Some(0));
        assert_eq!(*REQUEST_LOAD_FONT.read().unwrap(), Some(0));

        // Empty name (default font): same, and the serialization keeps it "".
        let defaulted = AppConfig {
            font_name: String::new(),
            ..AppConfig::from_statics()
        };
        defaulted.apply();
        assert_eq!(*SELECTED_FONT.read().unwrap(), Some(0));
        assert!(serialize(&defaulted).contains("font_name=\n"));
    }
}