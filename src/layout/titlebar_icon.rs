//! Application icon and name for window titlebars.
//!
//! The desktop entry is found by the window app ID; it gives the application name and icon name.
//! The icon is looked up in the user icon theme (following `Inherits`), then in hicolor and
//! pixmaps. SVG icons are converted to PNG with `rsvg-convert` and cached in
//! `$XDG_CACHE_HOME/niri-staking/icons`; the icon theme name comes from `gsettings`. When these
//! programs are missing, titlebars simply show no icon.
//!
//! The lookup scans icon theme directories, so it runs on a background thread. Until it finishes,
//! the titlebar is drawn without the icon and the application name.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Size of cached PNG icons, enough for scale 2.
const ICON_PX: u32 = 64;

#[derive(Debug, Default)]
pub struct AppInfo {
    /// Application name from the desktop entry.
    pub name: Option<String>,
    /// Icon decoded for cairo.
    pub icon: Option<Arc<IconImage>>,
}

/// Icon pixels in the cairo ARgb32 format: premultiplied B, G, R, A.
#[derive(Debug)]
pub struct IconImage {
    pub data: Vec<u8>,
    pub width: i32,
    pub height: i32,
}

/// Decodes a PNG into the cairo format.
fn decode_png(path: &Path) -> Option<IconImage> {
    let file = std::io::BufReader::new(fs::File::open(path).ok()?);
    let mut decoder = png::Decoder::new(file);
    decoder.set_transformations(
        png::Transformations::EXPAND | png::Transformations::STRIP_16 | png::Transformations::ALPHA,
    );
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => return None,
    };
    let mut data = Vec::with_capacity(w * h * 4);
    for px in buf[..w * h * channels].chunks_exact(channels) {
        let (r, g, b, a) = match channels {
            4 => (px[0], px[1], px[2], px[3]),
            3 => (px[0], px[1], px[2], 255),
            2 => (px[0], px[0], px[0], px[1]),
            _ => (px[0], px[0], px[0], 255),
        };
        let pm = |c: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
        data.extend_from_slice(&[pm(b), pm(g), pm(r), a]);
    }
    Some(IconImage {
        data,
        width: w as i32,
        height: h as i32,
    })
}

/// Lookup results by app ID; `None` means the lookup is in progress.
static FOUND: Mutex<Option<HashMap<String, Option<Arc<AppInfo>>>>> = Mutex::new(None);

/// Returns the application name and icon if the lookup has finished. The first call for an app ID
/// starts the lookup on a background thread.
pub fn app_info(app_id: &str) -> Option<Arc<AppInfo>> {
    let mut found = FOUND.lock().unwrap();
    let map = found.get_or_insert_with(HashMap::new);
    if let Some(entry) = map.get(app_id) {
        return entry.clone();
    }
    map.insert(app_id.to_owned(), None);
    let id = app_id.to_owned();
    std::thread::spawn(move || {
        let info = Arc::new(lookup(&id));
        if let Some(map) = FOUND.lock().unwrap().as_mut() {
            map.insert(id, Some(info));
        }
    });
    None
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![home().join(".local/share")];
    let system =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(
        system
            .split(':')
            .filter(|d| !d.is_empty())
            .map(PathBuf::from),
    );
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share"));
    dirs.push(home().join(".local/share/flatpak/exports/share"));
    dirs
}

/// Desktop entry fields that matter here.
struct DesktopEntry {
    name: Option<String>,
    icon: Option<String>,
    wm_class: Option<String>,
}

/// Returns the `Name` keys to try, most specific locale first (`ru_RU`, then `ru`).
fn locale_name_keys() -> Vec<String> {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .find(|value| !value.is_empty())
        .unwrap_or_default();
    let locale = locale.split(['.', '@']).next().unwrap_or_default();
    let mut keys = Vec::new();
    if !locale.is_empty() && locale != "C" && locale != "POSIX" {
        keys.push(format!("Name[{locale}]"));
        if let Some((lang, _)) = locale.split_once('_') {
            keys.push(format!("Name[{lang}]"));
        }
    }
    keys.push(String::from("Name"));
    keys
}

fn parse_desktop(path: &Path, name_keys: &[String]) -> Option<DesktopEntry> {
    let text = fs::read_to_string(path).ok()?;
    let mut in_entry = false;
    let mut names = vec![None; name_keys.len()];
    let (mut icon, mut wm_class) = (None, None);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim().to_owned());
        match key {
            "Icon" => icon = Some(value),
            "StartupWMClass" => wm_class = Some(value),
            _ => {
                if let Some(idx) = name_keys.iter().position(|k| k == key) {
                    names[idx] = Some(value);
                }
            }
        }
    }
    Some(DesktopEntry {
        name: names.into_iter().flatten().next(),
        icon,
        wm_class,
    })
}

