#[cfg(test)]
mod tests {
    use bevy::prelude::*;
    use viewport_protocol::*;

    use crate::viewport::api::bridge::scene_query::{
        dispatch_scene_query_commands, publish_scene_query_results,
    };
    use crate::viewport::api::bridge::state::SceneSearchRequests;
    use crate::viewport::api::scene_query::SceneQueryService;
    use crate::viewport::api::{SceneAnchorIndex, ViewportCommandInbox, ViewportEventOutbox};

    fn hierarchy_search_test_app(nodes: Vec<PrimNodeReadModel>) -> App {
        let mut app = App::new();
        app.init_resource::<ViewportCommandInbox>()
            .init_resource::<ViewportEventOutbox>()
            .insert_resource(SceneAnchorIndex::from_test_nodes(nodes))
            .init_resource::<SceneQueryService>()
            .init_resource::<SceneSearchRequests>()
            .add_systems(
                Update,
                (publish_scene_query_results, dispatch_scene_query_commands).chain(),
            );
        app
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
        Ok(())
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
