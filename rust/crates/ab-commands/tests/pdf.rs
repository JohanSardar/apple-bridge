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

fn accepted(op: &str, args: &serde_json::Value) -> Result<bool, Box<dyn std::error::Error>> {
    Ok(validate(
        &Command {
            id: "document-regression".into(),
            op: op.into(),
            args: serde_json::value::to_raw_value(args)?,
            expected_version: 0,
            wait_ms: 15000,
        },
        "2026-11-03T00:00:00Z".parse()?,
    )
    .is_ok())
}

#[test]
fn annotate_takes_a_reference_or_bytes_but_never_a_path() -> Result<(), Box<dyn std::error::Error>>
{
    let annotations =
        json!([{"page":1,"type":"note","text":"synthetic","x":0,"y":0,"width":20,"height":20}]);
    for input in [
        json!({"file": "/tmp/synthetic.pdf", "allow_file": true}),
        json!({"file": "/tmp/synthetic.pdf"}),
        json!({"data_base64": "JVBERi0xLjQ=", "allow_file": true}),
    ] {
        let mut args = input.clone();
        args["annotations"] = annotations.clone();
        assert!(!accepted("pdf.annotate", &args)?, "{input}");
    }
    let mut blob = json!({"blob": {"query_id": "qry_x", "digest": "0".repeat(64)}});
    blob["annotations"] = annotations;
    assert!(accepted("pdf.annotate", &blob)?);
    Ok(())
}

#[test]
fn print_takes_a_reference_or_bytes_but_never_a_path() -> Result<(), Box<dyn std::error::Error>> {
    assert!(!accepted(
        "print.document",
        &json!({"file": "/tmp/synthetic.pdf", "printer": "PDF"})
    )?);
    assert!(!accepted(
        "print.document",
        &json!({"data_base64": "JVBERi0xLjQ=", "file": "/x", "printer": "PDF"})
    )?);
    assert!(!accepted(
        "print.document",
        &json!({"data_base64": "JVBERi0xLjQ=", "blob": {"query_id": "qry_x", "digest": "0".repeat(64)}, "printer": "PDF"})
    )?);
    assert!(!accepted("print.document", &json!({"printer": "PDF"}))?);
    assert!(accepted(
        "print.document",
        &json!({"data_base64": "JVBERi0xLjQ=", "printer": "PDF"})
    )?);
    assert!(accepted(
        "print.document",
        &json!({"blob": {"query_id": "qry_x", "digest": "0".repeat(64)}, "printer": "PDF"})
    )?);
    assert!(accepted(
        "print.document",
        &json!({"blob": {"collection": "pdf.artifact", "record_id": "x", "digest": "0".repeat(64)}, "printer": "PDF"})
    )?);
    Ok(())
}
