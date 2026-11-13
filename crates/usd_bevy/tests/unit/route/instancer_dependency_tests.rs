use super::*;

#[test]
fn dependencies_resolve_through_compact_ids() {
    let mut paths = PathStore::default();
    let mut index = PointInstancerDependencyIndex::default();
    index.replace_instancer(
        &mut paths,
        "/World/Instancer",
        vec!["/World/Prototypes/Tree".to_owned()],
    );

    let dependents = index.dependents_for_path(&paths, "/World/Prototypes/Tree/Trunk");
    assert_eq!(
        dependents
            .iter()
            .filter_map(|id| paths.path(*id))
            .collect::<Vec<_>>(),
        vec!["/World/Instancer"]
    );
}
