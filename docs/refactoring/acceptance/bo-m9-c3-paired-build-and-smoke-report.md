# BO-M9-C3 Acceptance Manifest: Paired-Build Compatibility, Smoke Gates & CI Evidence

- **Mission:** `M-BO-M1-C0-20261109-CDS-LNN-01`
- **Checkpoint:** `BO-M9-C3` (Iteration 2)
- **Exact Repository Pair:**
  - `bevy_openusd`: branch `develop/refactoring`, commit `7097ae866a522a96cc67cfa5d933930e368002cb`
  - `UsdHubUI`: branch `refactoring`, commit `fa2faeef54e107cefbb0287ecf8830a0d45eba6f` (`git -C UsdHubUI status --short` clean before & after)
  - Sibling dependencies resolved by `bevy_openusd`: `bevy_frost@da3110c07ba8d79d6deeba5a68dd536231956fb9`, `bevy_glacial@6bef365be227e1461c1f8019e3b8c631c166fd44`
- **Host Environment:** Windows 11 (`x86_64-pc-windows-msvc`), Rust `1.99.0`, Python `3.12`, GStreamer MSVC `1.0` (`PKG_CONFIG` set, `CARGO_BUILD_JOBS=1`), NVIDIA GeForce RTX 5090 Laptop GPU (Vulkan driver `610.74`)
- **Raw Diagnostics Folders:** `AgentsComm/communication/agents/implementer/BO-M9-C3-I1/` and `AgentsComm/communication/agents/implementer/BO-M9-C3-I2/`

---

## 1. Exact Path-Dependency Compatibility Matrix

| Consumer Manifest | Sibling Crate | Declared Relative Path | `Cargo.lock` Version | Protocol / API Diff (`294d28e0..7097ae86`) | Compatibility Status |
| --- | --- | --- | --- | --- | --- |
| `UsdHubUI/Cargo.toml` (`usd_hub_desktop`, `viewport_client`) | `viewport_protocol` | `../bevy_openusd/crates/viewport_protocol` | `0.1.0` | `0` public/wire changes (test-only `#[path]` relocation) | **COMPATIBLE** |
| `UsdHubUI/Cargo.toml` (`usd_hub_desktop`) | `project_protocol` | `../bevy_openusd/crates/project_protocol` | `0.1.0` | `0` public/wire changes (inline tests moved to `tests/contracts.rs`) | **COMPATIBLE** |
| `UsdHubUI/Cargo.toml` (`usd_hub_desktop`) | `usd_project` | `../bevy_openusd/crates/usd_project` | `0.0.1` | `0` public/API changes (test-only `#[path]` relocation) | **COMPATIBLE** |
| `UsdHubUI/src-tauri/Cargo.toml` (`usd_hub_desktop_host`) | `project_protocol` | `../../bevy_openusd/crates/project_protocol` | `0.1.0` | `0` public/wire changes | **COMPATIBLE** |
| `UsdHubUI/src-tauri/Cargo.toml` (`usd_hub_desktop_host`) | `usd_project` | `../../bevy_openusd/crates/usd_project` | `0.0.1` | `0` public/API changes | **COMPATIBLE** |
| `UsdHubUI/src-tauri/Cargo.toml` (`usd_hub_desktop_host`) | `usdview` | `../../bevy_openusd` | `0.0.1` | `src/project/service` API preserved; compiles & links cleanly | **COMPATIBLE** |

---

## 2. Paired Build & Diagnostic Test Results

The paired Compile commands and the focused Rust/guard diagnostics below passed against the exact checkout pair `7097ae86` + `fa2faeef`:

