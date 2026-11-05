use ab_remote::{
    ct::{self, Certificate, CtClient, CtFuture, Notifications},
    state,
};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
    time::Duration,
};

type Check = Result<(), Box<dyn std::error::Error>>;

fn synthetic_report() -> Result<ct::Report, serde_json::Error> {
    serde_json::from_value(json!({
        "checked_at": 100,
        "names": ["mac.example.test"],
        "unknown": [],
        "unexpected": {"f00d": {
            "fingerprint": "f00d",
            "names": ["mac.example.test"],
            "precertificate": false
        }}
    }))
}

#[test]
fn an_unknown_certificate_reaches_doctor_without_a_local_opt_in() -> Check {
    let home = tempfile::tempdir()?;
    state::write(home.path(), "ct-monitor", &synthetic_report()?)?;
    state::write(
        home.path(),
        "ingresses",
        &BTreeMap::from([("relay".to_owned(), true)]),
    )?;
    let warnings = ct::doctor(home.path(), 100);
    assert!(
        warnings.iter().any(|warning| warning.contains("f00d")),
        "{warnings:?}"
    );
    Ok(())
}

fn certificate(fingerprint: &str, key: &str, precertificate: bool) -> Certificate {
    Certificate {
        fingerprint: fingerprint.into(),
        names: vec!["mac.example.test".into()],
        precertificate,
        public_key: key.into(),
    }
}

#[derive(Debug, Default)]
struct Log {
    certificates: Mutex<Vec<Certificate>>,
    outage: Mutex<bool>,
}
impl CtClient for Log {
    fn search<'a>(&'a self, query: &'a str) -> CtFuture<'a> {
        Box::pin(async move {
            if *self
                .outage
                .lock()
                .map_err(|_| ab_remote::Error::Contract("lock"))?
            {
                return Err(ab_remote::Error::Contract("outage"));
            }
            Ok(self
                .certificates
                .lock()
                .map_err(|_| ab_remote::Error::Contract("lock"))?
                .iter()
                .filter(|certificate| certificate.names.iter().any(|name| name == query))
                .cloned()
                .collect())
        })
    }
}

fn names() -> BTreeSet<String> {
    BTreeSet::from(["mac.example.test".to_owned()])
}

#[tokio::test]
async fn a_precertificate_for_this_macs_key_is_not_an_alarm_but_another_key_is() -> Check {
    let log = Log::default();
    log.certificates.lock().map_err(|_| "lock")?.extend([
        certificate("ours", "mac-key", true),
        certificate("rogue", "other-key", true),
    ]);
    let known = BTreeSet::from(["spki:mac-key".to_owned()]);
    let report = ct::check(&log, names(), &known, &ct::Report::default(), 100).await;
    assert!(!report.unexpected.contains_key("ours"));
    assert!(report.unexpected.contains_key("rogue"));
    Ok(())
}

#[tokio::test]
async fn a_finding_raised_before_this_mac_recorded_its_key_clears_once_it_does() -> Check {
    let log = Log::default();
    log.certificates
        .lock()
        .map_err(|_| "lock")?
        .push(certificate("ours", "mac-key", true));
    let first = ct::check(&log, names(), &BTreeSet::new(), &ct::Report::default(), 100).await;
    assert!(first.unexpected.contains_key("ours"));
    let known = BTreeSet::from(["spki:mac-key".to_owned()]);
    let second = ct::check(&log, names(), &known, &first, 200).await;
    assert!(second.unexpected.is_empty());
    Ok(())
}

async fn one_scan(home: &std::path::Path, log: &Log) {
    let notifications = Notifications {
        home: home.into(),
        status: None,
    };
    let _ = tokio::time::timeout(
        Duration::from_millis(100),
        ct::run(home, "mac.example.test", log, &notifications),
    )
    .await;
}

#[tokio::test]
async fn the_first_scan_is_a_baseline_and_later_certificates_raise_a_warning() -> Check {
    let home = tempfile::tempdir()?;
    let log = Log::default();
    log.certificates
        .lock()
        .map_err(|_| "lock")?
        .push(certificate("previous", "old-key", false));
    one_scan(home.path(), &log).await;
    let report: ct::Report = state::read(home.path(), "ct-monitor")?;
    assert_eq!(report.adopted, 1);
    assert_eq!(ct::owner_warning(home.path(), ct::now()), "");

    log.certificates
        .lock()
        .map_err(|_| "lock")?
        .push(certificate("rogue", "rogue-key", false));
    one_scan(home.path(), &log).await;
    let warning = ct::owner_warning(home.path(), ct::now());
    assert!(warning.contains("mac.example.test"), "{warning}");
    assert!(
        warning.contains("delayed detection, not prevention"),
        "{warning}"
    );
    Ok(())
}

