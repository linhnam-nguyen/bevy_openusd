#[cfg(test)]
mod tests {
    use bevy::prelude::*;
    use viewport_protocol::*;

    use crate::viewport::api::bridge::scene_query::{
        dispatch_scene_query_commands, publish_semantic_query_results,
    };
    use crate::viewport::api::bridge::state::SemanticSearchRequests;
    use crate::viewport::api::{SceneAnchorIndex, ViewportCommandInbox, ViewportEventOutbox};
    use crate::viewport::semantic::SemanticWorkingStore;

    fn semantic_search_test_app() -> App {
        let mut app = App::new();
        app.init_resource::<ViewportCommandInbox>()
            .init_resource::<ViewportEventOutbox>()
            .init_resource::<SceneAnchorIndex>()
            .init_resource::<SemanticWorkingStore>()
            .init_resource::<SemanticSearchRequests>()
            .add_systems(
                Update,
                (
                    publish_semantic_query_results,
                    dispatch_scene_query_commands,
                )
                    .chain(),
            );
        app
    }

    #[test]
    fn search_scene_routes_through_the_semantic_worker() -> anyhow::Result<()> {
        let mut app = semantic_search_test_app();
        let stage = openusd::usd::Stage::open("tests/stages/custom_attrs_extensive.usda")?;
        let ui = openusd::schemas::ui::SceneGraphPrimAPI::apply(&stage, "/World/Robot")?;
        ui.create_display_name_attr()?
            .set(openusd::sdf::Value::token("Robot"))?;
        let snapshot =
            usd_semantic::SemanticExtractor::new(usd_semantic::SemanticConfig::default()).extract(
                &stage,
                usd_model::SnapshotSource::Working {
                    session: "bridge-search-test".to_owned(),
                    live_revision: 1,
                },
            )?;
        assert!(
            app.world()
                .resource::<SemanticWorkingStore>()
                .submit_snapshot("bridge-load", snapshot)
        );

        let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::SearchScene {
                query: "Robot".to_owned(),
                offset: 0,
                limit: 10,
            },
        );

        for _ in 0..200 {
            app.update();
            if let Some(event) = app.world_mut().resource_mut::<ViewportEventOutbox>().pop() {
                assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
                let ViewportEvent::SearchResults {
                    query,
                    offset,
                    total,
                    matches,
                    has_more,
                } = event.event
                else {
                    panic!("expected semantic search results")
                };
                assert_eq!(query, "Robot");
                assert_eq!(offset, 0);
                assert_eq!(total, 1);
                assert!(matches.is_empty());
                assert!(!has_more);
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("semantic search result did not arrive")
    }

    #[test]
    fn hierarchy_search_routes_display_name_only() -> anyhow::Result<()> {
        let mut app = semantic_search_test_app();
        let stage = openusd::usd::Stage::builder().in_memory("bridge-display-name.usda")?;
        stage.define_prim("/Architecture")?.set_type_name("Xform")?;
        stage
            .define_prim("/Architecture/Level01")?
            .set_type_name("Xform")?;
        stage
            .define_prim("/Architecture/Level01/Wall_0042")?
            .set_type_name("Xform")?;
        let extractor =
            usd_semantic::SemanticExtractor::new(usd_semantic::SemanticConfig::default());
        let snapshot = extractor.extract(
            &stage,
            usd_model::SnapshotSource::Working {
                session: "bridge-display-name-search-test".to_owned(),
                live_revision: 1,
            },
        )?;
        let wall = snapshot
            .entities
            .get(&usd_model::EntityKey::from(
                "/Architecture/Level01/Wall_0042",
            ))
            .expect("wall entity exists");
        assert_eq!(wall.semantic.display_name, None);
        assert!(
            app.world()
                .resource::<SemanticWorkingStore>()
                .submit_snapshot("bridge-display-name-load", snapshot)
        );

        let run_search = |app: &mut App, query: &str| -> anyhow::Result<u32> {
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
                    let ViewportEvent::SearchResults { total, .. } = event.event else {
                        panic!("expected semantic search results")
                    };
                    return Ok(total);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            anyhow::bail!("search result did not arrive for {query}")
        };

        assert_eq!(run_search(&mut app, "Wall_0042")?, 0);

        let ui = openusd::schemas::ui::SceneGraphPrimAPI::apply(
            &stage,
            "/Architecture/Level01/Wall_0042",
        )?;
        ui.create_display_name_attr()?
            .set(openusd::sdf::Value::token("Exterior Wall"))?;
        let updated_snapshot = extractor.extract(
            &stage,
            usd_model::SnapshotSource::Working {
                session: "bridge-display-name-search-test".to_owned(),
                live_revision: 2,
            },
        )?;
        let wall = updated_snapshot
            .entities
            .get(&usd_model::EntityKey::from(
                "/Architecture/Level01/Wall_0042",
            ))
            .expect("updated wall entity exists");
        assert_eq!(wall.semantic.display_name.as_deref(), Some("Exterior Wall"));
        assert!(
            app.world()
                .resource::<SemanticWorkingStore>()
                .submit_snapshot("bridge-display-name-update", updated_snapshot)
        );

        assert_eq!(run_search(&mut app, "Exterior")?, 1);
        assert_eq!(run_search(&mut app, "Wall_0042")?, 0);
        Ok(())
    }
}
