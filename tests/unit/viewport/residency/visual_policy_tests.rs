use super::*;

fn candidate(path: &str, pixels: Option<f64>) -> VisualCandidate {
    VisualCandidate {
        path: path.to_owned(),
        bound_known: pixels.is_some(),
        visible: true,
        selected: false,
        camera_mount: false,
        editing: false,
        projected_pixels: pixels,
        distance: 50.0,
        estimated_bytes: 100,
    }
}

#[test]
fn missing_bounds_must_load_and_never_evict() {
    let mut policy = VisualPolicy::default();
    let paths = vec!["/Unknown".to_owned()];
    let plan = policy.plan(
        VisualPolicyConfig::default(),
        &paths,
        &BTreeSet::new(),
        &[candidate("/Unknown", None)],
    );
    assert_eq!(plan.load, paths);
    assert!(plan.unload.is_empty());
    assert_eq!(plan.conservative, vec!["/Unknown"]);
}

#[test]
fn visual_unload_requires_hysteresis_and_never_overrides_pin() {
    let mut policy = VisualPolicy::default();
    let names = vec!["/A".to_owned()];
    let loaded = names.iter().cloned().collect::<BTreeSet<_>>();
    let cold = candidate("/A", Some(1.0));
    for _ in 0..3 {
        let plan = policy.plan(
            VisualPolicyConfig::default(),
            &names,
            &loaded,
            &[cold.clone()],
        );
        assert!(plan.unload.is_empty());
    }
    let plan = policy.plan(
        VisualPolicyConfig::default(),
        &names,
        &loaded,
        &[cold.clone()],
    );
    assert_eq!(plan.unload, names);
    let mut selected = cold;
    selected.selected = true;
    let plan = policy.plan(VisualPolicyConfig::default(), &names, &loaded, &[selected]);
    assert!(plan.unload.is_empty());
}

#[test]
fn nested_load_admits_ancestor_before_child() {
    let mut policy = VisualPolicy::default();
    let names = vec!["/A".to_owned(), "/A/B".to_owned()];
    let plan = policy.plan(
        VisualPolicyConfig::default(),
        &names,
        &BTreeSet::new(),
        &[candidate("/A", Some(1.0)), candidate("/A/B", Some(300.0))],
    );
    assert_eq!(plan.load, names);
}

#[test]
fn editor_intent_pins_unselected_payload_until_stable_and_is_stage_scoped() {
    use crate::viewport::rendering::residency::visual_composition::EditPayloadPins;
    use viewport_protocol::ViewportCommand;
    let mut pins = EditPayloadPins::default();
    let request = EditPayloadPins::command_intent(&ViewportCommand::RemovePrim {
        path: "/Scene/Equipment/Valve".to_owned(),
    })
    .expect("editor command has an intent");
    pins.record((1, 1), request);
    assert!(pins.editing("/Scene/Equipment", false));
    assert!(!pins.editing("/Other", true));
    let owners = BTreeMap::from([(
        "/Scene/Equipment".to_owned(),
        (None::<usd_model::Bounds3>, 1024),
    )]);
    pins.settle(&owners, &BTreeSet::new());
    assert!(pins.editing("/Scene/Equipment", false));
    let loaded = BTreeSet::from(["/Scene/Equipment".to_owned()]);
    pins.settle(&owners, &loaded);
    assert!(pins.editing("/Scene/Equipment", true));
    pins.settle(&owners, &loaded);
    assert!(!pins.editing("/Scene/Equipment", true));

    let request = EditPayloadPins::command_intent(&ViewportCommand::UndoEditor)
        .expect("undo conservatively protects loaded owners");
    pins.record((1, 1), request);
    assert!(pins.editing("/Scene/Equipment", true));
    assert!(!pins.editing("/Scene/Equipment", false));
    pins.record(
        (2, 1),
        EditPayloadPins::command_intent(&ViewportCommand::UndoEditor).unwrap(),
    );
    assert!(!pins.editing("/Scene/Equipment", false));
}

#[test]
fn failed_visual_recovery_retries_bounded_batches_until_native_readback() {
    use crate::viewport::rendering::residency::visual_composition::{
        VisualCompositionState, VisualStreamingHealth, restore_visual_owners,
    };
    use std::cell::{Cell, RefCell};

    let mut state = VisualCompositionState::default();
    let mut health = VisualStreamingHealth::default();
    let wanted: BTreeSet<_> = std::iter::once("/A".to_owned())
        .chain((0..11).map(|index| format!("/A/Child{index:02}")))
        .collect();
    let loaded = RefCell::new(Vec::new());
    let attempts = Cell::new(0);
    let mut tick = || {
        restore_visual_owners(
            &mut state,
            &mut health,
            &wanted,
            || Ok(loaded.borrow().clone()),
            |paths| {
                assert!(paths.len() <= 8);
                attempts.set(attempts.get() + 1);
                if attempts.get() == 1 {
                    assert_eq!(paths.first().map(String::as_str), Some("/A"));
                    anyhow::bail!("injected transient composition error");
                }
                loaded.borrow_mut().extend(paths.iter().cloned());
                Ok(())
            },
        )
    };
    assert!(!tick());
    assert!(!tick());
    assert!(!tick());
    assert_eq!(attempts.get(), 1);
    assert!(!tick());
    assert!(!tick());
    assert!(tick());
    assert_eq!(attempts.get(), 3);
    assert_eq!(
        loaded.into_inner().into_iter().collect::<BTreeSet<_>>(),
        wanted
    );
}
