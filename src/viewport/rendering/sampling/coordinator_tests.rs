use bevy::prelude::{App, Update};
use viewport_protocol::{SamplingProvider, ViewportEvent};

use crate::viewport::api::{ViewerSettingsState, ViewportEventOutbox};

use super::coordinator::{
    ActiveUpscaler, SamplingCapabilities, SamplingSelectionError, choose_upscaler,
};
use super::{
    DlssCameraActivation, DlssCapability, FsrVulkanCapability, SamplingCoordinatorState,
    publish_sampling_capabilities,
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

fn reconciliation_app(dlss: bool, fsr: bool, active: ActiveUpscaler) -> App {
    let mut app = App::new();
    app.insert_resource(DlssCapability::from_probe(dlss, dlss))
        .insert_resource(FsrVulkanCapability::from_probe(fsr, fsr, fsr))
        .insert_resource(SamplingCoordinatorState {
            preference_enabled: true,
            active,
        })
        .insert_resource(DlssCameraActivation {
            enabled: active == ActiveUpscaler::Dlss,
        })
        .insert_resource(ViewerSettingsState::default())
        .insert_resource(ViewportEventOutbox::default())
        .add_systems(Update, publish_sampling_capabilities);
    app
}

#[test]
fn dlss_loss_falls_back_to_fsr_and_publishes_authoritative_settings() {
    let mut app = reconciliation_app(false, true, ActiveUpscaler::Dlss);

    app.update();

    let coordinator = app.world().resource::<SamplingCoordinatorState>();
    assert_eq!(coordinator.active, ActiveUpscaler::Fsr);
    assert!(!app.world().resource::<DlssCameraActivation>().enabled);

    let settings = app.world().resource::<ViewerSettingsState>().read_model();
    assert_eq!(settings.capabilities.dlss_available, false);
    assert_eq!(settings.capabilities.fsr_available, true);
    assert_eq!(settings.sampling.provider, SamplingProvider::Fsr);

    let events = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .take_published();
    assert_eq!(events.len(), 1);
    let ViewportEvent::ViewerSettingsChanged {
        settings: event_settings,
    } = &events[0].event
    else {
        panic!("capability loss must publish viewer settings");
    };
    assert_eq!(event_settings.sampling.provider, SamplingProvider::Fsr);
}

#[test]
fn provider_loss_without_fallback_selects_none_and_publishes_unavailable_state() {
    let mut app = reconciliation_app(false, false, ActiveUpscaler::Dlss);

    app.update();

    assert_eq!(
        app.world().resource::<SamplingCoordinatorState>().active,
        ActiveUpscaler::None
    );
    let settings = app.world().resource::<ViewerSettingsState>().read_model();
    assert_eq!(settings.sampling.provider, SamplingProvider::None);
    assert!(!settings.capabilities.dlss_available);
    assert!(!settings.capabilities.fsr_available);
    assert_eq!(
        app.world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .take_published()
            .len(),
        1
    );
}

#[test]
fn newly_available_provider_does_not_replace_the_applied_provider() {
    let mut app = reconciliation_app(true, true, ActiveUpscaler::Fsr);
    {
        let mut settings = app.world_mut().resource_mut::<ViewerSettingsState>();
        settings.set_sampling_capabilities(false, true);
        settings.set_sampling(true, SamplingProvider::Fsr);
    }
    app.world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .take_published();

    app.update();

    assert_eq!(
        app.world().resource::<SamplingCoordinatorState>().active,
        ActiveUpscaler::Fsr
    );
    let settings = app.world().resource::<ViewerSettingsState>().read_model();
    assert_eq!(settings.sampling.provider, SamplingProvider::Fsr);
    assert_eq!(settings.capabilities.dlss_available, true);
    assert_eq!(settings.capabilities.fsr_available, true);

    let events = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .take_published();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].event,
        ViewportEvent::ViewerSettingsChanged { .. }
    ));

    app.update();
    assert!(
        app.world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .take_published()
            .is_empty()
    );
}
