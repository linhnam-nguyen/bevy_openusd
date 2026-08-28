use openusd::usd::Stage;
use usd_model::{CanonicalValue, SnapshotSource};
use usd_semantic::{
    NvidiaRevitConfig, NvidiaRevitIdentityConfig, SemanticConfig, SemanticExtractor,
};
use viewport_protocol::{BimFieldKey, ClassificationLevel, ClassificationRecipe};

use super::BimReadService;

#[test]
#[ignore = "requires the supplied external NVIDIA/Revit Connector export"]
fn real_nvidia_fixture_projects_element_only_leaf_when_family_is_unvalidated() -> anyhow::Result<()>
{
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../external_assets/Omniverse/V2/Projet1.usdc");
    let stage = Stage::open(fixture.to_str().expect("fixture path is valid UTF-8"))?;
    let config = SemanticConfig {
        nvidia_revit: NvidiaRevitConfig {
            identity: NvidiaRevitIdentityConfig {
                element_id_property: Some("BIM:Instance:ElementId".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let snapshot = SemanticExtractor::new(config).extract(
        &stage,
        SnapshotSource::Working {
            session: "real-nvidia-fixture".to_owned(),
            live_revision: 1,
        },
    )?;
    let wall = snapshot
        .entities
        .values()
        .find(|entity| {
            entity.properties.iter().any(|property| {
                property.name == "BIM:Instance:ElementId"
                    && property.value == CanonicalValue::Text("150663".to_owned())
            })
        })
        .expect("real Revit wall entity");
    assert_eq!(wall.semantic.bim.element_id.as_deref(), Some("150663"));
    assert_eq!(wall.semantic.bim.family_name, None);
    assert!(wall.properties.iter().any(|property| {
        property.name == "BIM:Type:Name"
            && property.value == CanonicalValue::Text("Générique - 200 mm".to_owned())
    }));

    let mut service = BimReadService::new(&snapshot);
    let recipe = ClassificationRecipe::new(vec![ClassificationLevel::new(
        "category-property",
        BimFieldKey::property("BIM:Instance:Category"),
    )]);
    let roots = service
        .classification_page(&recipe, None, 0, 20)
        .expect("real category roots");
    let murs = roots
        .nodes
        .iter()
        .find(|node| node.name == "Murs")
        .map(|node| node.id.clone())
        .expect("real wall category group");
    let leaves = service
        .classification_page(&recipe, Some(&murs), 0, 20)
        .expect("real wall leaves");
    let wall_leaf = leaves
        .nodes
        .iter()
        .find(|node| {
            node.anchor
                .as_ref()
                .is_some_and(|anchor| anchor.prim_path == wall.prim_path)
        })
        .expect("real wall classification leaf");
    assert_eq!(wall_leaf.name, "150663");
    assert!(!wall_leaf.name.contains("Murs"));
    Ok(())
}