| Repository | Command | Exit | Result Summary | Log Artifact (`BO-M9-C3-I1/`) |
| --- | --- | ---: | --- | --- |
| `UsdHubUI` | `cargo fmt -p usd_hub_desktop -p viewport_client -- --check` | 0 | Clean | `ui-cargo-fmt.log` |
| `UsdHubUI` | `cargo check --workspace --locked` | 0 | Clean | `ui-cargo-check-workspace.log` |
| `UsdHubUI` | `cargo check --package usd_hub_desktop --target wasm32-unknown-unknown --locked` | 0 | Clean | `ui-cargo-check-wasm.log` |
| `UsdHubUI` | `cargo check --manifest-path src-tauri/Cargo.toml --locked` | 0* | Clean (`18.13s` on isolated rerun; *initial concurrent run hit `E0786` `libserde-*.rmeta` mmap failure; contention is the reported hypothesis) | `ui-cargo-check-tauri-rerun.log` |
| `UsdHubUI` | `cargo test --workspace --locked` | 0 | `usd_hub_desktop`: **554 passed, 0 failed, 1 ignored**; `viewport_client`: **11 passed, 0 failed** | `ui-cargo-test-workspace.log` |
| `UsdHubUI` | `cargo test --manifest-path src-tauri/Cargo.toml --locked` | 0 | `usd_hub_desktop_host`: **2 passed, 0 failed** | `ui-cargo-test-tauri.log` |
| `UsdHubUI` | `pnpm frontend:build` | 0 | CSS + Trunk WASM bundle built (`36.18s`), `git status --short` clean | `ui-pnpm-frontend-build.log` |
| `UsdHubUI` | `pnpm fluent:tokens:check`, `pnpm fluent:icons:check`, `pnpm rust:test-source-layout`, `fluent:tokens:test` (`6`), `fluent:icons:test` (`4`), `rust:test-source-layout:test` (`16+1 skipped`), `frontend:build:test` (`5`) | 0 | All 7 UI guard & script suites **PASS** | `ui-pnpm-checks.log` |
| `bevy_openusd` | `cargo test --locked --test test_architecture` | 0 | **2 passed, 0 failed** (`5.12s`) | `backend-paired-crate-tests.log` |
| `bevy_openusd` | `cargo test --locked -p project_protocol --all-targets` | 0 | **21 passed, 0 failed** | `backend-paired-crate-tests.log` |
| `bevy_openusd` | `cargo test --locked -p usd_project --all-targets` | 0 | **23 passed, 0 failed** | `backend-paired-crate-tests.log` |
| `bevy_openusd` | `cargo test --locked -p viewport_protocol --all-targets` | 0 | **73 passed, 0 failed** (`2` lib + `71` across 7 integration targets) | `backend-paired-crate-tests.log` |
| `bevy_openusd` | `cargo test --locked -p viewport_streaming --all-targets` | 0 | **58 passed, 0 failed** (`40` lib + `18` `relocated_contracts`) | `backend-paired-crate-tests.log` |

*Note on formal `Validation.py --layer test`:** per Protocol v8 separation of duties, full workspace `Validation.py --layer test` remains owned by the Local Agent; the Implementer executed `Validation.py --layer compile` (Section 6) and the paired diagnostic checks above.*

---

## 3. UI-R-C4 Stream / WebRTC / Native / Playwright Smoke Gate Evidence

Evidence is explicitly separated between **in-memory/unit protocol contract tests**, **live headless backend WebRTC execution**, **Playwright browser fixture UI tests**, and **unverified interactive Tauri+WebRTC end-to-end flows**:

### 3.1 Protocol Serialization, State Machine & Transport Contracts (PASSED)
- **Cross-repo envelope round-trip:** `UsdHubUI::apps/desktop/tests/unit/features/viewport/command_gateway_tests.rs` (`ui_viewport_and_project_commands_round_trip_through_shared_protocol_types`) round-trips UI-constructed `Handshake`, `ViewportCommand::RequestSnapshot`, `ServerEvent::Session(Pong)`, `ProjectActivationCommand`, and `ProjectActivationReply` directly through sibling `viewport_protocol`, `project_protocol`, and `usd_project`.
- **Client state machine & resilience:** `viewport_client` (`11/11` passed) covers action-ordered queueing, failed-write front-requeueing, contiguous `SelectionDeltaApplied` revisions, `ViewerSettingsChanged` authoritative updates, `Ready` vs `StageLoadState::Idle` separation, snapshot recovery, command rejection preserving connection, incompatible protocol version rejection, and disconnect retention.
- **WebRTC browser-side adapter & UI retention:** `UsdHubUI` unit suites verify DataChannel label/protocol classification (`viewport-control` / `usd-hub.viewport.v1`, `viewport-input` / `usd-hub.viewport-input.v1`), resize hysteresis, decoded-frame monotonicity, retained Viewport single-instance mounting, and active-project viewport gating (`root_navigation_tests`, `platform::webrtc::*`).
- **Backend streaming contracts:** `viewport_streaming` (`58/58` passed) verifies monotonic stream generation across WebView reconnects, resize/DPR/FPS normalization (`StreamLimits`), SDP/codec negotiation, `RenderServerInterface` command correlation, and semantic sync routing.

