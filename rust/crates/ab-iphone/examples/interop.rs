use ab_trust::{Authority, iphone};
use serde_json::{Value, json};
use std::io::Read;

fn run(input: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    let home = input["home"].as_str().ok_or("home")?;
    let a = Authority::open(home)?;
    match input["op"].as_str().ok_or("op")? {
        "code" => Ok(
            json!({"code": a.pair_iphone_code("https://fixture.example", false, std::time::Duration::from_secs(60))?}),
        ),
        "pair" => Ok(serde_json::to_value(a.pair_iphone(
            &serde_json::from_value::<iphone::PairRequest>(input["request"].clone())?,
        )?)?),
        "verify" => {
            let request = serde_json::from_value::<iphone::Request>(input["request"].clone())?;
            let (p, payload) = a.verify_iphone(input["path"].as_str().ok_or("path")?, &request)?;
            Ok(json!({"id":p.id,"payload":String::from_utf8(payload)?}))
        }
        "seal" => {
            let p = a.principal(input["id"].as_str().ok_or("id")?)?;
            Ok(
                json!({"packet":a.seal_iphone(&p, b"synthetic-command")?,"gateway_key":a.iphone_gateway_key()?}),
            )
        }
        "push-json" => {
            let request = ab_iphone::PushRequest {
                hostname: "fixture.example".into(),
                token: "synthetic-token".into(),
                packet: serde_json::from_value(input["packet"].clone())?,
                expires: 1_800_000_000,
            };
            Ok(
                json!({"raw":String::from_utf8(ab_wire::gojson::legacy_json(&request, ab_wire::gojson::EscapeMode::Html)?)?}),
            )
        }
        "tokens" => {
            let id = input["id"].as_str().ok_or("id")?;
            a.set_iphone_push_token(id, &"a".repeat(64))?;
            a.set_iphone_live_token(id, "_start", &"b".repeat(64))?;
            Ok(serde_json::to_value(a.principal(id)?)?)
        }
        _ => Err("op".into()),
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let input = serde_json::from_str(&raw)?;
    let output = match run(&input) {
        Ok(v) => v,
        Err(e) => json!({"error":e.to_string()}),
    };
    println!("{output}");
    Ok(())
}
