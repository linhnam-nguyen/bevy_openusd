use super::*;
use crate::live::{LiveStage, PrimEntities, project_stage};
use crate::route::SchemaRegistry;
use openusd::usd::Stage;

#[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Component, Default)]
struct Health {
    current: f64,
    max: f64,
}

#[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Component, Default)]
struct Placement {
    offset: Vec3,
}

fn world_with(register: impl FnOnce(&mut TypeRegistry)) -> World {
    let mut world = World::new();
    let type_registry = AppTypeRegistry::default();
    register(&mut type_registry.write());
    world.insert_resource(type_registry);
    world.insert_resource(SchemaRegistry::builtin());
    world
}

/// A component authored back into the stage re-reads (via the reflect
/// route) into an identical component — a full ECS → USD → ECS round-trip,
/// the thing BSN cannot do. Also proves the opinions land in an exportable
/// layer.
#[test]
fn author_component_roundtrips_through_usd() {
    let stage = Stage::builder().in_memory("author.usda").unwrap();
    stage
        .define_prim("/Enemy")
        .unwrap()
        .set_type_name("Xform")
        .unwrap();

    // Source world holds the component; author it onto the stage.
    let mut src = world_with(|r| {
        r.register::<Health>();
        r.register::<Placement>();
    });
    let e = src
        .spawn((
            crate::UsdPrimRef::new("/Enemy"),
            Health {
                current: 30.0,
                max: 120.0,
            },
            Placement {
                offset: Vec3::new(1.0, 2.0, 3.0),
            },
        ))
        .id();
    author_component_from_world(&src, &stage, e, "/Enemy", "Health").unwrap();
    author_component_from_world(&src, &stage, e, "/Enemy", "Placement").unwrap();

    // The opinions are really in the layer.
    let text = crate::authoring::export_stage_string(&stage).unwrap();
    assert!(
        text.contains("bevy:Health:max"),
        "authored attr present in exported layer:\n{text}"
    );

    // Re-project the stage into a fresh world → identical components.
    let live = LiveStage::new(stage);
    let mut dst = world_with(|r| {
        r.register::<Health>();
        r.register::<Placement>();
    });
    let mut map = PrimEntities::default();
    project_stage(&mut dst, &live, &mut map);
    let re = map.entity("/Enemy").unwrap();
    assert_eq!(
        dst.get::<Health>(re),
        Some(&Health {
            current: 30.0,
            max: 120.0
        }),
        "Health round-trips ECS → USD → ECS"
    );
    assert_eq!(
        dst.get::<Placement>(re),
        Some(&Placement {
            offset: Vec3::new(1.0, 2.0, 3.0)
        }),
        "nested Vec3 field round-trips as float3"
    );
}

#[derive(Reflect, Default, Debug, Clone, PartialEq)]
enum Mode {
    #[default]
    Idle,
    Run,
}

#[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Component, Default)]
struct Mixed {
    flag: bool,
    count: i32,
    label: String,
    mode: Mode,
    spin: Quat,
}

#[derive(Reflect, Default, Debug, Clone, PartialEq)]
struct Inner {
    a: f32,
    b: f32,
}

#[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Component, Default)]
struct Outer {
    inner: Inner,
    tag: i32,
}

#[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Component, Default)]
struct Tint {
    color: Color,
}

#[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Component, Default)]
struct Opts {
    maybe_hp: Option<f32>,
    maybe_name: Option<String>,
    count: u64,
    idx: usize,
}

#[derive(Reflect, Default, Debug, Clone, PartialEq)]
enum Motion {
    #[default]
    Idle,
    Moving(f32),
    Warp {
        x: i32,
        y: i32,
    },
}

#[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Component, Default)]
struct Mover {
    motion: Motion,
}

/// `Option<T>` (Some), `u64`, and `usize` fields round-trip; a `None` option
/// authors nothing and re-reads as the default.
#[test]
fn option_and_wide_int_roundtrip() {
    let value = Opts {
        maybe_hp: Some(12.5),
        maybe_name: Some("boss".into()),
        count: 9_000_000_000,
        idx: 7,
    };
    let back = roundtrip(|r| r.register::<Opts>(), value.clone());
    assert_eq!(back, value, "Some options + u64/usize round-trip");

    // A None option authors nothing → re-reads as the default (None).
    let none = Opts {
        maybe_hp: None,
        maybe_name: None,
        count: 1,
        idx: 2,
    };
    let back = roundtrip(|r| r.register::<Opts>(), none.clone());
    assert_eq!(back, none, "None options stay None through the round-trip");
}

#[test]
fn data_enum_roundtrip() {
    // Unit variant.
    let idle = Mover {
        motion: Motion::Idle,
    };
    assert_eq!(
        roundtrip(|r| r.register::<Mover>(), idle.clone()),
        idle,
        "unit variant round-trips as a bare token"
    );
    // Tuple variant carries its payload.
    let moving = Mover {
        motion: Motion::Moving(4.5),
    };
    assert_eq!(
        roundtrip(|r| r.register::<Mover>(), moving.clone()),
        moving,
        "tuple-variant payload round-trips"
    );
    // Struct variant carries its named fields.
    let warp = Mover {
        motion: Motion::Warp { x: 3, y: 7 },
    };
    assert_eq!(
        roundtrip(|r| r.register::<Mover>(), warp.clone()),
        warp,
        "struct-variant payload round-trips"
    );
}

