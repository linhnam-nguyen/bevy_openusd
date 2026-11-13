use super::*;

#[test]
fn coarse_proxy_has_one_wire_box() {
    let edges = proxy_edges(ProjectedWorldBounds {
        min: Vec3::ZERO,
        max: Vec3::ONE,
    });
    assert_eq!(edges.len(), 12);
}
