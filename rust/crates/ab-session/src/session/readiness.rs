use crate::readers::Readiness;
use serde_json::{Value, json};
use std::collections::BTreeMap;

const REMINDERS: [&str; 8] = [
    "reminders.create",
    "reminders.update",
    "reminders.complete",
    "reminders.uncomplete",
    "reminders.delete",
    "reminders.create_list",
    "reminders.rename_list",
    "reminders.delete_list",
];
const REMINDERS_UNPROVEN: [&str; 11] = [
    "reminders.set_flag",
    "reminders.indent",
    "reminders.outdent",
    "reminders.create_section",
    "reminders.move_to_section",
    "reminders.set_tags",
    "reminders.set_list_emblem",
    "reminders.set_list_group",
    "reminders.reorder",
    "reminders.add_image",
    "reminders.create_smart_list",
];
const CALENDAR: [&str; 4] = [
    "calendar.create_event",
    "calendar.update_event",
    "calendar.delete_event",
    "calendar.create_calendar",
];
const CALENDAR_UNPROVEN: [(&str, &str); 4] = [
    (
        "calendar.respond_invitation",
        "calendar_accessibility_invitation_response_not_proven_on_this_os_build",
    ),
    (
        "calendar.set_travel_time",
        "calendar_accessibility_travel_time_write_and_positive_units_not_proven_on_this_os_build",
    ),
    (
        "calendar.add_attachment",
        "calendar_accessibility_attachment_add_not_proven_on_this_os_build",
    ),
    (
        "calendar.remove_attachment",
        "calendar_accessibility_attachment_remove_not_proven_on_this_os_build",
    ),
];
const CONTACTS: [&str; 3] = ["contacts.create", "contacts.update", "contacts.delete"];
const CONTACT_GROUPS: [&str; 5] = [
    "contacts.create_group",
    "contacts.rename_group",
    "contacts.delete_group",
    "contacts.add_member",
    "contacts.remove_member",
];
const AUTOMATION: &str = "Automation dev.applebridge.helper → com.apple.Safari";
const CAPTURE: [&str; 6] = [
    "capture.screenshot",
    "capture.camera_photo",
    "capture.record_audio",
    "mac.screen_record",
    "print.document",
    "capture.play_sound",
];

fn permission(
    authorized: bool,
    coverage: Value,
    capabilities: Vec<String>,
    reason: &str,
) -> Readiness {
    Readiness {
        state: if authorized {
            "ready"
        } else {
            "permission_missing"
        },
        coverage,
        capabilities: if authorized { capabilities } else { Vec::new() },
        reason: if authorized {
            String::new()
        } else {
            reason.to_owned()
        },
    }
}

fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn reminders(status: &Value, calendar: Option<&Value>, os_build: &str) -> Readiness {
    let calendar_refused = calendar.is_some_and(|calendar| calendar["authorized"] == false);
    let unproven: Vec<Value> = REMINDERS_UNPROVEN
        .iter()
        .map(|operation| {
            json!({
                "operation": operation,
                "proven": false,
                "reason": "Native Reminders parity effect and capture are not verified on this OS build",
                "os_version": os_build,
            })
        })
        .collect();
    permission(
        status["authorized"] == true && !calendar_refused,
        json!({
            "fields": "partial",
            "missing": ["recurrence: next incomplete instance only"],
            "operations": unproven,
        }),
        owned(&REMINDERS),
        if status["authorized"] == true {
            "reminders_need_calendar_access"
        } else {
            "reminders_full_access_required"
        },
    )
}

fn calendar(status: &Value) -> Readiness {
    let unproven: Vec<Value> = CALENDAR_UNPROVEN
        .iter()
        .map(|(operation, reason)| json!({"operation": operation, "proven": false, "reason": reason}))
        .collect();
    permission(
        status["authorized"] == true,
        json!({
            "operations": unproven,
            "fields": "partial",
            "missing": [
                "travel_time: positive native units unverified",
                "structured_location: coordinate units unverified",
                "detached store identities",
                "calendar attachment derivatives",
                "start location, proposals, comments and extra store alarms: positive native fixtures pending",
            ],
        }),
        owned(&CALENDAR),
        "calendar_full_access_required",
    )
}

