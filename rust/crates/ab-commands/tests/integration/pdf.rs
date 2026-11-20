use ab_commands::{Command, validate};
use serde_json::json;

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
fn annotate_requires_one_document_input_and_bounded_geometry()
-> Result<(), Box<dyn std::error::Error>> {
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
    let valid = json!({"data_base64":"JVBERi0xLjQ=","annotations":annotations});
    assert!(accepted("pdf.annotate", &valid)?);
    let mut empty = valid.clone();
    empty["annotations"] = json!([]);
    assert!(!accepted("pdf.annotate", &empty)?);
    for field in ["page", "width", "height"] {
        let mut invalid = valid.clone();
        invalid["annotations"][0][field] = json!(0);
        assert!(!accepted("pdf.annotate", &invalid)?, "{field}");
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

#[test]
fn malformed_annotations_identify_the_field_and_expected_range()
-> Result<(), Box<dyn std::error::Error>> {
    let request: Command = serde_json::from_value(json!({
        "command_id":"pdf-validation",
        "op":"pdf.annotate",
        "args":{"data_base64":"JVBERi0xLjQ=","annotations":[
            {"page":1,"type":"note","text":"synthetic","x":0,"y":0,"width":0,"height":20}
        ]}
    }))?;
    let Err(problem) = ab_commands::validate_request(&request, "2026-11-03T00:00:00Z".parse()?)
    else {
        return Err("the annotation must be refused".into());
    };
    assert_eq!(problem.code, "invalid");
    assert_eq!(problem.field, "args.annotations[0].width");
    assert_eq!(problem.expected, "greater than 0 and at most 100000");
    assert_ne!(problem.hint, "");
    assert!(!problem.retryable);
    Ok(())
}