/// A `Color` field round-trips as linear `color4f` (alpha preserved).
#[test]
fn color_roundtrip() {
    let value = Tint {
        color: Color::linear_rgba(0.1, 0.2, 0.3, 0.4),
    };
    let back = roundtrip(|r| r.register::<Tint>(), value.clone());
    assert_eq!(
        back, value,
        "Color survives ECS → USD → ECS in linear space"
    );
}

#[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Component, Default)]
struct Level(u32);

/// A newtype / tuple-struct component authors its field by index (`:0`).
#[test]
fn newtype_roundtrip() {
    let back = roundtrip(|r| r.register::<Level>(), Level(42));
    assert_eq!(back, Level(42), "tuple-struct field `.0` round-trips");
}

#[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Component, Default)]
struct Path {
    waypoints: Vec<Vec3>,
    weights: Vec<f32>,
    labels: Vec<String>,
}

/// Array fields round-trip through USD array values.
#[test]
fn array_fields_roundtrip() {
    let value = Path {
        waypoints: vec![Vec3::new(0.0, 1.0, 2.0), Vec3::new(3.0, 4.0, 5.0)],
        weights: vec![0.5, 1.5, 2.5],
        labels: vec!["a".into(), "b".into()],
    };
    let back = roundtrip(|r| r.register::<Path>(), value.clone());
    assert_eq!(back, value, "Vec<Vec3>/Vec<f32>/Vec<String> round-trip");
}

fn roundtrip<C: Component + PartialReflect + Clone + PartialEq + std::fmt::Debug>(
    register: impl Fn(&mut TypeRegistry) + Copy,
    value: C,
) -> C {
    let stage = Stage::builder().in_memory("rt.usda").unwrap();
    stage.define_prim("/P").unwrap();
    let short = std::any::type_name::<C>().rsplit("::").next().unwrap();

    let mut src = world_with(register);
    let e = src.spawn((crate::UsdPrimRef::new("/P"), value)).id();
    author_component_from_world(&src, &stage, e, "/P", short).unwrap();

    let live = LiveStage::new(stage);
    let mut dst = world_with(register);
    let mut map = PrimEntities::default();
    project_stage(&mut dst, &live, &mut map);
    let re = map.entity("/P").unwrap();
    dst.get::<C>(re).cloned().expect("component re-projected")
}

/// bool / int / String / unit-enum / quaternion all survive ECS → USD →
/// ECS. The quaternion case guards the gf real-first ordering bug.
#[test]
fn scalar_enum_quat_roundtrip() {
    let value = Mixed {
        flag: true,
        count: -7,
        label: "boss".into(),
        mode: Mode::Run,
        spin: Quat::from_xyzw(0.1, 0.2, 0.3, 0.7),
    };
    let back = roundtrip(
        |r| {
            r.register::<Mixed>();
            r.register::<Mode>();
        },
        value.clone(),
    );
    assert_eq!(back, value, "all field kinds round-trip, quat unscrambled");
}

/// Nested struct fields round-trip through `:`-namespaced attribute paths in
/// both directions.
#[test]
fn nested_struct_roundtrip() {
    let value = Outer {
        inner: Inner { a: 1.5, b: -2.5 },
        tag: 9,
    };
    let back = roundtrip(
        |r| {
            r.register::<Outer>();
            r.register::<Inner>();
        },
        value.clone(),
    );
    assert_eq!(back, value, "nested `inner:a`/`inner:b` round-trip");
}

#[test]
fn author_errors_are_graceful() {
    let stage = Stage::builder().in_memory("err.usda").unwrap();
    stage.define_prim("/P").unwrap();
    let mut world = world_with(|r| r.register::<Health>());
    let e = world.spawn(crate::UsdPrimRef::new("/P")).id();

    // Unregistered type.
    assert!(
        author_component_from_world(&world, &stage, e, "/P", "Nope").is_err(),
        "unregistered type errors, not panics"
    );
    // Registered but entity lacks the component.
    assert!(
        author_component_from_world(&world, &stage, e, "/P", "Health").is_err(),
        "missing component errors"
    );
}

/// The reflect route must not panic when a stage carries `bevy:` opinions
/// but the world has no `AppTypeRegistry` (a bare headless world).
#[test]
fn no_type_registry_is_graceful() {
    let stage = Stage::builder().in_memory("noreg.usda").unwrap();
    stage.define_prim("/P").unwrap();
    stage
        .create_attribute("/P.bevy:Health:max", "double")
        .unwrap()
        .set(Value::Double(1.0))
        .unwrap();
    let live = LiveStage::new(stage);
    let mut world = World::new(); // no AppTypeRegistry, no SchemaRegistry
    let mut map = PrimEntities::default();
    // Should log a warning and carry on, not panic.
    project_stage(&mut world, &live, &mut map);
    assert!(map.entity("/P").is_some(), "prim still projected");
}