### 3.2 Live Headless `usdview --headless --webrtc` + WebSocket Signaling Probe (PASSED)
- **Command:** `target/debug/usdview.exe --headless --webrtc --preset adaptive --codec auto` (with GStreamer `1.0\msvc_x86_64\bin` on `PATH` and isolated `USDHUB_PROJECT_WORKSPACE_REGISTRY`).
- **Observed Execution (`usdview-headless-webrtc-stderr.log`, `webrtc-smoke-offer.sdp`):**
  1. Initialized Vulkan GPU renderer on `NVIDIA GeForce RTX 5090 Laptop GPU` (driver `610.74`).
  2. Started `viewport_streaming::session` and bound WebSocket signaling listener on `ws://127.0.0.1:8080`.
  3. Opened and projected `assets\animated_spinner.usda` (`3 prims (1 animated)`, `projection_complete duration_ms=3.4845`, `admitted progressive revision=2`, `published coalesced revision=2`).
  4. An initial WebSocket handshake failed; the second connection accepted live WebSocket HTTP `101 Switching Protocols` upgrade and client `SignalingMessage::Join` (`1280x720 @ 60 fps`, `codecs=["h264","h265","av1"]`).
  5. Enumerated GStreamer encoders (`nvav1enc`, `qsvav1enc`, `svtav1enc`, `nvh265enc`, `qsvh265enc`, `x265enc`, `nvh264enc`, `qsvh264enc`, `x264enc`), selected `nvav1enc` (`KnownDiscreteGpu`), negotiated RTP `AV1/90000` (`pt=96`), and returned a reported 905-byte payload (stored SDP file 935 bytes), with 2 media sections in the SDP `SignalingMessage::Offer` (`m=video 9 UDP/TLS/RTP/SAVPF 96` + `m=application 0 UDP/DTLS/SCTP webrtc-datachannel`).
  6. Closed cleanly on disconnect and terminated cleanly via `taskkill` with zero stray processes.

### 3.2b Real Chromium Browser `RTCPeerConnection` <-> Live `usdview --headless --webrtc` Smoke Gate (PASSED, I2)
- **Command & Harness (`BO-M9-C3-I2/webrtc-chromium-smoke.mjs`, raw output `BO-M9-C3-I2/webrtc-chromium-smoke.json`):**
  - Backend: `target/debug/usdview.exe --headless --webrtc --preset adaptive --codec auto d:/Nemeton/bevy_openusd/tests/fixtures/usd/primitives.usda` (with GStreamer `1.0\msvc_x86_64\bin` on `PATH` and isolated `USDHUB_PROJECT_WORKSPACE_REGISTRY`).
  - Client: real headless Playwright Chromium `RTCPeerConnection` + `WebSocket("ws://127.0.0.1:8080")` + `<video autoplay muted playsinline>`.
- **Verified End-to-End Flow:**
  1. Loaded `primitives.usda` cleanly (`admitted progressive revision=2 admitted=6 pending=0 prims=6`, `camera framed on scene: focus=Vec3(-0.034999967, 0.36, 0.35000002), diag=3.77 m (effective=3.77 m), 7 prims`, `published coalesced revision=2 prims=6`).
  2. Completed WebSocket `Join -> Offer -> Answer` + ICE trickle to `pc.iceConnectionState = connected`, `pc.connectionState = connected`, `candidatePairSucceeded = true` (`currentRoundTripTime: 0.001`).
  3. Opened both server-created WebRTC DataChannels: `viewport-control` (`protocol: "usd-hub.viewport.v1"`, `ordered: true`) and `viewport-input` (`protocol: "usd-hub.viewport-input.v1"`, `ordered: false`).
  4. Executed Protocol v11 control handshake and commands over `viewport-control`: `ClientHello` -> `ServerHello` (`session_id: "session-2"`, `codecs: ["av1"]`) -> `ConfigurationAccepted` (`1280x720`, `generation: 1`) -> `Ready` -> `Snapshot` -> `SemanticSyncStatus`; `SessionCommand::Ping` (`"bo-m9-c3-i2-ping"`) -> `Pong`; `SessionCommand::RequestSnapshot` -> `Snapshot`; `StreamCommand::ConfigureViewport` (`960x540 @ 60fps`, `generation: 2`) -> `ConfigurationAccepted` -> `PresentationChanged` -> `ConfigurationApplied` (backend `[viewport-resize] applied stream configuration 960x540 generation 2`).
  5. Delivered `InputCommand::PointerMotion` (`sequence: 1`, `x=320, y=180, dx=12, dy=-8`, `generation: 1`) over `viewport-input` with `readyState: "open"` and zero transport errors.
  6. Verified real hardware-encoded `nvav1enc` (`KnownDiscreteGpu`) `video/AV1` RTP streaming and browser `<video>` decoding via `RTCPeerConnection.getStats()`: `packetsReceived: 3384`, `packetsLost: 0`, `bytesReceived: 4466721`, `framesReceived: 251`, **`framesDecoded: 250`**, `keyFramesDecoded: 5`, `frameWidth: 960`, `frameHeight: 540`, `framesPerSecond: 57`, and `<video>.videoWidth == 960`, `videoHeight == 540`, `readyState == 4` (`HAVE_ENOUGH_DATA`). Cleanly closed session and terminated `usdview.exe` with zero stray processes.

