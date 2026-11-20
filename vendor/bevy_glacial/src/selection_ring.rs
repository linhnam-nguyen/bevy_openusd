//! Animated selection ring — a flat annulus on the ground that
//! spins a fine striped pattern around itself, marking "this is
//! the selected thing".
//!
//! Project-agnostic: the host writes a [`SelectionRing`] resource
//! each frame describing where to draw and at what radius / colour;
//! the [`SelectionRingPlugin`] handles mesh generation, shader
//! uniforms, and visibility. When the resource's `anchor` is `None`
//! the ring is hidden.
//!
//! Mesh is rebuilt only when `outer_radius` or `thickness` change
//! (per-frame mutation of `SelectionRing.color`/`anchor` is free).

use bevy::asset::{Asset, embedded_asset};
use bevy::light::NotShadowCaster;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

pub type SelectionRingMaterial = ExtendedMaterial<StandardMaterial, SelectionRingExtension>;

/// Uniform block for the spinning-ring shader. Fields are packed
/// manually (three `_pad` floats) so the WGSL struct matches the
/// WebGPU std140 layout.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct SelectionRingExtension {
    #[uniform(100)]
    pub color_r: f32,
    #[uniform(100)]
    pub color_g: f32,
    #[uniform(100)]
    pub color_b: f32,
    #[uniform(100)]
    pub time: f32,
    #[uniform(100)]
    pub pulse_speed: f32,
    #[uniform(100)]
    pub pulse_count: f32,
    #[uniform(100)]
    pub alpha: f32,
    #[uniform(100)]
    pub center_x: f32,
    #[uniform(100)]
    pub center_z: f32,
    #[uniform(100)]
    pub fine_mult: f32,
    #[uniform(100)]
    pub _pad2: f32,
    #[uniform(100)]
    pub _pad3: f32,
}

impl Default for SelectionRingExtension {
    fn default() -> Self {
        Self {
            color_r: 0.5,
            color_g: 0.5,
            color_b: 0.5,
            time: 0.0,
            pulse_speed: 3.0,
            pulse_count: 8.0,
            alpha: 1.0,
            center_x: 0.0,
            center_z: 0.0,
            fine_mult: 4.0,
            _pad2: 0.0,
            _pad3: 0.0,
        }
    }
}

impl MaterialExtension for SelectionRingExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://bevy_glacial/shaders/selection_ring.wesl".into()
    }
}

/// Per-frame target — the host system writes this each frame to
/// say "ring this point with this colour at this radius". `anchor`
/// of `None` hides the ring.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct SelectionRing {
    /// World-space position of the ring centre. `None` = hide.
    pub anchor: Option<Vec3>,
    /// Outer radius of the ring, in metres.
    pub outer_radius: f32,
    /// Ring colour (sRGB; converted to linear in the shader).
    pub color: Color,
}

/// User-tweakable ring look. `thickness` is the **world-space**
/// band width — the mesh stays at this thickness regardless of how
/// big the ring is, instead of scaling with the radius.
#[derive(Resource, Copy, Clone, Debug)]
pub struct SelectionRingSettings {
    pub thickness: f32,
}

impl Default for SelectionRingSettings {
    fn default() -> Self {
        Self { thickness: 0.15 }
    }
}

#[derive(Component)]
pub struct SelectionRingEntity {
    pub material: Handle<SelectionRingMaterial>,
    pub mesh: Handle<Mesh>,
    /// Last-built mesh dimensions (outer_radius, thickness) so we
    /// only rebuild when either changes.
    pub built_for: (f32, f32),
}

/// Reference diameter that produces the "drone" look: 8 coarse
/// segments. Bigger rings scale only the COARSE count linearly with
/// diameter so the big-gap cadence stays the same around any ring;
/// the fine-stripe count **per segment** is fixed so each
/// illuminated segment always looks the same density.
const REF_DIAMETER: f32 = 2.5;
const REF_PULSE_COUNT: f32 = 8.0;
/// Stripes per coarse segment, same on every ring.
const FINE_MULT: f32 = 4.0;

