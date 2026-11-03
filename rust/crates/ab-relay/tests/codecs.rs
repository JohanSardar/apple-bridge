#![allow(clippy::unwrap_used)]

use ab_relay::{
    hello::{client_hello, hello_name},
    store::Device,
};

fn hello(host: &str) -> Vec<u8> {
    let mut body = vec![0; 34];
    body[0] = 3;
    body[1] = 3;
    body.extend_from_slice(&[0, 0, 2, 0x13, 1, 1, 0]);
    let n = u16::try_from(host.len()).unwrap();
    let mut extension = vec![0, 0];
    extension.extend_from_slice(&(n + 5).to_be_bytes());
    extension.extend_from_slice(&(n + 3).to_be_bytes());
    extension.push(0);
    extension.extend_from_slice(&n.to_be_bytes());
    extension.extend_from_slice(host.as_bytes());
    body.extend_from_slice(&u16::try_from(extension.len()).unwrap().to_be_bytes());
    body.extend(extension);
    let size = u32::try_from(body.len()).unwrap().to_be_bytes();
    let mut result = vec![1, size[1], size[2], size[3]];
    result.extend(body);
    result
}
#[tokio::test]
async fn go_fragmented_client_hello_vectors() {
    let hello = hello("device.relay.test");
    for split in 1..hello.len() {
        let mut raw = Vec::new();
        for part in [&hello[..split], &hello[split..]] {
            raw.extend_from_slice(&[22, 3, 1]);
            raw.extend_from_slice(&u16::try_from(part.len()).unwrap().to_be_bytes());
            raw.extend_from_slice(part);
        }
        let (name, forwarded) = client_hello(&mut raw.as_slice()).await.unwrap();
        assert_eq!(name, "device.relay.test");
        assert_eq!(forwarded, raw);
    }
}
#[tokio::test]
async fn go_malformed_client_hello_vectors() {
    for raw in [
        vec![],
        vec![23, 3, 3, 0, 1, 0],
        vec![22, 3, 3, 255, 255],
        hello("invalid_host.relay.test"),
    ] {
        assert!(client_hello(&mut raw.as_slice()).await.is_err());
    }
    let hello = hello("a.relay.test");
    for length in 0..hello.len() - 4 {
        assert!(hello_name(&hello[4..4 + length]).is_err());
    }
}
#[test]
fn go_state_field_order_and_omissions() {
    let go = r#"{"public_key":"fixture","hostname":"device.relay.test","credential_hash":"hash","revoked":false,"disabled":true,"limits":{"connections":32,"bytes_per_second":1048576,"monthly_bytes":10737418240},"month":"2026-11","bytes":42}"#;
    let device: Device = serde_json::from_str(go).unwrap();
    assert_eq!(serde_json::to_string(&device).unwrap(), go);
    let go = r#"{"certificates":["abc"],"domain":"mac.example.test","public_key":"fixture","hostname":"device.relay.test","credential_hash":"hash","revoked":true,"disabled":false,"limits":{"connections":2,"bytes_per_second":99,"monthly_bytes":123},"month":"2026-10","bytes":100}"#;
    let device: Device = serde_json::from_str(go).unwrap();
    assert_eq!(serde_json::to_string(&device).unwrap(), go);
}

#[test]
fn go_credential_response_field_order() {
    let wire = r#"{"hostname":"device.relay.test","credential":"fixture-token"}"#;
    let credential: ab_relay::store::Credential = serde_json::from_str(wire).unwrap();
    assert_eq!(serde_json::to_string(&credential).unwrap(), wire);
}
#[test]
fn utc_month_boundaries() {
    for (instant, month) in [
        ("2026-10-31T23:59:59.999999999Z", "2026-10"),
        ("2026-11-01T00:00:00Z", "2026-11"),
        ("2026-10-31T19:00:00-05:00", "2026-11"),
        ("2028-02-29T23:59:59Z", "2028-02"),
        ("2028-03-01T00:00:00Z", "2028-03"),
    ] {
        assert_eq!(ab_relay::store::month_at(instant.parse().unwrap()), month);
    }
}
