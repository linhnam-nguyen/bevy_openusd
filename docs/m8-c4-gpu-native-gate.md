# M8-C4 GPU-native PointInstancer gate

Decision: **NO CHANGE RECOMMENDED / SENIOR GATE NOT OPEN**.

M8 has no controlled renderer extraction, draw, GPU-memory, or picking
comparison for a custom PointInstancer path. The current data plane therefore
stays on shared Bevy mesh/material handles with one logical entity per visible
source row. `UsdInstanceId` keeps source identity independent of any future
renderer instance index.

Opening this gate requires fresh evidence for transforms, multiple prototypes,
authored `invisibleIds`, picking/selection, live changes, reload, and framing,
plus a before/after benchmark showing a meaningful benefit. No GPU-native
implementation is included in M8.
