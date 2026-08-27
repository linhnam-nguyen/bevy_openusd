use super::*;
use crate::{NvidiaRevitConfig, NvidiaRevitMeasurementMapping};
use openusd::gf::{Vec3d, Vec3f};
use openusd::schemas::ui::SceneGraphPrimAPI;
use openusd::sdf::Value;

fn fixture() -> Result<Stage> {
    let stage = Stage::builder().in_memory("semantic-fixture.usda")?;
    stage.define_prim("/World")?.set_type_name("Xform")?;

    let mesh = stage
        .define_prim("/World/Triangle")?
        .set_type_name("Mesh")?;
    mesh.create_attribute("points", "point3f[]")?
        .set_custom(false)?
        .set(Value::Vec3fVec(vec![
            Vec3f::from([0.0, 0.0, 0.0]),
            Vec3f::from([1.0, 0.0, 0.0]),
            Vec3f::from([0.0, 1.0, 0.0]),
        ]))?;
    mesh.create_attribute("faceVertexCounts", "int[]")?
        .set_custom(false)?
        .set(Value::IntVec(vec![3]))?;
    mesh.create_attribute("faceVertexIndices", "int[]")?
        .set_custom(false)?
        .set(Value::IntVec(vec![0, 1, 2]))?;
    mesh.create_attribute("xformOp:translate", "double3")?
        .set_custom(false)?
        .set(Value::Vec3d(Vec3d::from([1.25, 2.0, 0.0])))?;
    mesh.create_attribute("xformOpOrder", "token[]")?
        .set_custom(false)?
        .set(Value::TokenVec(vec!["xformOp:translate".into()]))?;
    mesh.create_attribute("family", "string")?
        .set(Value::String("Furniture".to_owned()))?;
    mesh.create_attribute("height", "double")?
        .set(Value::Double(10.0))?;
    mesh.create_attribute("height_unit", "string")?
        .set(Value::String("[ft_i]".to_owned()))?;
    let ui = SceneGraphPrimAPI::apply(&stage, "/World/Triangle")?;
    ui.create_display_name_attr()?
        .set(Value::token("Triangle"))?;

    Ok(stage)
}

fn source() -> SnapshotSource {
    SnapshotSource::Working {
        session: "test".to_owned(),
        live_revision: 0,
    }
}

#[test]
fn extraction_is_deterministic_and_contains_mesh_signatures() -> Result<()> {
    let stage = fixture()?;
    let config = SemanticConfig {
        family_property: Some("family".to_owned()),
        ..Default::default()
    };
    let extractor = SemanticExtractor::new(config);

    let first = extractor.extract(&stage, source())?;
    let second = extractor.extract(&stage, source())?;

    assert_eq!(first, second);
    let entity = first.entities.get(&EntityKey::from("/World/Triangle"));
    let entity = entity.expect("triangle semantic entity");
    assert_eq!(entity.identity_source, usd_model::IdentitySource::PrimPath);
    assert_eq!(entity.semantic.type_name.as_deref(), Some("Mesh"));
    assert_eq!(entity.semantic.family.as_deref(), Some("Furniture"));
    assert_eq!(entity.semantic.display_name.as_deref(), Some("Triangle"));
    assert_eq!(entity.transform.translation_mm, [1250, 2000, 0]);

    let geometry = entity.geometry.as_ref().expect("mesh geometry");
    assert_eq!(geometry.vertex_count, 3);
    assert_eq!(geometry.index_count, 3);
    assert_eq!(geometry.local_centroid.0, [333333, 333333, 0]);
    assert_ne!(geometry.topology_hash, geometry.shape_hash);
    Ok(())
}

#[test]
fn absent_display_name_does_not_fall_back_to_prim_basename() -> Result<()> {
    let stage = Stage::builder().in_memory("display-name-authority.usda")?;
    stage.define_prim("/Architecture")?.set_type_name("Xform")?;
    stage
        .define_prim("/Architecture/Level01")?
        .set_type_name("Xform")?;
    stage
        .define_prim("/Architecture/Level01/Wall_0042")?
        .set_type_name("Xform")?;

    let snapshot = SemanticExtractor::default().extract(&stage, source())?;
    let wall = snapshot
        .entities
        .get(&EntityKey::from("/Architecture/Level01/Wall_0042"))
        .expect("wall entity");
    assert_eq!(wall.semantic.display_name, None);
    Ok(())
}

#[test]
fn configured_revit_measurement_is_normalized_during_snapshot_extraction() -> Result<()> {
    let stage = fixture()?;
    let config = SemanticConfig {
        nvidia_revit: NvidiaRevitConfig {
            measurement_mappings: vec![NvidiaRevitMeasurementMapping::new(
                "height",
                "length",
                "height_unit",
            )],
        },
        ..Default::default()
    };

    let snapshot = SemanticExtractor::new(config).extract(&stage, source())?;
    let entity = snapshot
        .entities
        .get(&EntityKey::from("/World/Triangle"))
        .expect("triangle semantic entity");
    let height = entity
        .properties
        .iter()
        .find(|property| property.name == "height")
        .expect("height property");

    assert_eq!(height.value, CanonicalValue::Real(3.048));
    let measurement = height.measurement.as_ref().expect("height measurement");
    assert_eq!(measurement.quantity.as_str(), "length");
    assert_eq!(measurement.canonical_unit.as_str(), "m");
    assert_eq!(measurement.source_unit.as_ref().unwrap().as_str(), "[ft_i]");
    Ok(())
}

#[test]
fn changing_a_custom_property_changes_entity_and_metadata_hashes() -> Result<()> {
    let stage = fixture()?;
    let config = SemanticConfig {
        family_property: Some("family".to_owned()),
        ..Default::default()
    };
    let extractor = SemanticExtractor::new(config);

    let before = extractor.extract(&stage, source())?;
    let prim = stage.prim(openusd::sdf::path("/World/Triangle")?);
    prim.attribute("family")
        .set(Value::String("Lighting".to_owned()))?;
    let after = extractor.extract(&stage, source())?;

    let before_entity = before
        .entities
        .get(&EntityKey::from("/World/Triangle"))
        .expect("before entity");
    let after_entity = after
        .entities
        .get(&EntityKey::from("/World/Triangle"))
        .expect("after entity");
    assert_ne!(before_entity.metadata_hash, after_entity.metadata_hash);
    assert_ne!(before_entity.full_hash, after_entity.full_hash);
    assert_ne!(before.snapshot_id, after.snapshot_id);
    Ok(())
}