### 3.3 Playwright Browser UI Gates (`Trunk.ui-e2e.toml`, Browser-Only Fixture Harness)
- `pnpm test:ui:a11y`: **23 passed, 0 failed (`exit 0`, `29.9s`)** (`BO-M9-C3-I1/ui-playwright-a11y.log`).
- `pnpm test:ui:e2e`: **26 passed, 35 failed (`exit 1`, `3.2m`)** (`BO-M9-C3-I1/ui-playwright-navigation.log`; exact per-spec breakdown in `BO-M9-C3-I2/playwright-35-failures.md`). `23` `@a11y` + `3` `navigation.spec.js` retained-viewport/report checks passed; all `35` failures belong strictly to `UsdHubUI@fa2faeef`:
  - `26` missing `win32` visual baseline PNGs in `tests/ui/fluent.spec.js` (`37`, `61`, `75`, `100`, `160`, `243`; `UsdHubUI` ships `9` `win32` dark snapshots and `tests/ui/README.md` designates Linux Chromium for C2 visual baselines).
  - `2` `locator.check` `10000ms` timeouts + missing `win32` snapshots in `tests/ui/fluent.spec.js:136:3` (dark/light BIM Data floating panel toggle).
  - `5` UI selector/attribute assertions in `UsdHubUI`: `fluent.spec.js:26:1` (button `'Switch to Light theme'` not found; `harness.js` uses `/^Switch to (Light|Dark).* theme$/`), `fluent.spec.js:118:3` dark & light (`status` notice expected `"info"`, received `""` on browser-harness rejection message), `navigation.spec.js:9:1` (`Viewport` nav button not found / expected disabled), and `navigation.spec.js:113:1` (`'0 projects'` text not found).
- **Runtime isolation proof (why Playwright E2E failures cannot originate in `bevy_openusd`):** `apps/desktop/Trunk.ui-e2e.toml` compiles `apps/desktop/index.ui-e2e.html` (`--cargo-features ui-test-diagnostics`). Every browser session invokes `installReadOnlyNativeHost(page)` in `UsdHubUI/tests/ui/support/fixtures.js` -> `native-host.js`, which overrides `window.WebSocket` with `PendingHarnessWebSocket` (`readyState = CONNECTING = 0`, `send() {}`, logging `[websocket] native signaling is held pending by the browser-only harness` in all `35` failure attachments) and stubs `window.__TAURI_INTERNALS__.invoke` with the hardcoded JS object `C2 UI Fixture Project`. Thus Playwright `test:ui:e2e` makes **zero runtime network/IPC calls** to `usdview`, `usd_project`, `project_protocol`, or `viewport_protocol`; compile-time compatibility of `UsdHubUI`'s Cargo path dependencies against `bevy_openusd@7097ae86` is separately proved by `cargo test --workspace --locked` (`565 passed, 0 failed`), `cargo check --target wasm32-unknown-unknown`, and `pnpm frontend:build` (`exit 0`). All temporary untracked `win32` PNGs remain cleaned (`git -C UsdHubUI status --short` empty).

