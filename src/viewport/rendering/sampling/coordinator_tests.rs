use super::coordinator::{
    ActiveUpscaler, SamplingCapabilities, SamplingSelectionError, choose_upscaler,
};

const NO_PROVIDERS: SamplingCapabilities = SamplingCapabilities::new(false, false);
const DLSS_ONLY: SamplingCapabilities = SamplingCapabilities::new(true, false);
const FSR_ONLY: SamplingCapabilities = SamplingCapabilities::new(false, true);
const BOTH_PROVIDERS: SamplingCapabilities = SamplingCapabilities::new(true, true);

#[test]
fn disabled_sampling_always_selects_none() {
    for capabilities in [NO_PROVIDERS, DLSS_ONLY, FSR_ONLY, BOTH_PROVIDERS] {
        assert_eq!(
            choose_upscaler(false, capabilities),
            Ok(ActiveUpscaler::None)
        );
    }
}

#[test]
fn dlss_is_preferred_when_both_providers_are_available() {
    assert_eq!(
        choose_upscaler(true, BOTH_PROVIDERS),
        Ok(ActiveUpscaler::Dlss)
    );
}

#[test]
fn dlss_is_selected_when_it_is_the_only_available_provider() {
    assert_eq!(choose_upscaler(true, DLSS_ONLY), Ok(ActiveUpscaler::Dlss));
}

#[test]
fn fsr_is_selected_when_dlss_is_unavailable() {
    assert_eq!(choose_upscaler(true, FSR_ONLY), Ok(ActiveUpscaler::Fsr));
}

#[test]
fn no_provider_returns_an_explicit_unsupported_result() {
    assert_eq!(
        choose_upscaler(true, NO_PROVIDERS),
        Err(SamplingSelectionError::NoProviderAvailable)
    );
}

#[test]
fn provider_choice_is_deterministic() {
    let cases = [
        (false, NO_PROVIDERS, Ok(ActiveUpscaler::None)),
        (false, BOTH_PROVIDERS, Ok(ActiveUpscaler::None)),
        (true, DLSS_ONLY, Ok(ActiveUpscaler::Dlss)),
        (true, FSR_ONLY, Ok(ActiveUpscaler::Fsr)),
        (true, BOTH_PROVIDERS, Ok(ActiveUpscaler::Dlss)),
        (
            true,
            NO_PROVIDERS,
            Err(SamplingSelectionError::NoProviderAvailable),
        ),
    ];

    for (preference_enabled, capabilities, expected) in cases {
        for _ in 0..8 {
            assert_eq!(choose_upscaler(preference_enabled, capabilities), expected);
        }
    }
}
