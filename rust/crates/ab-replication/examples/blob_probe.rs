use ab_blobs::{Digest, Error, RecordGrant, Store};
use ab_trust::domain::Record;
use ab_trust::{Grant, Principal};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::args().nth(1).ok_or("temporary home required")?;
    let mut store = Store::open(&home, 10)?;
    let (d, pin) = store.stage(b"fixture")?;
    let visibility = Principal {
        grants: vec![Grant::parse("messages:read")?],
        ..Principal::default()
    }
    .visibility();
    let record: Record = serde_json::from_value(
        serde_json::json!({"collection":"messages.attachment","id":"att_01ARZ3NDEKTSV4RRFFQ69G5FAV","version":1,"source":"messages","observed_at":"2026-01-02T00:00:00Z","deleted":false,"coverage":{"fields":"","missing":null},"data":{"blob":{"digest":d.as_str(),"size":7}}}),
    )?;
    let grant = RecordGrant::from_record(&visibility, &record)?;
    if store.open_authorized(&grant, &d)?.metadata()?.len() != 7 {
        return Err("blob length".into());
    }
    if RecordGrant::from_record(&ab_trust::Visibility::default(), &record).is_ok() {
        return Err("grant leak".into());
    }
    let over = Digest::try_from(ab_trust::fingerprint(b"over"))?;
    if !matches!(store.begin(&over, 20), Err(Error::Capacity)) {
        return Err("cap".into());
    }
    let now = jiff::Timestamp::now();
    store.protect([d.as_str().into()].into_iter().collect())?;
    let owner_refs = [(
        "gateway:fixture".into(),
        vec![ab_blobs::Reference {
            digest: d.as_str().into(),
            size: 7,
            ..ab_blobs::Reference::default()
        }],
    )]
    .into_iter()
    .collect();
    store.update_protection(&owner_refs, true)?;
    store.release_pin(&pin)?;
    store.gc(now, 0)?;
    store.open_authorized(&grant, &d)?;
    store.protect(std::collections::BTreeSet::default())?;
    store.gc(now, 0)?;
    store.open_authorized(&grant, &d)?;
    drop(store);
    let mut store = Store::open(&home, 10)?;
    store.gc(now, 0)?;
    store.open_authorized(&grant, &d)?;
    store.update_protection(
        &[("gateway:fixture".into(), Vec::new())]
            .into_iter()
            .collect(),
        false,
    )?;
    let later = now.checked_add(std::time::Duration::from_hours(25))?;
    if store.gc(later, 24 * 3600)? < 1 {
        return Err("GC".into());
    }
    println!("{{\"impl\":\"rust\",\"record_grants\":true,\"over_cap\":true,\"gc\":true}}");
    Ok(())
}
