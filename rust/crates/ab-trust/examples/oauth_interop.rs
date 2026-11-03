use ab_trust::{Authority, Grant, oauth, passkeys};
use serde_json::{Value, json};
use std::io::Read;
fn run(v: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    let a = Authority::open(v["home"].as_str().ok_or("home")?)?;
    match v["op"].as_str().ok_or("op")? {
        "register" => Ok(serde_json::to_value(
            a.register_oauth(serde_json::from_value(v["client"].clone())?)?,
        )?),
        "issue" => {
            let (p, _) =
                a.create_agent("synthetic", "approve", &[Grant::parse("messages:write")?])?;
            let mut c: oauth::Code = serde_json::from_value(v["code"].clone())?;
            c.principal.clone_from(&p.id);
            Ok(json!({"code":a.issue_oauth_code(c)?,"principal":p.id}))
        }
        "exchange" => Ok(serde_json::to_value(
            a.exchange_oauth(&serde_json::from_value(v["exchange"].clone())?)?,
        )?),
        "authenticate" => Ok(serde_json::to_value(
            a.authenticate_token(v["token"].as_str().ok_or("token")?)?,
        )?),
        "golden" => {
            let f = &v["fixture"];
            let key = passkeys::Passkey {
                id: f["login_credential"]["id"].as_str().ok_or("id")?.into(),
                rp_id: "localhost".into(),
                credential: f["stored_credential"].clone(),
                created_at: "2026-01-01T00:00:00Z".into(),
            };
            let enroll = a.create_web_enroll()?;
            a.save_passkey(key.clone(), &enroll, None)?;
            let (_, mut state) =
                passkeys::begin("localhost", "http://localhost:7780", true, &[key])?;
            state["challenge"] = f["login_options"]["publicKey"]["challenge"].clone();
            a.finish_passkey_login(
                "localhost",
                "http://localhost:7780",
                state,
                f["login_credential"].clone(),
            )?;
            Ok(
                json!({"accepted":true,"counter":a.passkeys("localhost")?[0].credential["authenticator"]["signCount"]}),
            )
        }
        _ => Err("op".into()),
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let result = match run(&serde_json::from_str(&raw)?) {
        Ok(v) => v,
        Err(e) => json!({"error":e.to_string()}),
    };
    println!("{result}");
    Ok(())
}
