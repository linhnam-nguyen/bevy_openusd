use crate::live::{
    LiveRevision, StageChange, StageChangeBatch, collect_stage_subtree_paths,
    is_descendant_or_self, minimize_resync_roots, normalize_prim_path,
};
use crate::snippet::UsdSnippet;

#[test]
fn test_normalize_prim_path() {
    assert_eq!(normalize_prim_path(""), "/");
    assert_eq!(normalize_prim_path("   "), "/");
    assert_eq!(normalize_prim_path("/"), "/");
    assert_eq!(normalize_prim_path("/World"), "/World");
    assert_eq!(normalize_prim_path("/World/"), "/World");
    assert_eq!(normalize_prim_path("World/A/B"), "/World/A/B");
    assert_eq!(normalize_prim_path("/World/A.property"), "/World/A");
    assert_eq!(
        normalize_prim_path("/World/Robot.userProperties:name"),
        "/World/Robot"
    );
    assert_eq!(
        normalize_prim_path("/World/A/B.xformOp:transform"),
        "/World/A/B"
    );
}

#[test]
fn test_is_descendant_or_self() {
    // Root / covers all paths
    assert!(is_descendant_or_self("/", "/"));
    assert!(is_descendant_or_self("/", "/World"));
    assert!(is_descendant_or_self("/", "/World/A/B"));

    // Exact match
    assert!(is_descendant_or_self("/World/A", "/World/A"));

    // True descendants
    assert!(is_descendant_or_self("/World/A", "/World/A/B"));
    assert!(is_descendant_or_self("/World/A", "/World/A/B/Leaf"));
    assert!(is_descendant_or_self("/World/A", "/World/A.property"));

    // Boundary awareness (avoiding prefix collisions)
    assert!(!is_descendant_or_self("/World/A", "/World/AB"));
    assert!(!is_descendant_or_self("/World/A", "/World/A_Other"));
    assert!(!is_descendant_or_self("/World/A", "/World/B"));
    assert!(!is_descendant_or_self("/World/A", "/World"));
}

#[test]
fn test_minimize_resync_roots() {
    // Empty
    assert_eq!(
        minimize_resync_roots(Vec::<&str>::new()),
        Vec::<String>::new()
    );

    // Deduplication
    assert_eq!(
        minimize_resync_roots(["/World/A", "/World/A"]),
        vec!["/World/A".to_string()]
    );

    // Deep overlap minimization
    let input = [
        "/World/A/B",
        "/World/C",
        "/World/A",
        "/World/A/B/Leaf",
        "/World/C/Sub",
    ];
    let result = minimize_resync_roots(input);
    assert_eq!(result, vec!["/World/A".to_string(), "/World/C".to_string()]);

    // Prefix boundary respected
    let input = ["/World/A", "/World/AB", "/World/A/Child"];
    let result = minimize_resync_roots(input);
    assert_eq!(
        result,
        vec!["/World/A".to_string(), "/World/AB".to_string()]
    );

    // Full stage root covers all
    let input = ["/World/A", "/World/B", "/", "/World/C/D"];
    let result = minimize_resync_roots(input);
    assert_eq!(result, vec!["/".to_string()]);

    // Property paths stripped to owning prims
    let input = ["/World/A.xformOp:transform", "/World/A/Child.property"];
    let result = minimize_resync_roots(input);
    assert_eq!(result, vec!["/World/A".to_string()]);
}

#[test]
fn test_stage_change_batch_resync_roots_and_unshaded_changed_info() {
    let batch = StageChangeBatch {
        revision: LiveRevision(1),
        changes: vec![
            StageChange {
                resynced: vec![
                    "/World/A/Child".to_string(),
                    "/World/A".to_string(),
                    "/World/C".to_string(),
                ],
                changed_info: vec![
                    "/World/A/Child.userProperties:speed".to_string(),
                    "/World/B.userProperties:name".to_string(),
                    "/World/C/Leaf.xformOp:transform".to_string(),
                    "/World/D.visibility".to_string(),
                ],
            },
            StageChange {
                resynced: vec!["/World/C/Sub".to_string()],
                changed_info: vec!["/World/D.visibility".to_string()], // duplicate
            },
        ],
    };

    assert!(batch.has_resync());
    assert_eq!(
        batch.resync_roots(),
        vec!["/World/A".to_string(), "/World/C".to_string()]
    );
    assert!(batch.is_path_under_resync("/World/A"));
    assert!(batch.is_path_under_resync("/World/A/Child/Leaf"));
    assert!(batch.is_path_under_resync("/World/C"));
    assert!(!batch.is_path_under_resync("/World/B"));
    assert!(!batch.is_path_under_resync("/World/D"));

    // /World/A/... and /World/C/... are shaded by resync roots /World/A and /World/C
    let unshaded = batch.unshaded_changed_info();
    assert_eq!(
        unshaded,
        vec![
            "/World/B.userProperties:name".to_string(),
            "/World/D.visibility".to_string(),
        ]
    );
}