### 3.4 Explicitly Unverified End-to-End Interactive Scope & Known Carried-Forward Limitations
- **Verified in Iteration 2 (live browser WebRTC peer <-> `usdview.exe`):** real `primitives.usda` load, `Join/Offer/Answer` + ICE/DTLS connected, both DataChannels (`viewport-control`, `viewport-input`), Protocol v11 handshake + `Ping/Pong` + `RequestSnapshot` + dynamic `ConfigureViewport` resize (`1280x720 -> 960x540`), `PointerMotion` over `viewport-input`, and real GPU `nvav1enc` AV1 RTP video decoding (`250` decoded frames).
- **Not executed end-to-end here (`NOT_VERIFIED`):** interactive OS-window Tauri WebView (`pnpm tauri:dev`) launching `usdview` concurrently in desktop GUI, multi-peer disconnect/reconnect retaining the UI Viewport component, and interactive scratch Project Scene creation/activation in GUI.
- **Known UI-R-C4 carried-forward observation (`authoritaritative_plan.md` line 3723):** in `UI-R-C4/I1`, Owner closed UI-R-C4 noting a pre-existing backend stage-activation issue (`"No active Scene to refresh"` during scratch Project Scene activation vs `animated_spinner.usda`), which was explicitly deferred out of the test-relocation mission (`BO-M1`–`BO-M9`).

---

### 3.5 Gate disposition and Local Agent evidence status (`E-01`)

| Gate | Outcome (`I1` + `I2`) | Evidence / Disposition |
| --- | --- | --- |
| Live headless GPU + `Join -> Offer -> Answer` + ICE/DTLS + DataChannels + dynamic resize + decoded video + input | **PASS** | `BO-M9-C3-I1/usdview-headless-webrtc-stderr.log`, `webrtc-smoke-offer.sdp`; `BO-M9-C3-I2/webrtc-chromium-smoke.mjs`, `webrtc-chromium-smoke.json` (`nvav1enc` AV1 `250` decoded frames at `960x540 @ 57 fps`, `Ping/Pong`, `RequestSnapshot`, `ConfigureViewport`, `PointerMotion`). |
| Browser fixture accessibility (`pnpm test:ui:a11y`) | **PASS (`23/23`)** | `BO-M9-C3-I1/ui-playwright-a11y.log` (Axe + keyboard focus + forced-colors + 200% reflow against `PendingHarnessWebSocket` + `C2 UI Fixture Project`). |
| Browser E2E (`pnpm test:ui:e2e`) | **FAIL (`26 pass / 35 fail`) — pre-existing `UsdHubUI` / missing `win32` snapshots, backend-isolated** | All `35` specs documented in `BO-M9-C3-I2/playwright-35-failures.md`; `native-host.js` stubs `WebSocket` & `__TAURI_INTERNALS__` (zero backend calls); actual `FAIL` preserved without modifying `UsdHubUI`. |
| Native OS-window Tauri GUI (`tauri:dev`) + scratch Project Scene activation | **NOT_RUN / NOT_VERIFIED** | Remains outside headless automation without live interactive desktop GUI; known `"No active Scene to refresh"` issue stays Owner-deferred (`authoritaritative_plan.md:3723`). |

## 4. CI & Parity Evidence Summary (`BO-M9-C2` + Hosted Run `43783801774`)

- **Primary CI Workflow (`.github/workflows/rust.yml`):**
  - Triggers on `push` and `pull_request` for `[main, develop, develop/refactoring]` plus `workflow_dispatch`; `permissions: contents: read`; `persist-credentials: false`; zero `continue-on-error` and zero `if:` skip conditions.
  - Checks out `bevy_openusd` alongside pinned siblings `bevy_frost@da3110c07ba8d79d6deeba5a68dd536231956fb9` and `bevy_glacial@6bef365be227e1461c1f8019e3b8c631c166fd44`.
