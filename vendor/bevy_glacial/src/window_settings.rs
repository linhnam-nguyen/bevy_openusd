//! Window-geometry persistence — remembers the Bevy primary window's
//! size + position across runs so apps don't boot into a default
//! tiny pane in the top-left every session.
//!
//! Plain-text `key=value` format on disk so the config is diffable
//! and editable by hand. File lives under
//! `${XDG_CONFIG_HOME:-~/.config}/<app_name>/window.txt`.
//!
//! Pattern:
//!
//! ```ignore
//! use bevy::prelude::*;
//! use bevy_glacial::prelude::*;
//!
//! fn main() {
//!     let geometry = WindowGeometry::load("my_app");
//!
//!     App::new()
//!         .add_plugins(DefaultPlugins.set(WindowPlugin {
//!             primary_window: Some(geometry.to_window("My App")),
//!             ..default()
//!         }))
//!         .add_plugins(WindowSettingsPlugin::new("my_app"))
//!         .run();
//! }
//! ```
//!
//! The plugin is intentionally **not** part of [`GlacialPlugins`] —
//! it needs an app-specific name and a place to persist state, both
//! of which the host app must opt into.

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowMoved, WindowPosition, WindowResized};
use std::env;
use std::fs;
use std::path::PathBuf;

/// Plugin: writes the primary window's size + position to
/// `${XDG_CONFIG_HOME:-~/.config}/<app_name>/window.txt` whenever
/// the user moves or resizes the window.
pub struct WindowSettingsPlugin {
    app_name: String,
}

impl WindowSettingsPlugin {
    /// `app_name` becomes the config sub-directory — pick a short,
    /// filesystem-safe identifier matching your binary.
    pub fn new(app_name: impl Into<String>) -> Self {
        Self {
            app_name: app_name.into(),
        }
    }
}

impl Plugin for WindowSettingsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WindowSettingsConfig {
            app_name: self.app_name.clone(),
        })
        .add_systems(Update, persist_window_geometry);
    }
}

#[derive(Resource, Clone)]
struct WindowSettingsConfig {
    app_name: String,
}

/// On-disk representation of the primary window: pixel size and
/// top-left position (logical pixels, as reported by Bevy).
#[derive(Debug, Clone, Copy)]
pub struct WindowGeometry {
    pub width: f32,
    pub height: f32,
    pub position: IVec2,
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self {
            width: 1280.0,
            height: 800.0,
            position: IVec2::new(120, 120),
        }
    }
}

impl WindowGeometry {
    /// Read the saved geometry for `app_name`. Falls back to the
    /// default (1280×800 at (120, 120)) if the file is missing or
    /// malformed.
    pub fn load(app_name: &str) -> Self {
        let path = config_path(app_name);
        let Ok(contents) = fs::read_to_string(path) else {
            return Self::default();
        };
        parse_geometry(&contents).unwrap_or_default()
    }

    /// Build a Bevy [`Window`] from this geometry, with the given
    /// title. All other window fields stay at Bevy defaults — set
    /// them on the returned struct if you need to tweak.
    pub fn to_window(self, title: impl Into<String>) -> Window {
        Window {
            title: title.into(),
            resolution: (self.width.round() as u32, self.height.round() as u32).into(),
            position: WindowPosition::At(self.position),
            ..default()
        }
    }
}

fn persist_window_geometry(
    primary_window: Query<(Entity, &Window), With<PrimaryWindow>>,
    mut moved_events: MessageReader<WindowMoved>,
    mut resized_events: MessageReader<WindowResized>,
    config: Res<WindowSettingsConfig>,
) {
    let Ok((window_entity, window)) = primary_window.single() else {
        return;
    };
    let mut dirty = false;

    for event in moved_events.read() {
        if event.window == window_entity {
            dirty = true;
        }
    }
    for event in resized_events.read() {
        if event.window == window_entity {
            dirty = true;
        }
    }
    if !dirty {
        return;
    }

    let WindowPosition::At(position) = window.position else {
        return;
    };

    let geometry = WindowGeometry {
        width: window.resolution.width(),
        height: window.resolution.height(),
        position,
    };
    let _ = save_geometry(&config.app_name, geometry);
}

fn save_geometry(app_name: &str, geometry: WindowGeometry) -> std::io::Result<()> {
    let path = config_path(app_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        path,
        format!(
            "width={}\nheight={}\nx={}\ny={}\n",
            geometry.width, geometry.height, geometry.position.x, geometry.position.y
        ),
    )
}

fn parse_geometry(contents: &str) -> Option<WindowGeometry> {
    let mut width = None;
    let mut height = None;
    let mut x = None;
    let mut y = None;

    for line in contents.lines() {
        let (key, value) = line.split_once('=')?;
        match key.trim() {
            "width" => width = value.trim().parse::<f32>().ok(),
            "height" => height = value.trim().parse::<f32>().ok(),
            "x" => x = value.trim().parse::<i32>().ok(),
            "y" => y = value.trim().parse::<i32>().ok(),
            _ => {}
        }
    }

    Some(WindowGeometry {
        width: width?,
        height: height?,
        position: IVec2::new(x?, y?),
    })
}

fn config_path(app_name: &str) -> PathBuf {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(app_name).join("window.txt")
}