#[test]
fn test_collect_stage_subtree_paths_synthetic_wide() {
    let mut usda = String::from("#usda 1.0\n\ndef Xform \"World\"\n{\n");
    for group in ["A", "B", "C"] {
        usda.push_str(&format!("    def Xform \"{group}\"\n    {{\n"));
        for i in 0..10 {
            usda.push_str(&format!(
                "        def Xform \"{group}{i}\"\n        {{\n        }}\n"
            ));
        }
        usda.push_str("    }\n");
    }
    usda.push_str("}\n");

    let stage = UsdSnippet::new(&usda)
        .open_stage()
        .expect("synthetic wide stage opens");

    // Subtree /World/B has 1 root + 10 children = 11 prims
    let b_paths = collect_stage_subtree_paths(&stage, "/World/B").expect("collect /World/B");
    assert_eq!(b_paths.len(), 11);
    assert_eq!(b_paths[0], "/World/B");
    for i in 0..10 {
        assert!(b_paths.contains(&format!("/World/B/B{i}")));
    }

    // Leaf prim /World/A/A0 has 1 prim
    let leaf_paths =
        collect_stage_subtree_paths(&stage, "/World/A/A0").expect("collect /World/A/A0");
    assert_eq!(leaf_paths, vec!["/World/A/A0".to_string()]);

    // Full stage root "/" collects all 34 prims
    let all_paths = collect_stage_subtree_paths(&stage, "/").expect("collect /");
    assert_eq!(all_paths.len(), 34);

    // Non-existent subtree returns empty
    let missing =
        collect_stage_subtree_paths(&stage, "/World/NonExistent").expect("collect missing");
    assert!(missing.is_empty());
}

#[test]
fn test_collect_stage_subtree_paths_deep_overlap() {
    let stage = UsdSnippet::new(
        r#"#usda 1.0

def Xform "World"
{
    def Xform "A"
    {
        def Xform "Child"
        {
            def Xform "Leaf"
            {
            }
        }
    }
    def Xform "B"
    {
    }
}
"#,
    )
    .open_stage()
    .expect("deep overlap stage opens");

    let a_paths = collect_stage_subtree_paths(&stage, "/World/A").expect("collect /World/A");
    assert_eq!(
        a_paths,
        vec![
            "/World/A".to_string(),
            "/World/A/Child".to_string(),
            "/World/A/Child/Leaf".to_string(),
        ]
    );

    let child_paths =
        collect_stage_subtree_paths(&stage, "/World/A/Child").expect("collect /World/A/Child");
    assert_eq!(
        child_paths,
        vec![
            "/World/A/Child".to_string(),
            "/World/A/Child/Leaf".to_string(),
        ]
    );
}

#[test]
fn test_collect_stage_subtree_paths_respects_projection_predicate() {
    let stage = UsdSnippet::new(
        r#"#usda 1.0

def Xform "World"
{
    def Xform "Visible"
    {
    }
    class "_AbstractBase"
    {
        def Xform "UnderAbstract"
        {
        }
    }
}
"#,
    )
    .open_stage()
    .expect("stage opens");

    let paths = collect_stage_subtree_paths(&stage, "/").expect("collect /");
    assert!(paths.contains(&"/World".to_string()));
    assert!(paths.contains(&"/World/Visible".to_string()));
    // Abstract classes should be excluded by the projection predicate
    assert!(!paths.iter().any(|p| p.contains("_AbstractBase")));
}
