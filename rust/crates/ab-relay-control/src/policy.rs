use crate::{Control, Error, Result, auth::text, decode, hash};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::HeaderMap,
    routing::post,
};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone)]
struct Policy {
    control: Arc<Control>,
    token_hash: String,
}
impl std::fmt::Debug for Policy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Policy([redacted])")
    }
}
pub fn policy_router(control: Arc<Control>, token: &str) -> Result<Router> {
    if decode(token)?.len() < 32 {
        return Err(Error::Invalid);
    }
    Ok(Router::new()
        .route("/v1/policy/{action}", post(action))
        .layer(DefaultBodyLimit::max(4096))
        .with_state(Policy {
            control,
            token_hash: hash(token),
        }))
}
async fn action(
    State(policy): State<Policy>,
    Path(action): Path<String>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    let secret = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(Error::Unauthorized)?;
    if hash(secret) != policy.token_hash {
        return Err(Error::Unauthorized);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::Unavailable)?
        .as_secs()
        .try_into()
        .map_err(|_| Error::Unavailable)?;
    let c = &policy.control;
    let response = match action.as_str() {
        "admit" => c.admit(
            text(&input, "device")?,
            text(&input, "credential")?,
            input["generation"].as_i64().ok_or(Error::Invalid)?,
            now,
        )?,
        "charge" => {
            c.charge(
                text(&input, "lease")?,
                input["upstream"].as_u64().ok_or(Error::Invalid)?,
                input["downstream"].as_u64().ok_or(Error::Invalid)?,
                now,
            )?;
            json!({"allowed":true})
        }
        "release" => {
            c.release(text(&input, "lease")?)?;
            json!({"released":true})
        }
        "disconnects" => c.pending_disconnects()?,
        "ack" => {
            c.ack_disconnect(input["id"].as_i64().ok_or(Error::Invalid)?)?;
            json!({"acknowledged":true})
        }
        _ => return Err(Error::Invalid),
    };
    Ok(Json(response))
}
