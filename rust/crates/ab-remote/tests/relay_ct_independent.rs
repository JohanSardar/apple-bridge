#![allow(clippy::unwrap_used, clippy::too_many_lines)]

use ab_remote::{
    ct::{self, Certificate, CtClient, CtFuture, IphonePush, Notifications, Report},
    state,
};
use std::collections::BTreeSet;

#[derive(Debug)]
struct Fixture {
    outage: bool,
}
impl CtClient for Fixture {
    fn search<'a>(&'a self, query: &'a str) -> CtFuture<'a> {
        Box::pin(async move {
            if self.outage {
                return Err(ab_remote::Error::Contract("aggregator outage"));
            }
            Ok(vec![
                Certificate {
                    fingerprint: "known".into(),
                    names: vec!["mac.example.test".into()],
                    precertificate: false,
                    public_key: "known-key".into(),
                },
                Certificate {
                    fingerprint: "wildcard".into(),
                    names: vec!["*.example.test".into()],
                    precertificate: false,
                    public_key: "wildcard-key".into(),
                },
                Certificate {
                    fingerprint: "precert".into(),
                    names: vec!["custom.other.test".into()],
                    precertificate: true,
                    public_key: "precert-key".into(),
                },
                Certificate {
                    fingerprint: "unrelated".into(),
                    names: vec!["badexample.test".into()],
                    precertificate: false,
                    public_key: "unrelated-key".into(),
                },
            ]
            .into_iter()
            .filter(|certificate| {
                certificate.names.iter().any(|name| name == query)
                    || certificate.fingerprint == "unrelated"
            })
            .collect())
        })
    }
}
#[tokio::test]
async fn relay_ct_independent() {
    let names = BTreeSet::from(["mac.example.test".into(), "custom.other.test".into()]);
    let known = BTreeSet::from(["known".into()]);
    let relay_all_clear = Report::default();
    let report = ct::check(
        &Fixture { outage: false },
        names.clone(),
        &known,
        &relay_all_clear,
        100,
    )
    .await;
    assert_eq!(report.unexpected.len(), 2);
    assert!(report.unexpected.contains_key("wildcard"));
    println!("R2_ASSERT\tfake_all_clear_ignored");
    println!("R2_ASSERT\tunexpected_wildcard");
    assert!(report.unexpected["precert"].precertificate);
    println!("R2_ASSERT\tunexpected_precertificate");
    assert!(!report.unexpected.contains_key("known"));
    assert!(!report.unexpected.contains_key("unrelated"));
    assert!(report.note.contains("lag"));
    println!("R2_ASSERT\taggregator_lag_unknown");
    let outage = ct::check(&Fixture { outage: true }, names, &known, &report, 200).await;
    assert_eq!(outage.unexpected.len(), 2);
    assert!(outage.unknown.iter().any(|s| s.contains("unavailable")));
    println!("R2_ASSERT\taggregator_outage_unknown");
    assert!(outage.warnings(7500).iter().any(|s| s.contains("stale")));
    assert!(outage.warnings(100).iter().any(|s| s.contains("stale")));
    println!("R2_ASSERT\tstale_monitor_unknown");
    let home = tempfile::tempdir().unwrap();
    state::write(
        home.path(),
        "ingresses",
        &std::collections::BTreeMap::from([("relay".to_owned(), true)]),
    )
    .unwrap();
    assert_eq!(ct::doctor(home.path(), 100).len(), 0);
    state::write(home.path(), "ct-monitor", &outage).unwrap();
    assert!(
        ct::doctor(home.path(), 7500)
            .iter()
            .any(|s| s.contains("stale"))
    );
    Notifications {
        home: home.path().into(),
        status: None,
    }
    .alert(&outage)
    .await
    .unwrap();
    let alert: Report = state::read(home.path(), "ct-alert").unwrap();
    assert_eq!(alert.unexpected.len(), 2);
    println!("R2_ASSERT\tdoctor_and_push_hook");
    ct::remember(home.path(), "locally-issued").unwrap();
    let known: BTreeSet<String> = state::read(home.path(), "ct-known").unwrap();
    assert!(known.contains("locally-issued"));
    state::write(
        home.path(),
        "config",
        &state::Config {
            domain: "custom.other.test".into(),
            ..state::Config::default()
        },
    )
    .unwrap();
    state::write(home.path(), "ct-names", &BTreeSet::from(["*.extra.test"])).unwrap();
    let notifications = Notifications {
        home: home.path().into(),
        status: None,
    };
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            ct::run(
                home.path(),
                "mac.example.test",
                &Fixture { outage: false },
                &notifications
            )
        )
        .await
        .is_err()
    );
    let running: Report = state::read(home.path(), "ct-monitor").unwrap();
    assert!(running.names.contains("mac.example.test"));
    assert!(running.names.contains("custom.other.test"));
    assert!(running.names.contains("*.extra.test"));
    println!("R2_ASSERT\tall_owned_names");
    assert_eq!(running.adopted, running_adopted(&running));
    assert!(
        running.unexpected.is_empty(),
        "the first scan is a baseline"
    );
    state::write(
        home.path(),
        "ct-monitor",
        &Report {
            unexpected: outage.unexpected.clone(),
            ..running
        },
    )
    .unwrap();
    state::write(
        home.path(),
        "ingresses",
        &std::collections::BTreeMap::from([("relay".to_owned(), false)]),
    )
    .unwrap();
    assert!(
        ct::doctor(home.path(), 100)
            .iter()
            .all(|warning| warning.contains("unexpected"))
    );
}
fn running_adopted(report: &Report) -> usize {
    report.adopted
}

#[test]
fn ct_der_fixture() {
    let mut params = rcgen::CertificateParams::new(vec!["*.example.test".into()]).unwrap();
    let mut poison =
        rcgen::CustomExtension::from_oid_content(&[1, 3, 6, 1, 4, 1, 11129, 2, 4, 3], vec![5, 0]);
    poison.set_criticality(true);
    params.custom_extensions.push(poison);
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    let observed = Certificate::from_der(cert.der()).unwrap();
    assert!(observed.precertificate);
    assert_eq!(observed.names, vec!["*.example.test"]);
    assert_eq!(observed.fingerprint.len(), 64);
    assert!(Certificate::from_der(b"invalid DER").is_err());
    println!("R2_ASSERT\tprecertificate_der");
}
