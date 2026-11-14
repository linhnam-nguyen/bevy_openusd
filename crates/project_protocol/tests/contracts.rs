mod activation {
    mod tests {
        use project_protocol::*;
        use usd_project::SceneId;
        use usd_project::{ModelId, ProjectId, ProjectRoot};

        #[test]
        fn activation_command_and_reply_round_trip_without_a_path() {
            let command = ProjectActivationCommand::new(
                "activation-1",
                4,
                ProjectId::new_v4(),
                ProjectStageTarget::ProjectRoot(ProjectRoot::Scene(SceneId::new_v4())),
            );
            command.validate().unwrap();

            let encoded =
                serde_json::to_string(&ProjectActivationReply::activated(&command)).unwrap();
            let decoded: ProjectActivationReply = serde_json::from_str(&encoded).unwrap();

            assert_eq!(decoded, ProjectActivationReply::activated(&command));
            assert!(!encoded.contains("/Users/"));
            assert!(!encoded.contains("PathBuf"));
        }

        #[test]
        fn activation_reply_requires_request_project_root_and_generation_match() {
            let command = ProjectActivationCommand::new(
                "activation-1",
                4,
                ProjectId::new_v4(),
                ProjectStageTarget::ProjectRoot(ProjectRoot::Scene(SceneId::new_v4())),
            );
            let mut reply = ProjectActivationReply::activated(&command);
            assert!(reply.matches_command(&command));

            if let ProjectActivationResult::Activated { generation, .. } = &mut reply.result {
                *generation += 1;
            }
            assert!(!reply.matches_command(&command));
        }

        #[test]
        fn activation_reply_rejects_a_different_stage_target() {
            let command = ProjectActivationCommand::new(
                "activation-1",
                4,
                ProjectId::new_v4(),
                ProjectStageTarget::Scene(SceneId::new_v4()),
            );
            let mut reply = ProjectActivationReply::activated(&command);
            reply.result = ProjectActivationResult::Activated {
                generation: command.generation,
                project_id: command.project_id,
                target: ProjectStageTarget::Scene(SceneId::new_v4()),
            };

            assert!(!reply.matches_command(&command));
        }

        #[test]
        fn activation_command_decodes_old_v2_payload_then_rejects_it() {
            let command = ProjectActivationCommand::new(
                "activation-v2",
                7,
                ProjectId::new_v4(),
                ProjectStageTarget::Model(ModelId::new_v4()),
            );
            let mut encoded = serde_json::to_value(&command).unwrap();
            encoded["protocol_version"] = serde_json::json!(2);

            let decoded: ProjectActivationCommand = serde_json::from_value(encoded).unwrap();

            assert_eq!(decoded.protocol_version, 2);
            assert_eq!(
                decoded.validate(),
                Err(ProjectActivationError::UnsupportedProtocolVersion {
                    expected: PROJECT_ACTIVATION_PROTOCOL_VERSION,
                    actual: 2,
                })
            );
        }
    }
}

mod placement {
    mod tests {
        use project_protocol::*;
        use usd_project::ScenePlacementTransform;

        #[test]
        fn default_placement_is_identity() {
            assert_eq!(
                PlacementSpec::Default.resolve().unwrap(),
                ScenePlacementTransform::IDENTITY
            );
        }

        #[test]
        fn matrix_accepts_whitespace_and_commas() {
            let matrix = PlacementSpec::Matrix("1, 0 0 0\n0 2 0 0\n0 0 -1 0\n3 4 5 1".to_owned())
                .resolve()
                .unwrap();
            assert_eq!(matrix.0[5], 2.0);
            assert_eq!(matrix.0[10], -1.0);
            assert_eq!(matrix.0[12..15], [3.0, 4.0, 5.0]);
        }

        #[test]
        fn matrix_rejects_non_affine_and_singular_values() {
            let non_affine = PlacementSpec::Matrix("1 0 0 0\n0 1 0 0\n0 0 1 0\n0 0 0 2".to_owned());
            assert_eq!(
                non_affine.resolve(),
                Err(PlacementValidationError::NonAffine)
            );

            let singular = PlacementSpec::Matrix("1 0 0 0\n0 0 0 0\n0 0 1 0\n0 0 0 1".to_owned());
            assert_eq!(singular.resolve(), Err(PlacementValidationError::Singular));
        }
    }
}

mod tests {
    use usd_project::{ProjectId, ProjectSummary};

    use project_protocol::*;

    #[test]
    fn command_and_reply_round_trip_as_shared_json() {
        let project_id = ProjectId::new_v4();
        let command = ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(project_id));
        let encoded = serde_json::to_string(&command).unwrap();
        let decoded: ProjectReadCommand = serde_json::from_str(&encoded).unwrap();
        assert_eq!(command, decoded);

