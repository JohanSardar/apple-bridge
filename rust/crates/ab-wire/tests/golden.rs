use ab_wire::gojson::{CommandHash, EscapeMode, GoTime, record_data};
use serde::Deserialize;

#[derive(Deserialize)]
struct Golden {
    vectors: Vec<Vector>,
    rfc3339nano: Vec<String>,
}
#[derive(Deserialize)]
struct Vector {
    input: String,
    record_data: String,
    query: String,
    command_preimage: String,
    command_hash: String,
}

#[test]
fn frozen_json_paths() -> Result<(), Box<dyn std::error::Error>> {
    let golden: Golden =
        serde_json::from_str(include_str!("../../../../contracts/golden/json.json"))?;
    for vector in golden.vectors {
        assert_eq!(
            record_data(vector.input.as_bytes(), EscapeMode::Html)?,
            vector.record_data.as_bytes()
        );
        assert_eq!(
            record_data(vector.input.as_bytes(), EscapeMode::NoHtml)?,
            vector.query.as_bytes()
        );
        let command = CommandHash {
            op: "fixture.op",
            args: vector.input.as_bytes(),
            version: 7,
            expires: GoTime::parse("2026-01-02T03:04:05.123456789Z")?,
            era: "era-fixture",
        };
        assert_eq!(command.encode()?, vector.command_preimage.as_bytes());
        assert_eq!(command.digest()?, vector.command_hash);
    }
    for stamp in golden.rfc3339nano {
        let (_, fraction) = stamp.split_once("03:04:05").ok_or("golden timestamp")?;
        let fraction = fraction.strip_suffix("+05:30").ok_or("golden offset")?;
        assert_eq!(
            GoTime::parse(&stamp)?.as_str(),
            format!("2026-01-01T21:34:05{fraction}Z")
        );
    }
    Ok(())
}

#[derive(Deserialize)]
struct Frames {
    frames: Vec<Frame>,
}
#[derive(Deserialize)]
struct Frame {
    json: String,
}
#[derive(Deserialize)]
struct Capture {
    #[serde(default)]
    observations: Vec<Observation>,
}
#[derive(Deserialize)]
struct Observation {
    data: Box<serde_json::value::RawValue>,
}

#[test]
fn frozen_capture_data() -> Result<(), Box<dyn std::error::Error>> {
    let frames: Frames =
        serde_json::from_str(include_str!("../../../../contracts/golden/ipc.json"))?;
    let mut observations = 0;
    for frame in frames.frames {
        let capture: Capture = serde_json::from_str(&frame.json)?;
        for observation in capture.observations {
            let raw = observation.data.get().as_bytes();
            assert_eq!(record_data(raw, EscapeMode::Html)?, raw);
            observations += 1;
        }
    }
    assert_eq!(observations, 1);
    Ok(())
}
