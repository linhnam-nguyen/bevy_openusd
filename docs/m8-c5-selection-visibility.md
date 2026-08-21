# M8-C5 selection and visibility contract

Decision: **ACCEPTED on the shared-asset logical-entity path**.

- `UsdInstanceId.logical_id` is the authored `ids[i]` value; the source row is
  used only when the `ids` attribute is unauthored.
- `UsdInstanceId.source_index` is the current PointInstancer source-row index.
- `UsdInstanceId.prototype_index` is the authored prototype relationship index.
- Hidden rows are omitted from the ECS projection, so they cannot render or be
  selected through a projected instance entity.
- Visible-row ordering remains the source ordering with hidden rows removed;
  the compacted order is never used as logical identity.
- `PointInstancerSelection` identifies `(instancer prim path, logical ID)`, not
  a replaceable Bevy entity. The viewport highlight resolves the current child
  from that key after a reproject.
- Reprojection removes only children carrying `UsdInstance` and recreates the
  current visible set from the stage source of truth. Hidden logical IDs have
  no projected/rendered/pickable child.
- Shared mesh/material handles are independent of selection identity.
- `PointInstancerDependencyIndex` registers prototype roots and routes a live
  prototype edit only to dependent instancers; removed instancers unregister
  their reverse-index entries.

The M8 correctness suite proves authored-ID `invisibleIds`, two prototype
indices, logical selection across entity replacement, hidden-ID removal,
changed-prototype propagation through `LiveStagePlugin`, and transform-only
selection/entity preservation. SceneAnchor remains outside the renderer route
and no frontend or Instance 2 folder is modified.
