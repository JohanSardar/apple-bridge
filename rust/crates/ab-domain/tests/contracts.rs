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
