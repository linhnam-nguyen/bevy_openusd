//! Probe a composed skel asset: walk the prim tree, find every Skeleton
//! and SkelRoot, dump joint count + first few joint paths so we can see
//! whether our wrapper exposes the rig.

mod animation;
mod census;
mod mesh;
mod skeleton;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/skel_human.usda".to_string());
    let stage = openusd::usd::Stage::open(&path).unwrap();

    skeleton::probe_skeletons(&stage);
    mesh::probe_skinned_meshes(&stage);
    animation::probe_animations(&stage);
    mesh::probe_bindings(&stage);
    census::probe_ancestors_and_blendshapes(&stage);
}
