use warp_core::features::FeatureFlag;
use warpui::App;

use super::WarpDriveSettings;
use crate::auth::AuthStateProvider;
use crate::test_util::settings::initialize_settings_for_tests;

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
}

#[test]
fn warp_drive_is_available_by_default() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.read(|ctx| {
            assert!(WarpDriveSettings::is_warp_drive_available(ctx));
            assert!(WarpDriveSettings::is_warp_drive_enabled(ctx));
        });
    });
}

#[test]
fn warp_drive_is_never_available_when_disabled_by_flag() {
    let _disable_drive = FeatureFlag::DisableWarpDrive.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.read(|ctx| {
            assert!(!WarpDriveSettings::is_warp_drive_available(ctx));
            assert!(
                !WarpDriveSettings::is_warp_drive_enabled(ctx),
                "the stored enable_warp_drive preference must not override the flag"
            );
        });
    });
}