/// Angular resolution of the annulus mesh.
const RING_RESOLUTION: u32 = 128;

/// Plugin: registers the material, embeds the WGSL shader, init
/// resources, and adds the ring spawn + per-frame update systems.
pub struct SelectionRingPlugin;

impl Plugin for SelectionRingPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/selection_ring.wesl");
        app.add_plugins(MaterialPlugin::<SelectionRingMaterial>::default())
            .init_resource::<SelectionRing>()
            .init_resource::<SelectionRingSettings>()
            .add_systems(Startup, setup_selection_ring)
            .add_systems(Update, update_selection_ring);
    }
}

fn make_ring_mesh(outer: f32, thickness: f32) -> Mesh {
    let inner = (outer - thickness).max(0.01);
    Annulus::new(inner, outer)
        .mesh()
        .resolution(RING_RESOLUTION)
        .build()
}

fn setup_selection_ring(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SelectionRingMaterial>>,
    settings: Res<SelectionRingSettings>,
) {
    let initial_outer = 1.0_f32;
    let initial_thickness = settings.thickness;
    let mesh = meshes.add(make_ring_mesh(initial_outer, initial_thickness));
    let mat = materials.add(SelectionRingMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            ..default()
        },
        extension: SelectionRingExtension::default(),
    });

    commands.spawn((
        Name::new("SelectionRing"),
        SelectionRingEntity {
            material: mat.clone(),
            mesh: mesh.clone(),
            built_for: (initial_outer, initial_thickness),
        },
        Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
            scale: Vec3::ONE,
        },
        Mesh3d(mesh),
        MeshMaterial3d(mat),
        Visibility::Hidden,
        NotShadowCaster,
    ));
}

fn update_selection_ring(
    target: Res<SelectionRing>,
    settings: Res<SelectionRingSettings>,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SelectionRingMaterial>>,
    mut q: Query<(&mut SelectionRingEntity, &mut Transform, &mut Visibility)>,
) {
    let Ok((mut ring, mut tr, mut vis)) = q.single_mut() else {
        return;
    };

    let Some(anchor) = target.anchor else {
        *vis = Visibility::Hidden;
        return;
    };

    let outer = target.outer_radius.max(0.01);
    let thickness = settings.thickness.max(0.01);

    // Pulse / fine counts derived from diameter so the cadence reads
    // similarly on tiny vs. huge rings.
    let diameter = outer * 2.0;
    let ratio = (diameter / REF_DIAMETER).max(1.0);
    let mut pulse_count = (REF_PULSE_COUNT * ratio).round().max(2.0);
    if (pulse_count as i32) & 1 == 1 {
        pulse_count += 1.0;
    }

    // Rebuild the mesh only when outer / thickness actually change.
    let key = (outer, thickness);
    if (ring.built_for.0 - key.0).abs() > 1e-3 || (ring.built_for.1 - key.1).abs() > 1e-3 {
        if let Some(mut m) = meshes.get_mut(&ring.mesh) {
            *m = make_ring_mesh(outer, thickness);
        }
        ring.built_for = key;
    }

    *vis = Visibility::Visible;
    tr.translation = anchor;
    tr.rotation = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    tr.scale = Vec3::ONE;

    if let Some(mut mat) = materials.get_mut(&ring.material) {
        let linear = target.color.to_linear();
        mat.extension.color_r = linear.red;
        mat.extension.color_g = linear.green;
        mat.extension.color_b = linear.blue;
        mat.extension.time = time.elapsed_secs();
        mat.extension.alpha = 0.9;
        mat.extension.center_x = tr.translation.x;
        mat.extension.center_z = tr.translation.z;
        mat.extension.pulse_count = pulse_count;
        mat.extension.fine_mult = FINE_MULT;
    }
}
