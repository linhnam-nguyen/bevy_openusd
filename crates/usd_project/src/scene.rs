use serde::{Deserialize, Serialize};

use crate::{ModelId, SceneId, SceneMemberId};

/// The stable object identity targeted by one Scene placement.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum SceneMemberTarget {
    Scene(SceneId),
    Model(ModelId),
}

/// One placement relationship owned by a Scene.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SceneMember {
    pub id: SceneMemberId,
    pub target: SceneMemberTarget,
    pub name: Option<String>,
}