fn contacts(status: &Value) -> Readiness {
    let proven: Vec<&str> = status["proven_groups"]
        .as_array()
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let groups: Vec<&str> = CONTACT_GROUPS
        .iter()
        .copied()
        .filter(|operation| proven.contains(operation))
        .collect();
    let operations: Vec<Value> = CONTACT_GROUPS
        .iter()
        .map(|operation| {
            let done = groups.contains(operation);
            json!({
                "operation": operation,
                "proven": done,
                "reason": if done { "proven_public_contacts_save_request" } else { "os_build_group_proof_required" },
            })
        })
        .collect();
    permission(
        status["authorized"] == true,
        json!({
            "fields": "partial",
            "missing": [
                "note: CNContactNoteKey requires com.apple.developer.contacts.notes; Helper lacks this restricted entitlement",
                "writability: conservatively limited to default container",
                "contact_poster: unavailable through public Contacts API on this SDK",
            ],
            "operations": operations,
        }),
        CONTACTS
            .iter()
            .chain(&groups)
            .map(|name| (*name).to_owned())
            .collect(),
        "contacts_full_access_required",
    )
}

fn focus(status: &Value) -> Readiness {
    let authorized = status["authorization"] == "authorized";
    Readiness {
        state: "ready",
        coverage: json!({
            "fields": "partial",
            "missing": ["named Focus identity and restoration unavailable"],
        }),
        capabilities: if authorized {
            owned(&["focus.set"])
        } else {
            Vec::new()
        },
        reason: if authorized {
            String::new()
        } else {
            "needs Focus status permission".to_owned()
        },
    }
}

fn location(status: &Value) -> Readiness {
    let name = status["authorization"].as_str().unwrap_or_default();
    let reason = match name {
        "authorized_always" => "",
        "not_determined" | "denied" | "restricted" => name,
        _ => "unavailable",
    };
    Readiness {
        state: if reason.is_empty() {
            "ready"
        } else {
            "permission_missing"
        },
        coverage: json!({"fields": "complete"}),
        capabilities: Vec::new(),
        reason: reason.to_owned(),
    }
}

fn vision(status: &Value) -> Readiness {
    match status["languages"].as_u64() {
        Some(count) if count > 0 => {
            Readiness::ready(json!({"fields": "complete", "language_count": count}), &[])
        }
        _ => Readiness::not(
            "unavailable",
            json!({"fields": "unsupported"}),
            "recognition_languages_unavailable",
        ),
    }
}

fn safari(status: &Value) -> Readiness {
    if status["directory"] != true {
        return Readiness::not(
            "unavailable",
            json!({"fields": "partial"}),
            "Safari data directory unavailable",
        );
    }
    let permitted = status["automation"] == true;
    let mut missing = vec!["CloudTabs schema availability varies"];
    if !permitted {
        missing.push(AUTOMATION);
    }
    let coverage = json!({"fields": "partial", "missing": missing});
    if status["readable"] != true {
        return Readiness::not("permission_missing", coverage, "Full Disk Access required");
    }
    Readiness {
        state: "ready",
        coverage,
        capabilities: Vec::new(),
        reason: if permitted {
            String::new()
        } else {
            AUTOMATION.to_owned()
        },
    }
}

