use crate::live::{ProjectionPlan, ProjectionPlanBuilder, collect_stage_subtree_paths};
use crate::snippet::UsdSnippet;

fn hierarchy_stage() -> openusd::usd::Stage {
    UsdSnippet::new(
        r#"#usda 1.0

def Xform "Z"
{
    def Xform "Child"
    {
    }
}
def Xform "A"
{
    def Xform "Leaf"
    {
    }
}
def Xform "B"
{
}
"#,
    )
    .open_stage()
    .expect("hierarchy stage opens")
}

#[test]
fn projection_plan_is_deterministic_and_parent_before_child() {
    let stage = hierarchy_stage();
    let first = ProjectionPlan::from_stage(&stage).expect("first plan builds");
    let second = ProjectionPlan::from_stage(&stage).expect("second plan builds");
    assert_eq!(first, second);
    assert_eq!(
        first.paths().collect::<Vec<_>>(),
        vec!["/", "/A", "/B", "/Z", "/A/Leaf", "/Z/Child"]
    );
    for (index, entry) in first.entries().enumerate() {
        if let Some(parent) = entry.parent_index() {
            assert!(parent < index, "parent must precede {}", entry.path());
            assert_eq!(
                first.entry(parent).expect("parent entry").path(),
                match entry.path() {
                    "/A" | "/B" | "/Z" => "/",
                    "/A/Leaf" => "/A",
                    "/Z/Child" => "/Z",
                    path => panic!("unexpected path {path}"),
                }
            );
        }
    }
}

#[test]
fn projection_plan_matches_the_canonical_traversal_predicate() {
    let stage = hierarchy_stage();
    let plan = ProjectionPlan::from_stage(&stage).expect("plan builds");
    let mut traversed = collect_stage_subtree_paths(&stage, "/").expect("paths collect");
    traversed.sort();
    let mut planned = plan.paths().skip(1).map(str::to_owned).collect::<Vec<_>>();
    planned.sort();
    assert_eq!(planned, traversed);
}

#[test]
fn projection_plan_keeps_unloaded_payload_prim_as_placeholder_work() {
    let stage = UsdSnippet::new(
        r#"#usda 1.0
def Xform "World"
{
    def "PayloadPrim" (
        payload = @./sub.usda@</Sub>
    )
    {
    }
}
"#,
    )
    .open_stage()
    .expect("payload stage opens");
    stage.unload(openusd::sdf::path("/World/PayloadPrim").expect("payload prim path"));
    let plan = ProjectionPlan::from_stage(&stage).expect("payload plan builds");
    assert!(plan.paths().any(|path| path == "/World/PayloadPrim"));
    assert!(
        !stage
            .prim(openusd::sdf::path("/World/PayloadPrim").unwrap())
            .is_loaded()
            .expect("payload load state is readable")
    );
}

#[test]
fn subtree_plan_preserves_root_and_parent_relation() {
    let stage = hierarchy_stage();
    let plan = ProjectionPlan::from_subtree(&stage, "/A").expect("subtree plan builds");
    assert_eq!(plan.paths().collect::<Vec<_>>(), vec!["/", "/A", "/A/Leaf"]);
    assert_eq!(plan.entry(1).unwrap().parent_index(), Some(0));
    assert_eq!(plan.entry(2).unwrap().parent_index(), Some(1));
}

#[test]
fn incremental_builder_yields_parent_before_child_work() {
    let stage = hierarchy_stage();
    let mut builder = ProjectionPlanBuilder::new(&stage);
    assert_eq!(builder.len(), 1);
    assert!(!builder.is_finished());

    assert!(!builder.advance_one().expect("root expansion succeeds"));
    assert_eq!(builder.len(), 4);
    assert_eq!(builder.entry(1).unwrap().path(), "/A");
    assert_eq!(builder.entry(3).unwrap().path(), "/Z");

    while !builder.is_finished() {
        builder.advance_one().expect("parent expansion succeeds");
    }
    let plan = builder.finish().expect("incremental plan finishes");
    assert_eq!(
        plan.paths().collect::<Vec<_>>(),
        vec!["/", "/A", "/B", "/Z", "/A/Leaf", "/Z/Child"]
    );
}