- **Hosted GitHub Actions Execution at Exact Commit `7097ae866a522a96cc67cfa5d933930e368002cb` (`E-02`):**
  - **Run ID / URL:** [`43783801774`](https://github.com/linhnam-nguyen/bevy_openusd/actions/runs/43783801774) (`event: push`, `headBranch: develop/refactoring`, overall `conclusion: failure`; raw receipts `BO-M9-C3-I2/github-actions-43783801774.json`, `github-actions-43783801774-failed.log`, summary `github-actions-43783801774-summary.md`).
  - **Job `solari feature gate` (`133026800508`, `ubuntu-24.04`):** **`success`** — passed `cargo check --workspace --all-targets --features solari --locked`, `check_test_enumeration_parity.py --include-solari` (`violation_count: 0`), and `Run CPU-safe solari unit tests` (`11 passed; 0 failed; 0 ignored; 785 filtered out`).
  - **Job `build + test` (`133026800716`, `ubuntu-24.04`):** **`failure`** — passed Steps 1–15 (`check_test_architecture.py`, `50` Python guard/parity/CI self-tests, `cargo fmt --all -- --check`, `cargo check --workspace --all-targets --locked`, `cargo build --workspace --all-targets --locked`, `check_test_enumeration_parity.py`), and failed Step 16 (`cargo test --workspace --locked`, exit `101`); Step 17 (`Contract and architecture explicit targets`) was `skipped`. Inside Step 16, all preceding workspace crates passed (`247 passed; 0 failed; 2 ignored`) and `usdview --lib` reported `768 passed; 10 failed; 10 ignored`, where **all 10 failures** stemmed from missing runner Git identity (`git command failed: exit status: 128`, `Author identity unknown` / `fatal: empty ident name` -> `tests/support/project/lib.rs:41:5`, `lifecycle_import_tests.rs:70:5`, and 8 downstream `CommitFailed` panics).
- **Capability Gates Workflow (`.github/workflows/capability-gates.yml`):**
  - Explicit manual `workflow_dispatch` gates (`windows-os-gate`, `macos-external-assets-gate`, `dlss-gpu-gate`, `external-revit-and-profile-ignored-gate`): **`NOT_EXECUTED / NOT_VERIFIED`** for `7097ae866a522a96cc67cfa5d933930e368002cb` (no workflow dispatch run recorded on that SHA).
- **Live Local vs Cross-OS Draft Baselines (`scripts/test_enumeration_qualified_baseline.json`):**
  - Locally verified (`7097ae86`): `check_test_architecture.py` (`violations=0`), `50/50` Python self-tests (`OK`), live Windows enumeration parity (`1,966`/`1,185` default & `1,982`/`1,193` solari, `violation_count=0`), and all `UsdHubUI` + shared-protocol Cargo checks.
  - `windows:default` and `windows:solari` are `NATIVE_CAPTURED` (`native_host_verified: true`); `linux:{default,solari}` and `macos:{default,solari}` remain Owner-authorized synthetic Windows-derived drafts (`DRAFT_SYNTHETIC_WINDOWS_DERIVED_AWAITING_NATIVE_HOST`, `native_host_verified: false`) pending replacement on native Linux and macOS hosts.

---

### 4.1 Local Agent closure contract (`E-02`)

Preserve the six existing baseline configurations and two native Windows captures. When native Linux/macOS hosts are available, acquire exact C1 baseline captures at `59ccddbea7be85a34a8e8f1d4d131c91c7ade858` with pinned siblings and reconcile against the drafts before controlled replacement. Never derive a new expected baseline from current source or relabel synthetic evidence as native. Hosted run `43783801774` provides SHA-pinned `solari feature gate` PASS and `build + test` Steps 1–15 PASS / Step 16 Git-ident runner failure (`768 pass / 10 fail`); capability-gate manual dispatches remain pending/blocked unless dispatched on provisioned runners, and formal local `Validation.py --layer test` belongs to Local Agent Phase 5.

## 5. Formal Compile Receipt (`Validation.py --layer compile`)

Recorded after all `BO-M9-C3` Iteration 2 documentation edits were completed; see `AgentsComm/communication/agents/compile.log` (executed in `d:\Nemeton` via `Python312/python.exe -B AgentsComm/roles/local_agent_toolkit/scripts/Validation.py --layer compile`).

---

## 6. Acceptance Disposition

- **Implementer Verification (I2):**
  - `F-01` & `F-02` offline verifier (`verify_bo_m9_c3_ledger.py --root .`) passes (`status: "pass"`, confirming `1` unchanged + `36` `BO-M7`-normalized dormant blobs with all `45` definitions preserved, plus all `158` visibility hits).
  - Paired Compile/contract/guard checks, Windows C1 parity, and real Chromium <-> `usdview.exe --headless --webrtc` end-to-end streaming (`nvav1enc` AV1, DataChannels, dynamic `960x540` resize, `PointerMotion`, `250` decoded video frames) **PASSED**.
  - Playwright `@a11y` (`23/23`) **PASSED**; `test:ui:e2e` (`26 pass / 35 fail`) documented with complete 35-spec attribution and `PendingHarnessWebSocket` isolation proof; native OS-window `tauri:dev` GUI & scratch scene activation remain `NOT_VERIFIED`; hosted GitHub Actions run `43783801774` captured truthfully (`solari` PASS, `build + test` Steps 1–15 PASS, Step 16 Git-author-ident `10` failures).
- **Formal Workspace Test (`Validation.py --layer test`), Reviewer Verdict & Owner Signoff:** `PENDING` (owned by Local Agent / Reviewer / Owner).
