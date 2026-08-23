# Viewer Settings FSR provider boundary

Bevy 0.19 does not provide a native FSR super-resolution implementation. The
USDHub sampling module therefore owns a renderer-local `FsrVulkanProvider`
contract rather than inventing a `bevy::fsr` API or exposing FidelityFX/Vulkan
objects through `viewport_protocol`.

The provider is fail-closed in this checkout because no reviewed FidelityFX
Vulkan backend or SDK is present. `FsrVulkanCapability` remains unavailable
until an integration supplies all of these runtime facts:

- a Vulkan backend;
- a FidelityFX backend;
- the render-graph input contract.

The adapter contract also rejects frames unless the input extent is lower than
the output extent, motion vectors/depth/exposure are present, and no CPU
readback is inserted between rendering and the provider. These checks prove
the ownership and failure boundary only; they do not claim a live FSR dispatch.

A future backend must implement the dispatch at the reviewed render-graph
post-process point and provide a Vulkan runtime packet containing input/output
resolutions, input resource identities, GPU time, capture dimensions, and
encoded frame dimensions. It must not change the frontend or wire protocol.
