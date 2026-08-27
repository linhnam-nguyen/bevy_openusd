# Viewer Settings DLSS provider

The `usdview` DLSS integration is opt-in. Normal builds do not enable the
NVIDIA SDK path:

```text
cargo check --workspace
```

An explicit provider build uses Bevy 0.19's optional `dlss` feature:

```text
cargo check -p usdview --features dlss
cargo run -p usdview --features dlss -- --headless --webrtc <stage.usda>
```

The feature requires a Bevy-supported DLSS environment: an NVIDIA RTX GPU,
the Windows or Linux Vulkan backend, the Vulkan SDK (`VULKAN_SDK`), clang for
bindgen, and the non-redistributable DLSS SDK (`DLSS_SDK`) setup described by
Bevy's `dlss_wgpu` integration. Builds without that environment must keep the
feature disabled. The application project ID is registered before
`DefaultPlugins`; it is public application metadata and is not a credential.

This Apple development host is not a supported DLSS/Vulkan target for the
provider feature. The feature must be compiled and exercised on the supported
Windows/Linux Vulkan target; the normal feature-off workspace gate remains the
portable check on Apple.

Runtime availability is true only when Bevy inserts
`DlssSuperResolutionSupported` after renderer initialization. A GPU name or
operating-system guess is never sufficient. If that resource is absent, the
USDHub adapter keeps the capability disabled and removes any requested DLSS
camera component.

The camera provider boundary preserves the existing output path: the
headless `OffscreenTarget` remains the final output-size image consumed by
`FrameCapturePlugin` and the encoder. A Vulkan runtime verification packet
must additionally record the DLSS internal render resolution, output
resolution, capture dimensions, GPU frame time, and encoded frame dimensions.
