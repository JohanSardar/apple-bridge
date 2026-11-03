use ab_domain::{Collection, CommandState, ContractError, ErrorCode, MessagesMessageId};

#[test]
fn ids_validate_prefix_alphabet_and_ulid_overflow() {
    assert!(
        "msg_01ARZ3NDEKTSV4RRFFQ69G5FAV"
            .parse::<MessagesMessageId>()
            .is_ok()
    );
    for invalid in [
        "con_01ARZ3NDEKTSV4RRFFQ69G5FAV",
        "msg_81ARZ3NDEKTSV4RRFFQ69G5FAV",
        "msg_01ARZ3NDEKTSV4RRFFQ69G5FAI",
        "msg_fixture",
    ] {
        assert!(invalid.parse::<MessagesMessageId>().is_err());
        assert!(serde_json::from_str::<MessagesMessageId>(&format!("\"{invalid}\"")).is_err());
    }
    assert_eq!(Collection::ALL.len(), 46);
}

#[test]
fn command_lifecycle_rejects_effects_after_rejection() {
    assert_eq!(
        CommandState::Prepared.transition(CommandState::Dispatched),
        Ok(CommandState::Dispatched)
    );
    assert_eq!(
        CommandState::Rejected.transition(CommandState::Dispatched),
        Err(ErrorCode::Conflict)
    );
    let error = ContractError::from(ErrorCode::Unavailable);
    assert_eq!(error.message, "peer unavailable");
    assert!(error.retryable);
}

#[test]
fn collection_prefixes_equal_go_registry() -> Result<(), Box<dyn std::error::Error>> {
    let expected: std::collections::BTreeMap<String, String> = serde_json::from_str(include_str!(
        "../../../../contracts/golden/collections.json"
    ))?;
    let mut actual = std::collections::BTreeMap::new();
    for collection in Collection::ALL {
        let name = serde_json::to_value(collection)?
            .as_str()
            .ok_or("collection")?
            .to_owned();
        actual.insert(name, collection.prefix().to_owned());
    }
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn record_data_retains_legacy_number_tokens() -> Result<(), Box<dyn std::error::Error>> {
    let record: ab_domain::Record = serde_json::from_str(
        r#"{"collection":"messages.message","id":"msg_01ARZ3NDEKTSV4RRFFQ69G5FAV","version":1,"source":"messages","observed_at":"2026-01-02T03:04:05Z","deleted":false,"coverage":{"fields":"","missing":null},"data":{"n":123456789012345678901234567890,"one":1.0,"exp":1e0}}"#,
    )?;
    record.validate()?;
    assert_eq!(
        record.data.get(),
        r#"{"n":123456789012345678901234567890,"one":1.0,"exp":1e0}"#
    );
    let encoded = serde_json::to_string(&record)?;
    assert!(encoded.contains(record.data.get()));
    Ok(())
}

#[test]
fn command_records_accept_caller_chosen_ids_and_command_sources()
-> Result<(), Box<dyn std::error::Error>> {
    let record: ab_domain::Record = serde_json::from_str(
        r#"{"collection":"system.command","id":"cmd_caller-chosen:1","version":1,"source":"reminders","observed_at":"2026-01-02T03:04:05Z","deleted":false,"coverage":{"fields":"complete","missing":[]},"data":{"state":"admitted"}}"#,
    )?;
    record.validate()?;
    assert_eq!(record.id.as_str(), "cmd_caller-chosen:1");
    assert!(
        serde_json::from_str::<ab_domain::Record>(
            &serde_json::to_string(&record)?.replace("system.command", "reminders.reminder")
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn coverage_without_fields_matches_go_decoding() -> Result<(), Box<dyn std::error::Error>> {
    let coverage: ab_domain::Coverage =
        serde_json::from_str(r#"{"max_depth":16,"roots":1,"max_items":10000}"#)?;
    assert_eq!(
        serde_json::to_string(&coverage)?,
        r#"{"fields":"","missing":null}"#
    );
    Ok(())
}
