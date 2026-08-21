# M8-C5 selection and visibility contract

Decision: **ACCEPTED on the shared-asset logical-entity path**.

- `UsdInstanceId.index` is the authored PointInstancer source-row index.
- `UsdInstanceId.prototype_index` is the authored prototype relationship index.
- Hidden rows are omitted from the ECS projection, so they cannot render or be
  selected through a projected instance entity.
- Visible-row ordering remains the source ordering with hidden rows removed;
  the compacted order is never used as logical identity.
- Reprojection removes only children carrying `UsdInstance` and recreates the
  current visible set from the stage source of truth.
- Shared mesh/material handles are independent of selection identity.

The M8 correctness suite proves authored-ID `invisibleIds`, two prototype
indices, stable source-row IDs, reprojection, and changed-prototype cache
invalidation. SceneAnchor remains outside the renderer route and no frontend or
Instance 2 folder is modified.