        let reply = ProjectReadReply::success(ProjectReadResponse::Projects(vec![
            ProjectListItem::Available(Box::new(ProjectSummary {
                id: project_id,
                name: "Project".to_owned(),
                root: usd_project::ProjectRoot::Empty,
                repository: usd_project::RepositorySummary {
                    active_branch: None,
                    branches: Vec::new(),
                    dirty: false,
                    head: None,
                    latest_commit: None,
                },
                counts: usd_project::ProjectContentCounts::default(),
                issues: usd_project::ProjectIssueSummary::default(),
                people: usd_project::ProjectPeopleSummary::default(),
                capabilities: usd_project::ProjectCapabilities::default(),
            })),
        ]));
        let encoded = serde_json::to_string(&reply).unwrap();
        let decoded: ProjectReadReply = serde_json::from_str(&encoded).unwrap();
        assert_eq!(reply, decoded);
    }

    #[test]
    fn invalid_protocol_version_is_typed_and_does_not_need_a_path() {
        let command = ProjectReadCommand {
            protocol_version: PROJECT_READ_PROTOCOL_VERSION + 1,
            request: ProjectReadRequest::ListProjects,
        };

        assert_eq!(
            command.validate().unwrap_err(),
            ProjectReadError::UnsupportedProtocolVersion {
                expected: PROJECT_READ_PROTOCOL_VERSION,
                actual: PROJECT_READ_PROTOCOL_VERSION + 1,
            }
        );
        let encoded = serde_json::to_string(&command.validate().unwrap_err()).unwrap();
        assert!(!encoded.contains("/"));
    }

    #[test]
    fn local_selection_wire_contract_does_not_contain_a_filesystem_path() {
        let result = ProjectLocationResult::Selected(LocalSelectionView {
            token: LocalSelectionToken::new("session-token"),
            display_name: "Selected Project Folder".to_owned(),
        });
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(encoded.contains("session-token"));
        assert!(!encoded.contains("PathBuf"));
        assert!(!encoded.contains("/Users/"));
    }

    #[test]
    fn write_command_round_trip_keeps_selection_and_fingerprint_opaque() {
        let command = ProjectWriteCommand::new(ProjectWriteRequest::Import(ProjectImportRequest {
            selection: LocalSelectionToken::new("selection-token"),
            inspection: ProjectInspection {
                classification: ProjectInspectionClassification::AdoptableGit,
                display_name: "Project".to_owned(),
                warnings: vec![ProjectInspectionWarning::MissingLocalCacheRoots],
                fingerprint: "opaque-fingerprint".to_owned(),
            },
        }));
        let encoded = serde_json::to_string(&command).unwrap();
        let decoded: ProjectWriteCommand = serde_json::from_str(&encoded).unwrap();

        assert_eq!(command, decoded);
        assert!(!encoded.contains("PathBuf"));
        assert!(!encoded.contains("/Users/"));
    }

    #[test]
    fn scene_inspection_command_round_trips_operation_and_generation() {
        let command = ProjectSceneInspectionCommand::new(ProjectSceneInspectionRequest {
            source: LocalSelectionToken::new("scene-source"),
            operation_id: "inspection-1".to_owned(),
            generation: 9,
        });

        let encoded = serde_json::to_string(&command).unwrap();
        let decoded: ProjectSceneInspectionCommand = serde_json::from_str(&encoded).unwrap();

        assert_eq!(command, decoded);
        assert!(!encoded.contains("/Users/"));
    }

    #[test]
    fn previous_write_protocol_version_is_rejected_after_json_deserialization() {
        let mut encoded = serde_json::to_value(ProjectWriteCommand::new(
            ProjectWriteRequest::DeleteProject(ProjectLifecycleRequest {
                project_id: ProjectId::new_v4(),
            }),
        ))
        .unwrap();
        encoded["protocol_version"] = serde_json::json!(1);
        let command: ProjectWriteCommand = serde_json::from_value(encoded).unwrap();

        assert_eq!(
            command.validate().unwrap_err(),
            ProjectWriteError::UnsupportedProtocolVersion {
                expected: PROJECT_WRITE_PROTOCOL_VERSION,
                actual: 1,
            }
        );
    }

    #[test]
    fn previous_scene_inspection_protocol_version_is_rejected_after_json_deserialization() {
        let mut encoded = serde_json::to_value(ProjectSceneInspectionCommand::new(
            ProjectSceneInspectionRequest {
                source: LocalSelectionToken::new("scene-source"),
                operation_id: "inspection-legacy".to_owned(),
                generation: 1,
            },
        ))
        .unwrap();
        encoded["protocol_version"] = serde_json::json!(1);
        let command: ProjectSceneInspectionCommand = serde_json::from_value(encoded).unwrap();

        assert_eq!(
            command.validate().unwrap_err(),
            ProjectWriteError::UnsupportedProtocolVersion {
                expected: PROJECT_SCENE_INSPECTION_PROTOCOL_VERSION,
                actual: 1,
            }
        );
    }

    #[test]
    fn previous_model_preparation_protocol_version_is_rejected_after_json_deserialization() {
        let mut encoded = serde_json::to_value(ProjectModelPreparationCommand::new(
            ProjectModelPreparationRequest {
                source: LocalSelectionToken::new("model-source"),
                operation_id: "preparation-legacy".to_owned(),
                generation: 1,
            },
        ))
        .unwrap();
        encoded["protocol_version"] = serde_json::json!(1);
        let command: ProjectModelPreparationCommand = serde_json::from_value(encoded).unwrap();

        assert_eq!(
            command.validate().unwrap_err(),
            ProjectWriteError::UnsupportedProtocolVersion {
                expected: PROJECT_MODEL_PREPARATION_PROTOCOL_VERSION,
                actual: 1,
            }
        );
    }

    #[test]
    fn scene_adoption_command_round_trips_typed_target_and_preview_identity() {
        let project_id = ProjectId::new_v4();
        let command =
            ProjectWriteCommand::new(ProjectWriteRequest::AdoptScene(ProjectAdoptSceneRequest {
                project_id,
                target: ProjectWriteTarget::Project(project_id),
                source: LocalSelectionToken::new("scene-source"),
                inspection: usd_project::CompositionInspection {
                    classification: usd_project::CompositionClassification::SceneLike,
                    root_prims: vec!["/Assembly".to_owned()],
                    dependencies: Vec::new(),
                    diagnostics: Vec::new(),
                    has_variants: false,
                    has_payloads: false,
                    has_references: true,
                    has_sublayers: false,
                    spatial: Default::default(),
                },
                name: "Assembly".to_owned(),
                operation_id: "adoption-1".to_owned(),
                generation: 11,
                placement: PlacementSpec::Default,
            }));

        let encoded = serde_json::to_string(&command).unwrap();
        let decoded: ProjectWriteCommand = serde_json::from_str(&encoded).unwrap();

        assert_eq!(command, decoded);
        assert!(!encoded.contains("/Users/"));
    }

    #[test]
    fn model_preparation_command_round_trips_operation_and_generation() {
        let command = ProjectModelPreparationCommand::new(ProjectModelPreparationRequest {
            source: LocalSelectionToken::new("model-source"),
            operation_id: "preparation-1".to_owned(),
            generation: 12,
        });

        let encoded = serde_json::to_string(&command).unwrap();
        let decoded: ProjectModelPreparationCommand = serde_json::from_str(&encoded).unwrap();

        assert_eq!(command, decoded);
        assert!(!encoded.contains("/Users/"));
    }

    #[test]
    fn import_progress_round_trips_the_latest_operation_phase() {
        let progress = ProjectImportProgress {
            operation_id: "import-1".to_owned(),
            generation: 3,
            phase: ProjectImportPhase::Publishing,
        };
        let encoded = serde_json::to_string(&progress).unwrap();
        let decoded: ProjectImportProgress = serde_json::from_str(&encoded).unwrap();

        assert_eq!(progress, decoded);
        assert_eq!(decoded.phase, ProjectImportPhase::Publishing);
    }

    #[test]
    fn import_progress_query_round_trips_opaque_operation_identity() {
        let command = ProjectImportProgressCommand::new(ProjectImportProgressRequest {
            operation_id: "import-1".to_owned(),
            generation: 3,
        });
        let encoded = serde_json::to_string(&command).unwrap();
        let decoded: ProjectImportProgressCommand = serde_json::from_str(&encoded).unwrap();

        assert_eq!(command, decoded);
        assert!(!encoded.contains("/Users/"));
    }

    #[test]
    fn create_scene_command_round_trips_typed_target_and_result() {
        let project_id = ProjectId::new_v4();
        let command = ProjectWriteCommand::new(ProjectWriteRequest::CreateScene(
            ProjectCreateSceneRequest {
                project_id,
                target: ProjectWriteTarget::Project(project_id),
                name: "Main Scene".to_owned(),
            },
        ));
        let encoded = serde_json::to_string(&command).unwrap();
        assert!(!encoded.contains("PathBuf"));
        assert_eq!(
            command,
            serde_json::from_str::<ProjectWriteCommand>(&encoded).unwrap()
        );
    }

    #[test]
    fn lifecycle_commands_round_trip_without_filesystem_paths() {
        let project_id = ProjectId::new_v4();
        let parent_scene_id = usd_project::SceneId::new_v4();
        let requests = [
            ProjectWriteRequest::RemoveProject(ProjectLifecycleRequest { project_id }),
            ProjectWriteRequest::DeleteProject(ProjectLifecycleRequest { project_id }),
            ProjectWriteRequest::DeleteScene(ProjectDeleteSceneRequest {
                project_id,
                scene_id: parent_scene_id,
            }),
            ProjectWriteRequest::DeleteModel(ProjectDeleteModelRequest {
                project_id,
                model_id: usd_project::ModelId::new_v4(),
            }),
        ];

        for request in requests {
            let command = ProjectWriteCommand::new(request);
            let encoded = serde_json::to_string(&command).unwrap();
            assert!(!encoded.contains("/Users/"));
            assert_eq!(
                command,
                serde_json::from_str::<ProjectWriteCommand>(&encoded).unwrap()
            );
        }
    }
}
