use super::*;

#[test]
fn ordinary_lookup_preserves_descendant_prototype_dependencies() {
    let mut paths = PathStore::default();
    let prototype = paths.intern("/World/Prototype/Nested");
    let proxy = paths.intern("/World/Instance/Nested");
    let unrelated_prototype = paths.intern("/World/OtherPrototype/Nested");
    let unrelated_proxy = paths.intern("/World/OtherInstance/Nested");
    let mut index = NativeInstanceDependencyIndex::default();
    index.by_prototype.insert(prototype, HashSet::from([proxy]));
    index.by_proxy.insert(proxy, HashSet::from([prototype]));
    index
        .by_prototype
        .insert(unrelated_prototype, HashSet::from([unrelated_proxy]));
    index
        .by_proxy
        .insert(unrelated_proxy, HashSet::from([unrelated_prototype]));

    assert_eq!(
        index.dependents_for_path(&paths, "/World/Prototype"),
        HashSet::from([proxy]),
        "an inheritable ancestor change must reach registered descendant prototypes"
    );
    assert_eq!(
        index.dependents_for_path(&paths, "/World/Prototype/Nested/Geometry"),
        HashSet::from([proxy])
    );
    assert_eq!(
        index.dependents_for_resync_root(&paths, "/World/Prototype"),
        HashSet::from([proxy])
    );
}
