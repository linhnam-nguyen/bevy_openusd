use anyhow::{Context, Result, bail, ensure};
use openusd::{gf::Matrix4d, sdf::Value, usd::Prim};
use usd_project::ScenePlacementTransform;

pub(crate) fn author_scene_member_transform(
    member_prim: &Prim,
    transform: ScenePlacementTransform,
) -> Result<()> {
    member_prim
        .create_attribute("xformOp:transform", "matrix4d")?
        .set_custom(false)?
        .set(Value::Matrix4d(Matrix4d(transform.0)))?;
    member_prim
        .create_attribute("xformOpOrder", "token[]")?
        .set_custom(false)?
        .set(Value::TokenVec(vec!["xformOp:transform".into()]))?;
    Ok(())
}

pub(super) fn read_scene_member_transform(member_prim: &Prim) -> Result<ScenePlacementTransform> {
    let Some(value) = member_prim.attribute("xformOp:transform").get::<Value>()? else {
        return Ok(ScenePlacementTransform::IDENTITY);
    };
    let Value::Matrix4d(matrix) = value else {
        bail!("Project Scene placement transform must be matrix4d");
    };
    let order = member_prim
        .attribute("xformOpOrder")
        .get::<Value>()?
        .context("Project Scene placement is missing xformOpOrder")?;
    ensure!(
        order == Value::TokenVec(vec!["xformOp:transform".into()]),
        "Project Scene placement must use only xformOp:transform"
    );
    Ok(ScenePlacementTransform(matrix.0))
}
