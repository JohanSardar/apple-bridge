use super::{
    Duration, Instant, Result, Session, Shared, Ticket, Value, id, json, lock, now, send, string,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const MODES: [&str; 4] = ["driving", "walking", "transit", "cycling"];
const SNAPSHOT_KEYS: [&str; 5] = ["center", "span", "width", "height", "markers"];

fn failure(id: &Value, code: &str) -> Value {
    json!({"t":"QueryError","id":id,"code":code})
}

fn point(value: &Value) -> Option<(f64, f64)> {
    let map = value.as_object()?;
    if map.keys().any(|key| key != "lat" && key != "lon") {
        return None;
    }
    let lat = map.get("lat")?.as_f64()?;
    let lon = map.get("lon")?.as_f64()?;
    (lat.abs() <= 90.0 && lon.abs() <= 180.0).then_some((lat, lon))
}

fn text(value: &Value, limit: usize) -> bool {
    value
        .as_str()
        .is_some_and(|text| !text.is_empty() && text.len() <= limit)
}

fn allowed(op: &str) -> Option<&'static [&'static str]> {
    Some(match op {
        "location.geocode" => &["address"],
        "location.reverse_geocode" => &["lat", "lon"],
        "maps.search" => &["query", "near", "radius"],
        "maps.nearby" => &["category", "near", "radius"],
        "maps.directions" | "maps.eta" => &["from", "to", "mode"],
        _ => return None,
    })
}

fn simulate(op: &str, args: &Value) -> std::result::Result<Value, &'static str> {
    let map = args.as_object().ok_or("invalid")?;
    if op == "location.current" {
        if !map.is_empty() {
            return Err("invalid");
        }
        return Ok(
            json!({"lat":37.7955,"lon":-122.3937,"accuracy":5,"timestamp":now(),"source":"mac"}),
        );
    }
    let keys = allowed(op).ok_or("unsupported")?;
    if map.is_empty() || map.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err("invalid");
    }
    match op {
        "location.geocode" => {
            if !text(&args["address"], 1024) {
                return Err("invalid");
            }
        }
        "location.reverse_geocode" => {
            if point(args).is_none() {
                return Err("invalid");
            }
        }
        "maps.search" | "maps.nearby" => {
            let key = if op == "maps.nearby" {
                "category"
            } else {
                "query"
            };
            if !text(&args[key], 512) || point(&args["near"]).is_none() {
                return Err("invalid");
            }
            if let Some(radius) = map.get("radius")
                && !radius
                    .as_f64()
                    .is_some_and(|radius| radius > 0.0 && radius <= 100_000.0)
            {
                return Err("invalid");
            }
        }
        _ => {
            if point(&args["from"]).is_none() || point(&args["to"]).is_none() {
                return Err("invalid");
            }
            let mode = args["mode"].as_str().unwrap_or_default();
            if mode == "transit" && op == "maps.directions" {
                return Err("unsupported");
            }
            if !MODES.contains(&mode) {
                return Err("invalid");
            }
            if op == "maps.eta" {
                return Ok(json!({"distance":1800,"duration":600_000}));
            }
            return Ok(json!({"routes":[{
                "distance":1800,"duration":600_000,
                "steps":[{"instructions":"Continue","distance":1800}],
                "polyline":[{"lat":37.7955,"lon":-122.3937},{"lat":37.8087,"lon":-122.4098}]
            }]}));
        }
    }
    Ok(json!({"places":[{
        "name":"Public fixture","address":"San Francisco Ferry Building",
        "lat":37.7955,"lon":-122.3937,"category":"cafe","phone":"","url":""
    }]}))
}

fn blank_png(width: u32, height: u32) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: [u8; 4], body: &[u8]) {
        out.extend_from_slice(&u32::try_from(body.len()).unwrap_or_default().to_be_bytes());
        let mut checked = kind.to_vec();
        checked.extend_from_slice(body);
        out.extend_from_slice(&checked);
        out.extend_from_slice(&crc32fast::hash(&checked).to_be_bytes());
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, *b"IHDR", &header);
    let row = vec![0u8; width as usize * 4 + 1];
    let mut raw = Vec::with_capacity(row.len() * height as usize);
    for _ in 0..height {
        raw.extend_from_slice(&row);
    }
    chunk(
        &mut out,
        *b"IDAT",
        &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6),
    );
    chunk(&mut out, *b"IEND", &[]);
    out
}

fn snapshot_size(args: &Value, key: &str) -> Option<u32> {
    let number = args[key].as_f64()?;
    if number.fract() != 0.0 || !(128.0..=1024.0).contains(&number) {
        return None;
    }
    format!("{number}").parse().ok()
}

fn snapshot(blobs: &mut BTreeMap<String, Vec<u8>>, id: &Value, args: &Value) -> Value {
    let Some(map) = args.as_object() else {
        return failure(id, "invalid");
    };
    if map.keys().any(|key| !SNAPSHOT_KEYS.contains(&key.as_str())) {
        return failure(id, "invalid");
    }
    let span = args["span"].as_f64().unwrap_or_default();
    let centered = point(&args["center"]).is_some_and(|(lat, _)| lat.abs() + span / 2.0 <= 90.0);
    let markers = match args.get("markers") {
        None | Some(Value::Null) => Some(0),
        Some(Value::Array(markers)) => markers
            .iter()
            .all(|marker| point(marker).is_some())
            .then_some(markers.len()),
        Some(_) => None,
    };
    let (Some(width), Some(height)) = (snapshot_size(args, "width"), snapshot_size(args, "height"))
    else {
        return failure(id, "invalid");
    };
    if !centered || span <= 0.0 || span > 180.0 || markers.is_none_or(|count| count > 50) {
        return failure(id, "invalid");
    }
    let data = blank_png(width, height);
    let digest = hex::encode(Sha256::digest(&data));
    let size = data.len();
    blobs.insert(digest.clone(), data);
    json!({"t":"QueryResult","id":id,
        "result_json":{"attribution":"© Apple Maps","image":{"digest":digest,"size":size}},
        "blobs":[{"digest":digest,"size":size,"handle":digest}]})
}

