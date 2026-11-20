use bevy::prelude::*;

// ── User-visible settings ───────────────────────────────────────────

#[derive(Resource, Clone, Copy)]
pub struct GroundGrid {
    pub visible: bool,
    /// Base RGB + alpha. Alpha scales everything.
    pub color: Color,
    /// World-space height of the loaded scene's ground reference. `None`
    /// keeps the grid hidden until the host application has a scene bound.
    pub ground_y: Option<f32>,
    /// Desired outer coverage radius for the coarsest level.
    pub coverage_radius: f32,
}

impl Default for GroundGrid {
    fn default() -> Self {
        Self {
            visible: true,
            color: Color::srgba(0.62, 0.76, 0.95, 0.6),
            ground_y: None,
            coverage_radius: LEVEL_HALF[LEVEL_HALF.len() - 1],
        }
    }
}

/// Diagnostic observability counters for GroundGrid rebuild operations.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct GlacialGridCounters {
    pub alpha_rebuild_calls: u64,
    pub lines_rebuilt: u64,
    pub dots_rebuilt: u64,
    pub vertices_generated: u64,
    pub indices_generated: u64,
}

/// Last mesh-baked grid style successfully synchronized into the mesh assets.
#[derive(Resource, Default)]
pub struct GroundGridMeshStyle {
    pub(crate) color: Option<Color>,
}

// ── LOD levels ──────────────────────────────────────────────────────

pub const LEVEL_SCALE: f32 = 4.0;
pub const BASE_STEP: f32 = 0.5;

pub const LEVEL_STEPS: [f32; 4] = [
    BASE_STEP,
    BASE_STEP * LEVEL_SCALE,
    BASE_STEP * LEVEL_SCALE * LEVEL_SCALE,
    BASE_STEP * LEVEL_SCALE * LEVEL_SCALE * LEVEL_SCALE,
];

pub const LINES_PER_SIDE: [f32; 4] = [100.0, 60.0, 35.0, 20.0];

pub const LEVEL_HALF: [f32; 4] = [
    LINES_PER_SIDE[0] * LEVEL_STEPS[0],
    LINES_PER_SIDE[1] * LEVEL_STEPS[1],
    LINES_PER_SIDE[2] * LEVEL_STEPS[2],
    LINES_PER_SIDE[3] * LEVEL_STEPS[3],
];

pub const MAJOR_EVERY: i32 = LEVEL_SCALE as i32;
pub const MAJOR_BOOST: f32 = 3.5;
pub const EDGE_FADE_FRAC: f32 = 0.92;
pub const GAUSS_PEAK: f32 = 0.602_06; // log10(4)
pub const GAUSS_WIDTH: f32 = 0.55;
pub const LINE_CLOSE_FALLOFF: f32 = 2.5;
pub const DOT_CLOSE_FALLOFF: f32 = 6.0;

// ── Components ──────────────────────────────────────────────────────

#[derive(Component, Copy, Clone, PartialEq, Eq, Debug)]
pub enum GridKind {
    Lines,
    Dots,
}

#[derive(Component)]
pub struct LocalGrid {
    pub level: u8,
    pub kind: GridKind,
    pub material: Handle<StandardMaterial>,
}

pub const DOT_RADIUS_FRAC: f32 = 0.024;
pub const DOT_SEGMENTS: u32 = 8;
pub const DOT_OFFSET_FRAC: f32 = 0.0001;
