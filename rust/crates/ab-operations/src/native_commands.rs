use crate::{Code, Error};
use serde_json::Value;

fn fields(args: &Value, allowed: &[&str]) -> bool {
    args.as_object()
        .is_some_and(|args| args.keys().all(|key| allowed.contains(&key.as_str())))
}

fn coordinate(point: &Value) -> bool {
    fields(point, &["lat", "lon"])
        && point["lat"].as_f64().is_some_and(|n| n.abs() <= 90.0)
        && point["lon"].as_f64().is_some_and(|n| n.abs() <= 180.0)
}

pub(crate) fn maps(args: &Value) -> Result<(), Error> {
    let route = !args["from"].is_null()
        || !args["to"].is_null()
        || args["mode"].as_str().is_some_and(|mode| !mode.is_empty());
    if !fields(args, &["from", "to", "mode", "lat", "lon"])
        || args
            .get("mode")
            .is_some_and(|mode| !mode.is_null() && !mode.is_string())
    {
        return Err(Code::Invalid.into());
    }
    let valid = if route {
        args["lat"].is_null()
            && args["lon"].is_null()
            && coordinate(&args["from"])
            && coordinate(&args["to"])
            && matches!(
                args["mode"].as_str(),
                Some("driving" | "walking" | "transit" | "cycling")
            )
    } else {
        coordinate(&serde_json::json!({"lat":args["lat"], "lon":args["lon"]}))
    };
    if valid {
        Ok(())
    } else {
        Err(Code::Invalid.into())
    }
}