pub(super) fn query(blobs: &mut BTreeMap<String, Vec<u8>>, frame: &Value) -> Option<Value> {
    let op = frame["op"].as_str()?;
    if !op.starts_with("location.") && !op.starts_with("maps.") {
        return None;
    }
    let id = &frame["id"];
    let args = frame.get("args").unwrap_or(&Value::Null);
    if op == "maps.snapshot" {
        return Some(snapshot(blobs, id, args));
    }
    Some(match simulate(op, args) {
        Ok(result) => json!({"t":"QueryResult","id":id,"result_json":result,"blobs":[]}),
        Err(code) => failure(id, code),
    })
}

fn open_valid(args: &Value) -> bool {
    let Some(map) = args.as_object() else {
        return false;
    };
    if map
        .keys()
        .any(|key| !["lat", "lon", "from", "to", "mode"].contains(&key.as_str()))
    {
        return false;
    }
    if ["from", "to", "mode"]
        .iter()
        .any(|key| map.contains_key(*key))
    {
        return !map.contains_key("lat")
            && !map.contains_key("lon")
            && point(&args["from"]).is_some()
            && point(&args["to"]).is_some()
            && args["mode"]
                .as_str()
                .is_some_and(|mode| MODES.contains(&mode));
    }
    point(args).is_some()
}

impl Session<'_> {
    pub(super) fn maps_prepare(&mut self, _: &Shared, frame: &Value) -> Result<()> {
        let action = &frame["action"];
        if string(action, "op") != "maps.open" {
            return self.reject(frame, "unsupported");
        }
        if !open_valid(&action["args"]) {
            return self.reject(frame, "invalid");
        }
        let ticket = id("ticket")?;
        self.tickets.insert(
            ticket.clone(),
            Ticket {
                prepare: frame.clone(),
                before: Value::Null,
                deadline: Instant::now() + Duration::from_secs(30),
            },
        );
        send(
            &mut self.stream,
            &json!({"t":"Prepared","command_id":frame["command_id"],"ticket":ticket,"route":"maps","fingerprint":frame["request_hash"]}),
        )?;
        Ok(())
    }

    pub(super) fn maps_dispatch(&mut self, state: &Shared, frame: &Value) -> Result<()> {
        let command = string(frame, "command_id");
        let Some(ticket) = self.tickets.remove(&string(frame, "ticket")) else {
            return self.reject(frame, "expired");
        };
        if ticket.prepare["command_id"] != command
            || frame["owner_fence"].as_u64() != Some(self.fence)
            || Instant::now() >= ticket.deadline
        {
            return self.reject(frame, "expired");
        }
        let mut s = lock(state)?;
        s.guard(&json!({"command_id":command,"ticket":frame["ticket"],"started_at":now()}))?;
        let count = s.data["effects"][&command].as_u64().unwrap_or_default() + 1;
        s.data["effects"][&command] = json!(count);
        s.save()?;
        let outcome = json!({"t":"Outcome","command_id":command,"disposition":"locally_observed",
            "evidence":{"frontmost_bundle":"com.apple.Maps","prepared":{"request_hash":ticket.prepare["request_hash"]}}});
        s.guard(&json!({"command_id":command,"outcome":outcome}))?;
        drop(s);
        send(&mut self.stream, &outcome)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(op: &str, args: &Value) -> Value {
        let mut blobs = BTreeMap::new();
        query(&mut blobs, &json!({"id":"q","op":op,"args":args})).unwrap_or(Value::Null)
    }

    #[test]
    fn current_rejects_unknown_fields() {
        assert_eq!(call("location.current", &json!({}))["t"], "QueryResult");
        assert_eq!(
            call("location.current", &json!({"unexpected":true}))["code"],
            "invalid"
        );
    }

    #[test]
    fn directions_modes() {
        let route = |mode: &str| {
            call(
                "maps.directions",
                &json!({"from":{"lat":37.7,"lon":-122.3},"to":{"lat":37.8,"lon":-122.4},"mode":mode}),
            )
        };
        assert_eq!(route("walking")["t"], "QueryResult");
        assert_eq!(route("transit")["code"], "unsupported");
        assert_eq!(route("flying")["code"], "invalid");
    }

    #[test]
    fn snapshot_validates_and_stores_png() {
        let mut blobs = BTreeMap::new();
        let frame = json!({"id":"q","op":"maps.snapshot","args":{"center":{"lat":37.7,"lon":-122.3},"span":0.02,"width":128,"height":128}});
        let reply = query(&mut blobs, &frame).unwrap_or(Value::Null);
        assert_eq!(reply["t"], "QueryResult");
        assert_eq!(blobs.len(), 1);
        let oversized = json!({"id":"q","op":"maps.snapshot","args":{"center":{"lat":37.7,"lon":-122.3},"span":0.02,"width":2048,"height":512}});
        assert_eq!(
            query(&mut blobs, &oversized).unwrap_or(Value::Null)["code"],
            "invalid"
        );
    }

    #[test]
    fn open_requires_complete_route() {
        assert!(open_valid(&json!({"lat":37.7,"lon":-122.3})));
        assert!(open_valid(
            &json!({"from":{"lat":1,"lon":2},"to":{"lat":3,"lon":4},"mode":"walking"})
        ));
        assert!(!open_valid(
            &json!({"from":{"lat":1,"lon":2},"mode":"walking"})
        ));
    }
}