#[tokio::test]
async fn a_short_aggregator_outage_stays_quiet_and_a_long_one_is_shown() -> Check {
    let home = tempfile::tempdir()?;
    state::write(
        home.path(),
        "ingresses",
        &BTreeMap::from([("relay".to_owned(), true)]),
    )?;
    let log = Log::default();
    *log.outage.lock().map_err(|_| "lock")? = true;
    one_scan(home.path(), &log).await;
    let mut report: ct::Report = state::read(home.path(), "ct-monitor")?;
    assert_eq!(ct::owner_warning(home.path(), report.checked_at), "");
    let now = report.checked_at + 90_000;
    report.checked_at = now;
    state::write(home.path(), "ct-monitor", &report)?;
    assert!(ct::owner_warning(home.path(), now).contains("over a day"));
    Ok(())
}

#[test]
fn the_owner_can_accept_a_certificate_they_recognise() -> Check {
    let home = tempfile::tempdir()?;
    let fingerprint = "ab".repeat(32);
    let report: ct::Report = serde_json::from_value(json!({
        "checked_at": 100,
        "names": ["mac.example.test"],
        "unknown": [],
        "unexpected": {fingerprint.clone(): {
            "fingerprint": fingerprint,
            "names": ["mac.example.test"],
            "precertificate": false
        }}
    }))?;
    state::write(home.path(), "ct-monitor", &report)?;
    state::write(
        home.path(),
        "status",
        &state::Status {
            ct_warning: "warning".into(),
            ..state::Status::default()
        },
    )?;
    assert!(ct::accept(home.path(), "not-a-fingerprint").is_err());
    ct::accept(home.path(), &fingerprint)?;
    assert_eq!(ct::owner_warning(home.path(), 100), "");
    assert_eq!(state::status(home.path())?.ct_warning, "");
    let known: BTreeSet<String> = state::read(home.path(), "ct-known")?;
    assert!(known.contains(&fingerprint));
    Ok(())
}

#[test]
fn the_recorded_key_matches_the_key_read_from_a_certificate() -> Check {
    let key = rcgen::KeyPair::generate()?;
    let certificate =
        rcgen::CertificateParams::new(vec!["mac.example.test".to_owned()])?.self_signed(&key)?;
    let parsed = Certificate::from_der(certificate.der())?;
    assert_eq!(
        ct::key_entry(&rcgen::PublicKeyData::subject_public_key_info(&key)),
        format!("spki:{}", parsed.public_key)
    );
    Ok(())
}

#[tokio::test]
async fn a_monitor_that_stopped_running_is_shown_while_remote_is_on() -> Check {
    let home = tempfile::tempdir()?;
    let log = Log::default();
    one_scan(home.path(), &log).await;
    let report: ct::Report = state::read(home.path(), "ct-monitor")?;
    let later = report.checked_at + 3 * 86_400;
    assert_eq!(ct::owner_warning(home.path(), later), "", "Remote is off");
    state::write(
        home.path(),
        "ingresses",
        &BTreeMap::from([("relay".to_owned(), true)]),
    )?;
    assert_eq!(ct::owner_warning(home.path(), report.checked_at), "");
    assert!(ct::owner_warning(home.path(), later).contains("over a day"));
    Ok(())
}

#[test]
fn a_monitor_that_never_reported_is_shown_after_a_day_of_remote_being_on() -> Check {
    let home = tempfile::tempdir()?;
    state::write(
        home.path(),
        "ingresses",
        &BTreeMap::from([("relay".to_owned(), true)]),
    )?;
    let now = ct::now();
    assert_eq!(ct::owner_warning(home.path(), now), "");
    assert_eq!(ct::doctor(home.path(), now).len(), 0);
    let later = now + 2 * 86_400;
    assert!(ct::owner_warning(home.path(), later).contains("over a day"));
    assert!(
        ct::doctor(home.path(), later)
            .iter()
            .any(|w| w.contains("over a day"))
    );
    state::write(
        home.path(),
        "ingresses",
        &BTreeMap::from([("relay".to_owned(), false)]),
    )?;
    assert_eq!(ct::owner_warning(home.path(), later + 2 * 86_400), "");
    Ok(())
}
