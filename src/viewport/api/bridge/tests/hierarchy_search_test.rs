#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bevy::prelude::*;
    use usd_model::{HashDigest, SemanticSnapshot, SnapshotId, SnapshotSource};
    use viewport_protocol::*;

    use crate::viewport::api::bridge::scene_query::{
        dispatch_scene_query_commands, publish_scene_query_results,
    };
    use crate::viewport::api::bridge::state::SceneSearchRequests;
    use crate::viewport::api::scene_query::SceneQueryService;
    use crate::viewport::api::{
        ActiveHierarchyProvider, CurrentHierarchyProjection, SceneAnchorIndex,
        ViewportCommandInbox, ViewportEventOutbox,
    };
    use crate::viewport::semantic::SemanticSyncState;

    fn hierarchy_search_test_app(nodes: Vec<PrimNodeReadModel>) -> App {
        let projection = CurrentHierarchyProjection::from_prim_nodes(&nodes, 1);
        let mut app = App::new();
        app.init_resource::<ViewportCommandInbox>()
            .init_resource::<ViewportEventOutbox>()
            .insert_resource(SceneAnchorIndex::from_test_nodes(nodes))
            .insert_resource(projection)
            .init_resource::<SceneQueryService>()
            .init_resource::<SceneSearchRequests>()
            .add_systems(
                Update,
                (publish_scene_query_results, dispatch_scene_query_commands).chain(),
            );
        app
    }

    #[test]
    fn classification_provider_switch_builds_a_virtual_projection() {
        let mut app = App::new();
        app.init_resource::<ViewportCommandInbox>()
            .init_resource::<ViewportEventOutbox>()
            .init_resource::<SceneAnchorIndex>()
            .init_resource::<CurrentHierarchyProjection>()
            .init_resource::<ActiveHierarchyProvider>()
            .init_resource::<SceneQueryService>()
            .init_resource::<SceneSearchRequests>()
            .insert_resource(SemanticSyncState::from_test_snapshot(empty_snapshot()))
            .add_systems(Update, dispatch_scene_query_commands);

        let recipe = ClassificationRecipe::new(vec![ClassificationLevel::new(
            "category",
            BimFieldKey::Category,
        )]);
        let _request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::SetHierarchySource {
                source: HierarchySource::BimClassification,
                classification_recipe: Some(recipe),
            },
        );

        app.update();

        assert_eq!(
            app.world().resource::<ActiveHierarchyProvider>().source(),
            HierarchySource::BimClassification
        );
        let projection = app.world().resource::<CurrentHierarchyProjection>();
        assert_eq!(projection.source(), HierarchySource::BimClassification);
        assert!(projection.snapshot().nodes.is_empty());
        assert!(
            app.world_mut()
                .resource_mut::<ViewportEventOutbox>()
                .pop()
                .is_none()
        );
    }

    fn empty_snapshot() -> SemanticSnapshot {
        SemanticSnapshot {
            snapshot_id: SnapshotId("hierarchy-test".to_owned()),
            source: SnapshotSource::Working {
                session: "hierarchy-test".to_owned(),
                live_revision: 1,
            },
            config_hash: HashDigest::new([0; HashDigest::BYTE_LEN]),
            entities: HashMap::new(),
        }
    }

    #[test]
    fn hierarchy_search_routes_through_the_projected_snapshot() -> anyhow::Result<()> {
        let nodes = vec![
            node("/root", None, "root", None),
            node("/root/name1", Some("/root"), "name1", None),
            node("/root/name1/name2", Some("/root/name1"), "name2", None),
            node(
                "/root/name1/name2/name3",
                Some("/root/name1/name2"),
                "name3",
                None,
            ),
            node("/root/name10", Some("/root"), "name10", None),
            node("/root/name10/name20", Some("/root/name10"), "name20", None),
            node(
                "/Building/Level01/Wall_0042",
                None,
                "Wall_0042",
                Some("Exterior Wall"),
            ),
            node(
                "/Kitchen_set/Props_grp/DiningTable_grp/ChairB_1",
                Some("/Kitchen_set/Props_grp/DiningTable_grp"),
                "ChairB_1",
                None,
            ),
            node(
                "/Kitchen_set/Props_grp/DiningTable_grp/ChairB_2",
                Some("/Kitchen_set/Props_grp/DiningTable_grp"),
                "ChairB_2",
                None,
            ),
        ];
        let mut app = hierarchy_search_test_app(nodes);

        let run_search = |app: &mut App, query: &str| -> anyhow::Result<Vec<SceneSearchMatch>> {
            let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
                ViewportCommand::SearchScene {
                    query: query.to_owned(),
                    offset: 0,
                    limit: 10,
                },
            );
            for _ in 0..200 {
                app.update();
                if let Some(event) = app.world_mut().resource_mut::<ViewportEventOutbox>().pop() {
                    assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
                    let ViewportEvent::SearchResults { total, matches, .. } = event.event else {
                        panic!("expected hierarchy search results")
                    };
                    assert_eq!(total as usize, matches.len());
                    return Ok(matches);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            anyhow::bail!("hierarchy search result did not arrive for {query}")
        };

        let name2 = run_search(&mut app, "name2")?;
        assert_eq!(name2.len(), 1);
        assert_eq!(name2[0].label, "name2");
        assert_eq!(name2[0].breadcrumb, "/root/name1/name2");
        assert_eq!(name2[0].anchor.prim_path, "/root/name1/name2");

        let name3 = run_search(&mut app, "name3")?;
        assert_eq!(
            name3
                .iter()
                .map(|result| result.anchor.prim_path.as_str())
                .collect::<Vec<_>>(),
            vec!["/root/name1/name2/name3"]
        );
        let name1 = run_search(&mut app, "name1")?;
        assert_eq!(
            name1
                .iter()
                .map(|result| result.anchor.prim_path.as_str())
                .collect::<Vec<_>>(),
            vec!["/root/name1"]
        );
        assert!(run_search(&mut app, "missing")?.is_empty());
        assert!(run_search(&mut app, "Exterior Wall")?.is_empty());
        assert_eq!(
            run_search(&mut app, "Wall_0042")?[0].breadcrumb,
            "/Building/Level01/Wall_0042"
        );
        for query in ["chair", "cha", "hair"] {
            assert_eq!(
                run_search(&mut app, query)?
                    .iter()
                    .map(|result| result.anchor.prim_path.as_str())
                    .collect::<Vec<_>>(),
                vec![
                    "/Kitchen_set/Props_grp/DiningTable_grp/ChairB_1",
                    "/Kitchen_set/Props_grp/DiningTable_grp/ChairB_2",
                ]
            );
        }
        Ok(())
    }

    #[test]
    fn bim_search_routes_through_the_semantic_worker() -> anyhow::Result<()> {
        let mut app = App::new();
        app.init_resource::<ViewportCommandInbox>()
            .init_resource::<ViewportEventOutbox>()
            .init_resource::<SceneAnchorIndex>()
            .init_resource::<CurrentHierarchyProjection>()
            .init_resource::<SceneQueryService>()
            .init_resource::<SceneSearchRequests>()
            .insert_resource(SemanticSyncState::from_test_snapshot(empty_snapshot()))
            .add_systems(
                Update,
                (publish_scene_query_results, dispatch_scene_query_commands).chain(),
            );

        let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::SearchBim {
                query: BimSearchQuery::PropertyNameRegex {
                    pattern: "Fire.*".to_owned(),
                    page: BimPageRequest::new(0, 20),
                },
            },
        );

        for _ in 0..200 {
            app.update();
            if let Some(event) = app.world_mut().resource_mut::<ViewportEventOutbox>().pop() {
                assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
                let ViewportEvent::BimSearchResults { result } = event.event else {
                    panic!("expected BIM search results")
                };
                assert!(matches!(
                    result,
                    BimSearchResult::PropertyNames {
                        total: 0,
                        matches,
                        has_more: false,
                        ..
                    } if matches.is_empty()
                ));
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        anyhow::bail!("BIM search result did not arrive")
    }

    fn node(
        path: &str,
        parent: Option<&str>,
        name: &str,
        usd_display_name: Option<&str>,
    ) -> PrimNodeReadModel {
        PrimNodeReadModel {
            anchor: SceneAnchor::active_session(path),
            parent: parent.map(SceneAnchor::active_session),
            label: name.to_owned(),
            display_name: usd_display_name.map(str::to_owned),
            visible: true,
            has_children: false,
        }
    }
}