pub(super) fn framework(status: &Value, os_build: &str) -> BTreeMap<String, Readiness> {
    let mut told = BTreeMap::new();
    let Some(sources) = status.as_object() else {
        return told;
    };
    for (source, fields) in sources {
        let readiness = match source.as_str() {
            "reminders" => reminders(fields, sources.get("calendar"), os_build),
            "calendar" => calendar(fields),
            "contacts" => contacts(fields),
            "focus" => focus(fields),
            "location" => location(fields),
            "photos" => {
                if fields["authorized"] == true {
                    Readiness::ready(
                        json!({"fields":"live_only","limited":fields["limited"] == true}),
                        &[],
                    )
                } else {
                    Readiness::not(
                        "permission_missing",
                        json!({"fields":"live_only"}),
                        "Photos permission is optional; run bridge permissions guide",
                    )
                }
            }
            "vision" => vision(fields),
            "safari" => safari(fields),
            "capture" => Readiness::ready(
                json!({"fields": "partial", "permissions": fields["permissions"].as_object().cloned().unwrap_or_default()}),
                &CAPTURE,
            ),
            "maps" => Readiness::ready(json!({"fields": "complete"}), &["maps.open"]),
            "speech" => Readiness::ready(json!({"fields": "complete"}), &["speech.say"]),
            "pdf" => Readiness::ready(json!({"fields": "complete"}), &["pdf.annotate"]),
            _ => continue,
        };
        told.insert(source.clone(), readiness);
    }
    told
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn reminders_without_calendar_says_it_cannot_read() {
        let told = framework(
            &json!({"reminders": {"authorized": true}, "calendar": {"authorized": false}}),
            "b",
        );
        assert_eq!(told["reminders"].state, "permission_missing");
        assert_eq!(told["reminders"].reason, "reminders_need_calendar_access");
        assert_eq!(told["reminders"].capabilities, Vec::<String>::new());
        let both = framework(
            &json!({"reminders": {"authorized": true}, "calendar": {"authorized": true}}),
            "b",
        );
        assert_eq!(both["reminders"].state, "ready");
    }

    #[test]
    fn without_the_grant_a_source_offers_nothing_and_says_why() {
        let told = framework(
            &json!({"reminders": {"authorized": false}, "calendar": {"authorized": false}, "contacts": {"authorized": false}}),
            "b",
        );
        for (source, reason) in [
            ("reminders", "reminders_full_access_required"),
            ("calendar", "calendar_full_access_required"),
            ("contacts", "contacts_full_access_required"),
        ] {
            let readiness = &told[source];
            assert_eq!(readiness.state, "permission_missing");
            assert_eq!(readiness.capabilities, Vec::<String>::new());
            assert_eq!(readiness.reason, reason);
        }
    }

    #[test]
    fn a_contact_group_operation_needs_its_proof() {
        let told = framework(
            &json!({"contacts": {"authorized": true, "proven_groups": ["contacts.create_group", "made.up"]}}),
            "b",
        );
        let readiness = &told["contacts"];
        assert!(
            readiness
                .capabilities
                .contains(&"contacts.create_group".to_owned())
        );
        assert!(
            !readiness
                .capabilities
                .contains(&"contacts.delete_group".to_owned())
        );
        assert!(!readiness.capabilities.contains(&"made.up".to_owned()));
        assert_eq!(readiness.coverage["operations"][0]["proven"], true);
        assert_eq!(readiness.coverage["operations"][1]["proven"], false);
    }

    #[test]
    fn focus_set_needs_authorization_but_status_stays_readable() {
        let denied = &framework(&json!({"focus": {"authorization": "denied"}}), "b")["focus"];
        assert_eq!(denied.state, "ready");
        assert_eq!(denied.capabilities, Vec::<String>::new());
        let allowed = &framework(&json!({"focus": {"authorization": "authorized"}}), "b")["focus"];
        assert_eq!(allowed.capabilities, ["focus.set"]);
    }

    #[test]
    fn location_is_ready_only_when_always_authorized() {
        for (name, state, reason) in [
            ("authorized_always", "ready", ""),
            ("denied", "permission_missing", "denied"),
            ("not_determined", "permission_missing", "not_determined"),
            ("something_new", "permission_missing", "unavailable"),
        ] {
            let told = framework(&json!({"location": {"authorization": name}}), "b");
            assert_eq!(told["location"].state, state);
            assert_eq!(told["location"].reason, reason);
        }
    }

    #[test]
    fn capture_reports_individual_grants_without_disabling_other_operations() {
        let permissions = json!({"camera":false,"microphone":true,"screen_recording":false});
        let told = framework(&json!({"capture":{"permissions":permissions}}), "b");
        assert_eq!(told["capture"].coverage["permissions"], permissions);
        assert_eq!(told["capture"].state, "ready");
        assert!(
            told["capture"]
                .capabilities
                .contains(&"capture.record_audio".to_owned())
        );
    }

    #[test]
    fn safari_says_which_grant_is_missing() {
        let read = |directory, readable, automation| {
            framework(
                &json!({"safari": {"directory": directory, "readable": readable, "automation": automation}}),
                "b",
            )
            .remove("safari")
            .unwrap()
        };
        assert_eq!(read(false, false, false).state, "unavailable");
        assert_eq!(read(true, false, true).reason, "Full Disk Access required");
        assert!(read(true, true, false).reason.starts_with("Automation"));
        assert_eq!(read(true, true, true).state, "ready");
    }

    #[test]
    fn vision_without_languages_is_unavailable_and_unknown_sources_are_ignored() {
        let told = framework(&json!({"vision": {"languages": 0}, "made.up": {}}), "b");
        assert_eq!(told["vision"].state, "unavailable");
        assert_eq!(told.len(), 1);
        let working = framework(&json!({"vision": {"languages": 3}}), "b");
        assert_eq!(working["vision"].coverage["language_count"], 3);
    }
}
