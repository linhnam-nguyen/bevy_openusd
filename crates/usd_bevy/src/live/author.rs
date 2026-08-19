use anyhow::Result;
use bevy::prelude::*;
use openusd::sdf::Value;
use openusd::usd::Stage;

use super::projection::to_bevy_transform;
use crate::read::xform::read_transform;

/// Author `transform` onto `prim_path` as `xformOp:transform`. Errors if the
/// path is malformed or the layer rejects the edit.
pub fn author_transform(stage: &Stage, prim_path: &str, transform: &Transform) -> Result<()> {
    let prim = openusd::sdf::path(prim_path)?;
    let cols = Mat4::from_scale_rotation_translation(
        transform.scale,
        transform.rotation,
        transform.translation,
    )
    .to_cols_array();
    let m: [f64; 16] = std::array::from_fn(|i| cols[i] as f64);

    let xop = prim.append_property("xformOp:transform")?;
    stage
        .create_attribute(xop, "matrix4d")?
        .set(Value::Matrix4d(openusd::gf::Matrix4d(m)))?;
    let order = prim.append_property("xformOpOrder")?;
    stage
        .create_attribute(order, "token[]")?
        .set(Value::TokenVec(vec!["xformOp:transform".into()]))?;
    Ok(())
}

/// Current authored transform of a prim, if any.
pub fn current_transform(stage: &Stage, prim_path: &str) -> Option<Transform> {
    openusd::sdf::path(prim_path)
        .ok()
        .and_then(|p| read_transform(stage, &p).ok().flatten())
        .map(to_bevy_transform)
}

fn clear_transform(stage: &Stage, prim_path: &str) -> Result<()> {
    let prim = openusd::sdf::path(prim_path)?;
    let _ = stage.remove_property(prim.append_property("xformOp:transform")?);
    let _ = stage.remove_property(prim.append_property("xformOpOrder")?);
    Ok(())
}

struct TransformEdit {
    prim: String,
    before: Option<Transform>,
    after: Transform,
}

/// Undo/redo stack for transform edits.
#[derive(Default)]
pub struct TransformHistory {
    undo: Vec<TransformEdit>,
    redo: Vec<TransformEdit>,
}

impl TransformHistory {
    /// Author `after` onto `prim`, recording the prior transform for undo.
    pub fn author(&mut self, stage: &Stage, prim: &str, after: Transform) -> Result<()> {
        let before = current_transform(stage, prim);
        author_transform(stage, prim, &after)?;
        self.undo.push(TransformEdit {
            prim: prim.to_string(),
            before,
            after,
        });
        self.redo.clear();
        Ok(())
    }

    /// Undo the most recent edit. Returns `false` if nothing to undo.
    pub fn undo(&mut self, stage: &Stage) -> Result<bool> {
        let Some(edit) = self.undo.pop() else {
            return Ok(false);
        };
        match &edit.before {
            Some(t) => author_transform(stage, &edit.prim, t)?,
            None => clear_transform(stage, &edit.prim)?,
        }
        self.redo.push(edit);
        Ok(true)
    }

    /// Redo the most recently undone edit. Returns `false` if nothing to redo.
    pub fn redo(&mut self, stage: &Stage) -> Result<bool> {
        let Some(edit) = self.redo.pop() else {
            return Ok(false);
        };
        author_transform(stage, &edit.prim, &edit.after)?;
        self.undo.push(edit);
        Ok(true)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}
