use ab_commands::{Command, validate};
use serde_json::json;

#[test]
fn annotations_require_one_input_and_bounded_geometry() -> Result<(), Box<dyn std::error::Error>> {
    let annotation =
        json!({"page":1,"type":"note","text":"synthetic","x":0,"y":0,"width":20,"height":20});
    let valid = json!({"data_base64":"JVBERi0xLjQ=","annotations":[annotation]});
    let now = "2026-11-03T00:00:00Z".parse()?;
    let check = |args: serde_json::Value| -> Result<bool, serde_json::Error> {
        Ok(validate(
            &Command {
                id: "pdf-regression".into(),
                op: "pdf.annotate".into(),
                args: serde_json::value::to_raw_value(&args)?,
                expected_version: 0,
                wait_ms: 15000,
            },
            now,
        )
        .is_ok())
    };
    assert!(check(valid.clone())?);
    let mut invalid = valid.clone();
    invalid["annotations"] = json!([]);
    assert!(!check(invalid)?);
    for field in ["width", "height"] {
        let mut invalid = valid.clone();
        invalid["annotations"][0][field] = json!(0);
        assert!(!check(invalid)?);
    }
    let mut invalid = valid.clone();
    invalid["file"] = json!("/tmp/synthetic.pdf");
    invalid["allow_file"] = json!(true);
    assert!(!check(invalid)?);
    let mut invalid = valid;
    invalid["annotations"][0]["page"] = json!(0);
    assert!(!check(invalid)?);
    Ok(())
}
