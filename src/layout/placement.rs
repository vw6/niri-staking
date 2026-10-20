//! Remembered window placement per application in stacking mode.
//!
//! When a window closes, its position and size are stored by app ID, and the next window of that
//! application opens the same way. Stored in `$XDG_STATE_HOME/niri-staking/placement.json`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Tile position relative to the working area and window size. For a maximized window, these are
/// from before it was maximized.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    pub w: i32,
    pub h: i32,
    #[serde(default)]
    pub maximized: bool,
}

static SAVED: Mutex<Option<HashMap<String, Placement>>> = Mutex::new(None);

fn path() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".local/state")
        });
    state.join("niri-staking/placement.json")
}

fn with_saved<R>(f: impl FnOnce(&mut HashMap<String, Placement>) -> R) -> R {
    let mut saved = SAVED.lock().unwrap();
    let map = saved.get_or_insert_with(|| {
        fs::read(path())
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    });
    f(map)
}

pub fn get(app_id: &str) -> Option<Placement> {
    with_saved(|map| map.get(app_id).copied())
}

pub fn set(app_id: &str, placement: Placement) {
    if placement.w <= 0 || placement.h <= 0 {
        return;
    }
    let data = with_saved(|map| {
        map.insert(app_id.to_owned(), placement);
        serde_json::to_vec_pretty(map).ok()
    });
    let Some(data) = data else { return };
    let path = path();
    let tmp = path.with_extension("tmp");
    let res = path
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(&tmp, data))
        .and_then(|()| fs::rename(&tmp, &path));
    if let Err(err) = res {
        warn!("error writing window placement to {path:?}: {err:?}");
    }
}
