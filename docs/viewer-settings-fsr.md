# Viewer Settings FSR provider boundary

Bevy 0.19 does not provide a native FSR super-resolution implementation. The
USDHub sampling module therefore owns a renderer-local `FsrVulkanProvider`
adapter rather than inventing a `bevy::fsr` API or exposing FidelityFX/Vulkan
objects through `viewport_protocol`.

On Linux and Windows, the opt-in `fsr_vulkan` feature integrates the pinned
`fsr` 0.1.11 Rust bindings with Bevy's Vulkan HAL. The provider is registered
in `Core3dSystems::EarlyPostProcess`, creates an FSR2 context from Bevy's live
Vulkan instance/adapter/device, and dispatches the GPU workload from the
current render command encoder. The `fsr` project is archived, so the version
is pinned deliberately and remains an isolated adapter dependency pending a
maintained replacement.

The camera binding supplies the real provider inputs: a 0.667 render-scale
`MainPassResolutionOverride`, Bevy depth and motion-vector prepasses, and
storage-capable main textures. FidelityFX writes the upscaled result into the
post-process destination before Bevy tone mapping. The output remains the
existing `OffscreenTarget`, so frame capture still reads the final image. The
failure path copies the source to the destination and marks the provider
unavailable; it never inserts a CPU readback.

`FsrVulkanCapability` remains fail-closed on unsupported targets and is updated
from the render-world provider after a real context/dispatch attempt. Physical
GPU/runtime proof is still deferred to a supported Vulkan host.

The adapter contract rejects frames unless the input extent is lower than the
output extent, motion vectors/depth/exposure are present, and no CPU readback
is inserted between rendering and the provider. A supported-host verification
packet must record input/output resolutions, the GPU dispatch time, capture
dimensions, and encoded frame dimensions. The provider must not change the
frontend or wire protocol.
