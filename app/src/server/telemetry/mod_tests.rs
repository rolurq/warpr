use rudder_message::Track;
use virtual_fs::VirtualFS;
use warp_core::AppId;
use warp_core::channel::{
    Channel, ChannelConfig, OzConfig, RudderStackConfig, TelemetryConfig, WarpServerConfig,
};
use warp_core::telemetry::EnablementState;

use super::*;

// Tests that events with UGC are not persisted to desk.
#[test]
fn test_persist_events_doesnt_include_ugc_events() {
    let telemetry_api = TelemetryApi::new();

    VirtualFS::test(
        "test_persist_events_doesnt_include_ugc_events",
        |dirs, _sandbox| {
            // Add one event without UGC
            let user_id = Some("user".into());
            let anonymous_id = "anonymous_id".to_owned();

            warpui::telemetry::record_event(
                user_id.clone(),
                anonymous_id.clone(),
                "non UGC event name".into(),
                None,  /* payload */
                false, /* contains_ugc  */
                warpui::time::get_current_time(),
            );

            warpui::telemetry::record_event(
                user_id.clone(),
                anonymous_id.clone(),
                "UGC event name".into(),
                None, /* payload */
                true, /* contains_ugc  */
                warpui::time::get_current_time(),
            );

            let file_path = dirs.root().join("rudderstack");

            telemetry_api
                .flush_and_persist_events_at_path(10, PrivacySettingsSnapshot::mock(), &file_path)
                .expect("Should be able to persist events");

            let file_content: Vec<RudderBatchMessage> =
                serde_json::from_reader(File::open(file_path).expect("Failed to open file"))
                    .expect("Failed to parse file");

            assert_eq!(file_content.len(), 1);

            let track = file_content[0].unwrap_track();
            assert_eq!(track.event, "non UGC event name");
        },
    );
}

impl RudderBatchMessage {
    fn unwrap_track(&self) -> &Track {
        match self {
            RudderBatchMessage::Track(track) => track,
            _ => panic!("Expected a track event"),
        }
    }
}

/// Points the Rudderstack destination at `url` so a request that escapes the telemetry sink is
/// observable by a local mock server.
fn point_rudderstack_at(url: &str) {
    ChannelState::set(ChannelState::new(
        Channel::Oss,
        ChannelConfig {
            app_id: AppId::new("dev", "warp", "WarpOss"),
            logfile_name: "".into(),
            server_config: WarpServerConfig::production(),
            oz_config: OzConfig::production(),
            telemetry_config: Some(TelemetryConfig {
                telemetry_file_name: "telemetry.json".into(),
                rudderstack_config: Some(RudderStackConfig {
                    write_key: "write-key".into(),
                    root_url: url.to_owned().into(),
                    ugc_write_key: "ugc-write-key".into(),
                }),
            }),
            crash_reporting_config: None,
            autoupdate_config: None,
            mcp_static_config: None,
        },
    ));
}

fn queue_event(name: &'static str) {
    warpui::telemetry::record_event(
        Some("user".into()),
        "anonymous_id".into(),
        name.into(),
        None,  /* payload */
        false, /* contains_ugc  */
        warpui::time::get_current_time(),
    );
}

/// Rudderstack requests need a Tokio reactor, which `futures::executor::block_on` does not provide.
fn block_on_tokio<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Runtime::new().unwrap().block_on(fut)
}

#[test]
fn flushed_events_reach_rudderstack_when_usage_telemetry_is_not_disabled() {
    let mut server = mockito::Server::new();
    point_rudderstack_at(&server.url());
    let batch = server
        .mock("POST", "/v1/batch")
        .with_status(200)
        .expect(1)
        .create();
    let _sandbox_telemetry = FeatureFlag::WithSandboxTelemetry.override_enabled(true);

    queue_event("event");
    block_on_tokio(TelemetryApi::new().flush_events(PrivacySettingsSnapshot::mock()))
        .expect("Flush should succeed");

    batch.assert();
}

#[test]
fn no_request_reaches_rudderstack_when_usage_telemetry_is_disabled() {
    let mut server = mockito::Server::new();
    point_rudderstack_at(&server.url());
    let any_request = server
        .mock("POST", mockito::Matcher::Any)
        .with_status(200)
        .expect(0)
        .create();
    let _sandbox_telemetry = FeatureFlag::WithSandboxTelemetry.override_enabled(true);
    let _disable_usage_telemetry = FeatureFlag::DisableUsageTelemetry.override_enabled(true);
    let telemetry_api = TelemetryApi::new();

    VirtualFS::test(
        "no_request_reaches_rudderstack_when_usage_telemetry_is_disabled",
        |dirs, _sandbox| {
            let persisted_path = dirs.root().join("rudderstack");
            let persisted_event = warpui::telemetry::create_event(
                Some("user".into()),
                "anonymous_id".into(),
                "persisted event".into(),
                None,
                false,
                warpui::time::get_current_time(),
            )
            .to_rudder_batch_message()
            .message;
            serde_json::to_writer(
                File::create(&persisted_path).expect("Should create file"),
                &vec![persisted_event],
            )
            .expect("Should write persisted events");

            queue_event("queued event");
            block_on_tokio(async {
                telemetry_api
                    .flush_events(PrivacySettingsSnapshot::mock())
                    .await
                    .expect("Flushing the queue should be a no-op");
                telemetry_api
                    .flush_persisted_events_to_rudder(
                        &persisted_path,
                        PrivacySettingsSnapshot::mock(),
                    )
                    .await
                    .expect("Flushing persisted events should be a no-op");
                telemetry_api
                    .send_telemetry_event(
                        None,
                        "anonymous_id".into(),
                        TelemetryEvent::Login,
                        PrivacySettingsSnapshot::mock(),
                    )
                    .await
                    .expect("Sending a single event should be a no-op");
            });

            queue_event("event to persist");
            let unwritten_path = dirs.root().join("unwritten");
            telemetry_api
                .flush_and_persist_events_at_path(
                    10,
                    PrivacySettingsSnapshot::mock(),
                    &unwritten_path,
                )
                .expect("Persisting should be a no-op");
            assert!(!unwritten_path.exists());

            // The event queue is process-global; don't leak this test's event into others.
            let _ = warpui::telemetry::flush_events();
        },
    );

    any_request.assert();
}

#[test]
fn usage_telemetry_kill_switch_disables_every_event_enablement_state() {
    let _disable_usage_telemetry = FeatureFlag::DisableUsageTelemetry.override_enabled(true);

    assert!(!EnablementState::Always.is_enabled());
    assert!(!EnablementState::Flag(FeatureFlag::WithSandboxTelemetry).is_enabled());
    assert!(!TelemetryEvent::Login.enablement_state().is_enabled());
    assert!(!ChannelState::is_telemetry_available());
    assert!(!ChannelState::is_crash_reporting_available());
}