/// Finds the desktop entry by file name, by `StartupWMClass`, or by the last component of the app
/// ID (`org.gnome.Nautilus` matches `nautilus.desktop`).
fn find_desktop(app_id: &str) -> Option<DesktopEntry> {
    let id = app_id.to_lowercase();
    let short = id.rsplit('.').next().unwrap_or(&id).to_owned();
    let name_keys = locale_name_keys();
    let mut by_short = None;
    for dir in data_dirs() {
        let Ok(entries) = fs::read_dir(dir.join("applications")) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "desktop") {
                continue;
            }
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            if stem == id {
                return parse_desktop(&path, &name_keys);
            }
            if let Some(desk) = parse_desktop(&path, &name_keys) {
                if desk
                    .wm_class
                    .as_deref()
                    .is_some_and(|c| c.to_lowercase() == id)
                {
                    return Some(desk);
                }
                if by_short.is_none() && stem.rsplit('.').next() == Some(short.as_str()) {
                    by_short = Some(desk);
                }
            }
        }
    }
    by_short
}

/// Returns the user icon theme name.
fn icon_theme() -> Option<String> {
    let out = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
        .ok()?;
    let theme = String::from_utf8_lossy(&out.stdout)
        .trim()
        .trim_matches('\'')
        .to_owned();
    (!theme.is_empty()).then_some(theme)
}

fn theme_dirs(theme: &str) -> Vec<PathBuf> {
    let mut dirs = vec![home().join(".icons").join(theme)];
    dirs.extend(data_dirs().into_iter().map(|d| d.join("icons").join(theme)));
    dirs.into_iter().filter(|d| d.is_dir()).collect()
}

fn inherits(theme: &str) -> Vec<String> {
    for dir in theme_dirs(theme) {
        if let Ok(text) = fs::read_to_string(dir.join("index.theme")) {
            if let Some(line) = text.lines().find(|l| l.starts_with("Inherits=")) {
                return line["Inherits=".len()..]
                    .split(',')
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }
    }
    Vec::new()
}

/// Returns the icon themes to search: the user theme, its ancestors, then hicolor.
fn theme_chain() -> Vec<String> {
    let mut chain = Vec::new();
    let mut todo: Vec<String> = icon_theme().into_iter().collect();
    while let Some(theme) = todo.pop() {
        if chain.contains(&theme) || chain.len() > 8 {
            continue;
        }
        let parents = inherits(&theme);
        chain.push(theme);
        todo.extend(parents.into_iter().rev());
    }
    if !chain.iter().any(|t| t == "hicolor") {
        chain.push("hicolor".into());
    }
    chain
}

/// Collects all `name.png` and `name.svg` files in an icon theme directory.
fn collect(dir: &Path, name: &str, depth: u32, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                collect(&path, name, depth - 1, found);
            }
        } else if path.file_stem().is_some_and(|s| s == name)
            && path.extension().is_some_and(|e| e == "png" || e == "svg")
        {
            found.push(path);
        }
    }
}

/// Ranks an icon file: PNGs from 32 to 128 pixel directories are best, SVGs scale to any size.
fn score(path: &Path) -> u32 {
    if path.extension().is_some_and(|e| e == "svg") {
        return 50;
    }
    let size = path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .find_map(|c| c.split('x').next().and_then(|n| n.parse::<u32>().ok()))
        .unwrap_or(48);
    match size {
        32..=128 => 100,
        129.. => 70,
        _ => 30,
    }
}

fn find_icon(icon: &str) -> Option<PathBuf> {
    let path = Path::new(icon);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_owned());
    }
    for theme in theme_chain() {
        let mut found = Vec::new();
        for dir in theme_dirs(&theme) {
            collect(&dir, icon, 4, &mut found);
        }
        if let Some(best) = found.into_iter().max_by_key(|p| score(p)) {
            return Some(best);
        }
    }
    for dir in data_dirs() {
        for ext in ["png", "svg"] {
            let path = dir.join("pixmaps").join(format!("{icon}.{ext}"));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// Converts an SVG icon to a cached PNG.
fn svg_to_png(svg: &Path) -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".cache"));
    let dir = cache.join("niri-staking/icons");
    let name: String = svg
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let png = dir.join(format!("{name}-{ICON_PX}.png"));
    if png.is_file() {
        return Some(png);
    }
    fs::create_dir_all(&dir).ok()?;
    let tmp = png.with_extension("tmp");
    let ok = Command::new("rsvg-convert")
        .args(["-w", &ICON_PX.to_string(), "-h", &ICON_PX.to_string(), "-o"])
        .arg(&tmp)
        .arg(svg)
        .status()
        .is_ok_and(|s| s.success());
    (ok && fs::rename(&tmp, &png).is_ok()).then_some(png)
}

fn lookup(app_id: &str) -> AppInfo {
    let desk = find_desktop(app_id);
    let icon_name = desk
        .as_ref()
        .and_then(|d| d.icon.clone())
        .unwrap_or_else(|| app_id.to_owned());
    let icon = find_icon(&icon_name)
        .and_then(|path| {
            if path.extension().is_some_and(|e| e == "svg") {
                svg_to_png(&path)
            } else {
                Some(path)
            }
        })
        .and_then(|png| decode_png(&png))
        .map(Arc::new);
    AppInfo {
        name: desk.and_then(|d| d.name),
        icon,
    }
}
