#![allow(clippy::unwrap_used, clippy::too_many_lines)]

use ab_relay_control::caa::{
    Binding, CaaRecord, Change, Control, DnsAdapter, Error, Operation, Result,
};
use ed25519_dalek::{Signer, SigningKey};
use std::collections::BTreeMap;

#[derive(Debug, Default)]
struct Dns {
    bindings: BTreeMap<String, Binding>,
    records: BTreeMap<String, Vec<CaaRecord>>,
    cnames: BTreeMap<String, String>,
    fail: bool,
}
impl DnsAdapter for Dns {
    fn binding(&self, host: &str) -> Result<Option<Binding>> {
        Ok(self.bindings.get(host).cloned())
    }
    fn commit(
        &mut self,
        previous: Option<&Binding>,
        next: &Binding,
        records: &[CaaRecord],
    ) -> Result<()> {
        if self.fail
            || self.bindings.get(&next.host) != previous
            || self.cnames.contains_key(&next.host)
        {
            return Err(Error::Dns("conflict or outage".into()));
        }
        self.records.insert(next.host.clone(), records.to_vec());
        self.bindings.insert(next.host.clone(), next.clone());
        Ok(())
    }
}
struct AcmeTestCa;
impl AcmeTestCa {
    fn issue(dns: &Dns, name: &str, account: &str, method: &str, issuer: &str) -> bool {
        let wildcard = name.starts_with("*.");
        let mut host = name.trim_start_matches("*.").to_owned();
        for _ in 0..10 {
            if let Some(target) = dns.cnames.get(&host) {
                host.clone_from(target);
                continue;
            }
            if let Some(records) = dns.records.get(&host) {
                let tag = if wildcard && records.iter().any(|r| r.tag == "issuewild") {
                    "issuewild"
                } else {
                    "issue"
                };
                return records.iter().filter(|r| r.tag == tag).any(|r| {
                    let mut fields = r.value.split(';').map(str::trim);
                    fields.next() == Some(issuer)
                        && fields.any(|v| v.strip_prefix("accounturi=") == Some(account))
                        && r.value.split(';').map(str::trim).any(|v| {
                            v.strip_prefix("validationmethods=")
                                .is_some_and(|v| v.split(',').any(|m| m == method))
                        })
                });
            }
            if let Some((_, parent)) = host.split_once('.') {
                host = parent.into();
            } else {
                return false;
            }
        }
        false
    }
}
fn apply(
    control: &mut Control<Dns>,
    change: &Change,
    key: &SigningKey,
    enrolled: Option<[u8; 32]>,
) -> Result<()> {
    control.apply(
        change,
        &key.sign(&change.transcript()?).to_bytes(),
        enrolled,
        100,
    )
}
#[test]
fn relay_caa_account() {
    let mac = SigningKey::from_bytes(&[1; 32]);
    let recovery = SigningKey::from_bytes(&[2; 32]);
    let attacker = SigningKey::from_bytes(&[3; 32]);
    let mut control = Control::new(Dns::default(), true);
    let mut change = Change {
        operation: Operation::Enroll,
        expires: 200,
        binding: Binding {
            host: "mac.example.test".into(),
            account_uri: "https://ca.test/acct/mac".into(),
            mac_key: mac.verifying_key().to_bytes(),
            recovery_key: recovery.verifying_key().to_bytes(),
            revision: 0,
        },
    };
    assert!(
        apply(
            &mut control,
            &change,
            &attacker,
            Some(mac.verifying_key().to_bytes())
        )
        .is_err()
    );
    apply(
        &mut control,
        &change,
        &mac,
        Some(mac.verifying_key().to_bytes()),
    )
    .unwrap();
    println!("R2_ASSERT\tenrollment_caa_records");
    let dns = control.adapter();
    assert_eq!(dns.records[&change.binding.host], change.binding.records());
    let issue =
        |name, account, method, issuer| AcmeTestCa::issue(dns, name, account, method, issuer);
    assert!(issue(
        "mac.example.test",
        "https://ca.test/acct/mac",
        "dns-01",
        "letsencrypt.org"
    ));
    assert!(!issue(
        "mac.example.test",
        "https://ca.test/acct/attacker",
        "dns-01",
        "letsencrypt.org"
    ));
    assert!(!issue(
        "mac.example.test",
        "https://ca.test/acct/mac",
        "http-01",
        "letsencrypt.org"
    ));
    assert!(!issue(
        "mac.example.test",
        "https://ca.test/acct/mac",
        "tls-alpn-01",
        "letsencrypt.org"
    ));
    assert!(!issue(
        "mac.example.test",
        "https://ca.test/acct/mac",
        "dns-01",
        "other-ca.test"
    ));
    assert!(!issue(
        "*.mac.example.test",
        "https://ca.test/acct/mac",
        "dns-01",
        "letsencrypt.org"
    ));
    println!("R2_ASSERT\twrong_account_denied");
    println!("R2_ASSERT\twrong_method_denied");
    println!("R2_ASSERT\talternate_issuer_denied");
    println!("R2_ASSERT\twildcard_denied");
    assert!(apply(&mut control, &change, &mac, None).is_err());
    change.operation = Operation::Rotate;
    change.binding.revision = 1;
    change.binding.account_uri = "https://ca.test/acct/new".into();
    change.binding.mac_key = attacker.verifying_key().to_bytes();
    assert!(apply(&mut control, &change, &attacker, None).is_err());
    apply(&mut control, &change, &mac, None).unwrap();
    assert!(apply(&mut control, &change, &mac, None).is_err());
    change.operation = Operation::Recover;
    change.binding.revision = 2;
    change.binding.mac_key = mac.verifying_key().to_bytes();
    assert!(apply(&mut control, &change, &attacker, None).is_err());
    apply(&mut control, &change, &recovery, None).unwrap();
    println!("R2_ASSERT\tsigned_rotation_recovery");
    change.binding.revision = 3;
    change.expires = 99;
    assert!(apply(&mut control, &change, &recovery, None).is_err());
    change.expires = 200;
    change.binding.account_uri = "https://ca.test/acct/x;validationmethods=http-01".into();
    assert!(apply(&mut control, &change, &recovery, None).is_err());
    let mut dns = Dns::default();
    dns.cnames
        .insert("alias.example.test".into(), "target.example.test".into());
    let mut target = change.binding.clone();
    target.host = "target.example.test".into();
    target.account_uri = "https://ca.test/acct/mac".into();
    dns.records
        .insert(target.host.clone(), target.records().into());
    assert!(!AcmeTestCa::issue(
        &dns,
        "alias.example.test",
        "https://ca.test/acct/attacker",
        "dns-01",
        "letsencrypt.org"
    ));
    assert!(AcmeTestCa::issue(
        &dns,
        "alias.example.test",
        "https://ca.test/acct/mac",
        "dns-01",
        "letsencrypt.org"
    ));
    println!("R2_ASSERT\tcname_target_policy");
    target.account_uri = "https://ca.test/acct/attacker".into();
    dns.records
        .insert(target.host.clone(), target.records().into());
    assert!(AcmeTestCa::issue(
        &dns,
        "alias.example.test",
        "https://ca.test/acct/attacker",
        "dns-01",
        "letsencrypt.org"
    ));
    println!("R2_ASSERT\tdns_operator_residual_risk");
    let mut failure = Dns {
        fail: true,
        ..Dns::default()
    };
    let previous = control.adapter().bindings["mac.example.test"].clone();
    failure
        .bindings
        .insert(previous.host.clone(), previous.clone());
    let mut failed_control = Control::new(failure, true);
    change.binding = previous.clone();
    change.binding.revision += 1;
    change.operation = Operation::Rotate;
    assert!(apply(&mut failed_control, &change, &mac, None).is_err());
    assert_eq!(
        failed_control.adapter().bindings["mac.example.test"],
        previous
    );
    println!("R2_ASSERT\tdns_failure_preserves_binding");
    let mut disabled = Control::new(Dns::default(), false);
    assert!(matches!(
        apply(&mut disabled, &change, &mac, None),
        Err(Error::Disabled)
    ));
}
