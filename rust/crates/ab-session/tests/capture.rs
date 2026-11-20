#[path = "capture/capture_handle_budget.rs"]
mod capture_handle_budget;
#[path = "capture/capture_pacing.rs"]
mod capture_pacing;
#[path = "capture/capture_producer.rs"]
mod capture_producer;
#[path = "capture/files_facts.rs"]
mod files_facts;
#[allow(dead_code, clippy::unwrap_used)]
#[path = "../../ab-native/tests/support/voicemail.rs"]
mod fixture;
#[path = "capture/framework_sources.rs"]
mod framework_sources;
#[path = "capture/messages_live_first.rs"]
mod messages_live_first;
#[path = "capture/messages_media.rs"]
mod messages_media;
#[path = "capture/messages_policy_vectors.rs"]
mod messages_policy_vectors;
#[path = "capture/notes_session.rs"]
mod notes_session;
#[path = "capture/session_capture.rs"]
mod session_capture;
#[path = "capture/staging_confinement.rs"]
mod staging_confinement;
mod support;
#[path = "capture/voicemail_limits.rs"]
mod voicemail_limits;
#[path = "capture/voicemail_scan_budget.rs"]
mod voicemail_scan_budget;
