use std::path::PathBuf;

use crate::project::cache_contract::SceneCacheDescriptorV3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RepairPhase {
    NeedScenePayload,
    NeedProjectLookup {
        project_root: PathBuf,
        path: String,
        expected_descriptor: SceneCacheDescriptorV3,
    },
    CacheReady {
        project_root: PathBuf,
        path: String,
        expected_descriptor: SceneCacheDescriptorV3,
    },
}

impl RepairPhase {
    pub(crate) fn lookup_parts(
        &self,
    ) -> Option<(&PathBuf, &str, &SceneCacheDescriptorV3)> {
        match self {
            Self::NeedScenePayload => None,
            Self::NeedProjectLookup {
                project_root,
                path,
                expected_descriptor,
            }
            | Self::CacheReady {
                project_root,
                path,
                expected_descriptor,
            } => Some((project_root, path, expected_descriptor)),
        }
    }

    pub(crate) fn is_scene_payload(&self) -> bool {
        matches!(self, Self::NeedScenePayload)
    }
}
