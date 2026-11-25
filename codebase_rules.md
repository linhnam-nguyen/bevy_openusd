# Nemeton — Codebase Rules

Applies to `UsdHubUI` and `bevy_openusd`. Keep production code small, explicit and aligned with the existing architecture.

- **Rust style standard:** Follow **ONLY the Google Rust Style Guide** (https://google.github.io/styleguide/rustguide.html; local digest at `.agents/skills/rust-best-practices/references/google-rust-style-guide.md`) for both frontend (`UsdHubUI`) and backend (`bevy_openusd`). Where this document specifies stricter project rules (such as Section 4 test placement under `tests/`), this document takes precedence.

## 1. Scope and ownership

- Each crate, folder, module, file and function must have a narrow, coherent responsibility matching its location. Place code with its functional owner, not with the task that introduced it.
- Extend the existing owner before creating a new module or crate. Split code when responsibilities diverge; avoid catch-all `utils`, `helpers`, `common` or `manager` modules without a clear domain.
- Keep functions and files short enough to understand locally. Prefer small named operations and cohesive modules over long routines, deep nesting or fragmentation into trivial wrappers.
- Reuse existing implementations. Do not duplicate state, validation, conversions, business rules or execution paths. Do not add speculative abstractions or unrelated changes.
- Keep module entry points (`mod.rs`, `lib.rs`) focused on declarations, exports and concise facade/wiring logic. Use the narrowest necessary visibility.

## 2. Respect architectural boundaries

| Location | Owns |
| --- | --- |
| Frontend `apps/desktop/src/components/` | Presentation, layout, local interaction and typed user intents. |
| Frontend `apps/desktop/src/features/` | Feature models, state, view models, controllers, actions, reducers and gateways. |
| Frontend `apps/desktop/src/platform/`, `src-tauri/` | Platform integration, native host, preferences and transport adapters. |
| Frontend `crates/viewport_client/` | Client protocol handling and read-model reduction. |
| Backend `src/<domain>/` | Application orchestration within the owning domain: project, document, BIM, semantic, viewport, history, cache, etc. |
| Backend `src/api/`, `src/transport/` | Command/query adaptation, publication and wire transport; not duplicated domain logic. |
| Backend `crates/` | Reusable domain libraries, USD integration and stable protocol/streaming contracts. |

- Keep the backend authoritative for USD, scene mutations, camera and rendering. The frontend sends typed intents and presents authoritative read models; it must not reproduce backend decisions or import Bevy/ECS or native networking APIs into UI components.
- Keep viewport navigation, presentation, rendering and stage responsibilities separate. Generic viewport code must not depend on the API bridge. Presentation-only changes must not mutate authored USD or history.
- Keep pure domain and protocol crates independent of UI, host and renderer implementations. Cross boundaries through existing typed contracts; update both consumers and contract tests when a contract changes.

## 3. Explicit naming

- Use names that describe enduring functionality: Rust `snake_case` modules/functions, `PascalCase` types/components, `SCREAMING_SNAKE_CASE` constants.
- Never name committed files, symbols, types, comments or tests after missions, milestones, checkpoints, ticket IDs or temporary implementation phases. Such labels may exist only as necessary uncommitted working names and must be removed before committing.
- For example, split `controller_m7.rs` and `controller_m14.rs` by actual responsibilities such as dialog dispatch, project creation or location selection. Rename all references consistently; do not merely replace the checkpoint with another vague label.
- Comments explain behavior, invariants and non-obvious decisions, not the implementation plan. Use meaningful domain terms consistently across repositories.

## 4. Tests and diagnostics

- Never place test implementations, test fixtures, mocks, assertions for tests or test-only support code inside any production `src/` directory. Put them under the owning package's `tests/` (`unit/`, `contracts/`, `fixtures/`, etc.); frontend browser tests belong in `tests/ui/`.
- Existing minimal `#[cfg(test)]` / `#[path = "..."]` declarations may link external test files when private Rust access is required. They must contain no test implementation. Do not broaden the production API solely for testing.
- Test actual behavior, invariants, failures and regressions. Keep fixtures deterministic; never introduce mock or test-only execution paths into normal builds. Production diagnostics are allowed only when serving a real runtime purpose.

## 5. Before committing

- Verify ownership, dependency direction, clear names and removal of temporary code, stale comments and duplicate implementations.
- Run formatting, relevant lint/check/build commands, and affected unit, contract and integration tests. Report any checks that could not run; never claim unverified success.
