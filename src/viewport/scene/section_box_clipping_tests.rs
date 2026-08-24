use super::*;

#[test]
fn clip_uniform_uses_one_shared_box_space_transform() {
    let transform =
        Transform::from_translation(Vec3::new(2.0, 3.0, 4.0)).with_scale(Vec3::splat(6.0));
    let uniform = SectionClipUniform {
        world_to_box: transform.to_matrix().inverse(),
        enabled: 1,
        _padding: Vec3::ZERO,
    };

    assert_eq!(uniform.enabled, 1);
    assert!(
        uniform
            .world_to_box
            .transform_point3(transform.translation)
            .abs_diff_eq(Vec3::ZERO, 0.0001)
    );
}

#[test]
fn extension_keeps_the_standard_material_as_the_base_route() {
    let base = StandardMaterial {
        base_color: Color::srgb(0.2, 0.4, 0.8),
        perceptual_roughness: 0.35,
        ..default()
    };
    let extended = ExtendedMaterial {
        base: base.clone(),
        extension: SectionClipExtension::default(),
    };

    assert_eq!(extended.base.base_color, base.base_color);
    assert_eq!(
        extended.base.perceptual_roughness,
        base.perceptual_roughness
    );
}

#[test]
fn composition_preserves_uniform_and_selection_routes() {
    let mut materials = Assets::<StandardMaterial>::default();
    let authored = materials.add(StandardMaterial::default());
    let uniform = materials.add(StandardMaterial::default());
    let selection = materials.add(StandardMaterial::default());
    let original = OriginalRenderMaterial(authored.clone());
    let selection_base = SelectionBaseMaterial(uniform.clone());

    let composed = support::compose_clipped_route(
        selection.clone(),
        Some(&original),
        Some(&selection_base),
        true,
        true,
        Some(&selection),
        true,
        false,
        None,
        false,
        RenderMode::UniformColor,
        Some(&uniform),
    );

    assert_eq!(composed.route, selection);
    assert_eq!(composed.original, Some(authored));
    assert_eq!(composed.selection_base, Some(uniform));
    assert!(composed.selection_override);
}

#[test]
fn composition_restores_authored_route_after_shaded_selection_release() {
    let mut materials = Assets::<StandardMaterial>::default();
    let authored = materials.add(StandardMaterial::default());
    let selection = materials.add(StandardMaterial::default());
    let original = OriginalRenderMaterial(authored.clone());
    let selection_base = SelectionBaseMaterial(authored.clone());

    let composed = support::compose_clipped_route(
        selection,
        Some(&original),
        Some(&selection_base),
        true,
        false,
        None,
        false,
        false,
        None,
        false,
        RenderMode::Shaded,
        None,
    );

    assert_eq!(composed.route, authored);
    assert_eq!(composed.original, None);
    assert_eq!(composed.selection_base, None);
    assert!(!composed.selection_override);
}
