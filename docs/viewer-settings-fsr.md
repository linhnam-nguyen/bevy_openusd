# Viewer Settings FSR provider status

FSR is a pending feature and is intentionally not implemented or advertised
in B4. The current renderer publishes `fsr_available = false` and keeps
`SamplingProvider::Fsr` and `ActiveUpscaler::Fsr` only as forward-compatible
renderer-neutral vocabulary.

The active B4 sampling policy is:

```text
Sampling Off
    -> None

Sampling On
    -> DLSS when the runtime capability is supported
    -> explicit unsupported result otherwise
```

The experimental FSR2 Vulkan adapter, its `fsr_vulkan` feature, and the
`fsr` 0.1.11 dependency were removed from the production path. No FSR camera
binding, render-world backend, GPU dispatch, or CPU readback path remains in
the current renderer.

FSR remains deferred because the accepted official AMD source paths do not
provide the required combination for this milestone:

- FidelityFX SDK `v1.1.4` includes an open-source Vulkan backend, but its FSR3
  upscaler is version 3.1.4.
- FidelityFX SDK `v2.0.0` contains FSR3 upscaler 3.1.5, but only the DX12
  backend/runtime is shipped for that upscaler.

USDHub will not create a custom or unofficial Vulkan port to bridge that gap.
When AMD provides a reviewed official Vulkan-capable implementation, it can
be added behind the existing provider boundary without changing the frontend
or `viewport_protocol` vocabulary.
