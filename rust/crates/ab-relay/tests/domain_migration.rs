#![allow(clippy::unwrap_used)]

use ab_relay::store::{Device, Limits, Store};
use std::collections::BTreeMap;

#[test]
fn legacy_devices_reopen_only_with_explicit_suffix_and_keep_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let limits = Limits {
        connections: 2,
        bytes_per_second: 1024,
        monthly_bytes: 4096,
    };
    let host = "device.old.example";
    let device = Device {
        hostname: host.into(),
        public_key: "retained-key".into(),
        credential_hash: "retained-hash".into(),
        limits,
        ..Device::default()
    };
    let bytes = serde_json::to_vec(&BTreeMap::from([(host, device)])).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    assert!(Store::open(path.clone(), "new.example", limits).is_err());
    assert!(
        Store::open_with_legacy_zones(
            path.clone(),
            "new.example",
            &["example.invalid".into()],
            limits
        )
        .is_err()
    );
    let store =
        Store::open_with_legacy_zones(path.clone(), "new.example", &["old.example".into()], limits)
            .unwrap();
    assert_eq!(store.zone, "new.example");
    assert_eq!(store.devices[host].credential_hash, "retained-hash");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(
        Store::open_with_legacy_zones(path, "new.example", &["*.example".into()], limits).is_err()
    );
}
