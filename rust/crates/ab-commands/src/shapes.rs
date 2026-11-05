use crate::Field;
const REMINDERARGS: &[Field] = &[
    Field {
        name: "id",
        kind: "string",
        omit: true,
    },
    Field {
        name: "list_id",
        kind: "string",
        omit: true,
    },
    Field {
        name: "title",
        kind: "*string",
        omit: true,
    },
    Field {
        name: "notes",
        kind: "*string",
        omit: true,
    },
    Field {
        name: "url",
        kind: "*string",
        omit: true,
    },
    Field {
        name: "due",
        kind: "*string",
        omit: true,
    },
    Field {
        name: "due_all_day",
        kind: "*bool",
        omit: true,
    },
    Field {
        name: "start",
        kind: "*string",
        omit: true,
    },
    Field {
        name: "priority",
        kind: "*int",
        omit: true,
    },
];
const CALENDARARGS: &[Field] = &[
    Field {
        name: "id",
        kind: "string",
        omit: true,
    },
    Field {
        name: "calendar_id",
        kind: "string",
        omit: true,
    },
    Field {
        name: "span",
        kind: "string",
        omit: true,
    },
    Field {
        name: "title",
        kind: "*string",
        omit: true,
    },
    Field {
        name: "notes",
        kind: "*string",
        omit: true,
    },
    Field {
        name: "location",
        kind: "*string",
        omit: true,
    },
    Field {
        name: "url",
        kind: "*string",
        omit: true,
    },
    Field {
        name: "start",
        kind: "*string",
        omit: true,
    },
    Field {
        name: "end",
        kind: "*string",
        omit: true,
    },
    Field {
        name: "all_day",
        kind: "*bool",
        omit: true,
    },
    Field {
        name: "time_zone",
        kind: "*string",
        omit: true,
    },
    Field {
        name: "recurrence",
        kind: "*[]CalendarRecurrence",
        omit: true,
    },
    Field {
        name: "alarms",
        kind: "*[]CalendarAlarm",
        omit: true,
    },
    Field {
        name: "availability",
        kind: "*int",
        omit: true,
    },
];
const CALENDARRECURRENCE: &[Field] = &[
    Field {
        name: "frequency",
        kind: "string",
        omit: false,
    },
    Field {
        name: "interval",
        kind: "int",
        omit: false,
    },
    Field {
        name: "count",
        kind: "int",
        omit: true,
    },
    Field {
        name: "end",
        kind: "string",
        omit: true,
    },
];
const CALENDARALARM: &[Field] = &[
    Field {
        name: "relative_offset",
        kind: "*float64",
        omit: true,
    },
    Field {
        name: "absolute_date",
        kind: "string",
        omit: true,
    },
];
const NOTEARGS: &[Field] = &[
    Field {
        name: "id",
        kind: "string",
        omit: true,
    },
    Field {
        name: "folder_id",
        kind: "string",
        omit: true,
    },
    Field {
        name: "title",
        kind: "*string",
        omit: true,
    },
    Field {
        name: "plain_text",
        kind: "*string",
        omit: true,
    },
    Field {
        name: "html",
        kind: "*string",
        omit: true,
    },
    Field {
        name: "mode",
        kind: "string",
        omit: true,
    },
];
const MESSAGESARGS: &[Field] = &[
    Field {
        name: "resend_of",
        kind: "string",
        omit: true,
    },
    Field {
        name: "id",
        kind: "string",
        omit: true,
    },
    Field {
        name: "chat_id",
        kind: "string",
        omit: true,
    },
    Field {
        name: "handles",
        kind: "[]string",
        omit: true,
    },
    Field {
        name: "service",
        kind: "string",
        omit: true,
    },
    Field {
        name: "text",
        kind: "string",
        omit: true,
    },
    Field {
        name: "files",
        kind: "[]MessageFile",
        omit: true,
    },
    Field {
        name: "part_index",
        kind: "int",
        omit: true,
    },
    Field {
        name: "reaction",
        kind: "string",
        omit: true,
    },
    Field {
        name: "emoji",
        kind: "string",
        omit: true,
    },
    Field {
        name: "remove",
        kind: "bool",
        omit: true,
    },
];
const MESSAGEFILE: &[Field] = &[
    Field {
        name: "name",
        kind: "string",
        omit: false,
    },
    Field {
        name: "upload_id",
        kind: "string",
        omit: true,
    },
    Field {
        name: "data",
        kind: "string",
        omit: true,
    },
];
const NATIVEMESSAGEFILE: &[Field] = &[
    Field {
        name: "name",
        kind: "string",
        omit: false,
    },
    Field {
        name: "path",
        kind: "string",
        omit: false,
    },
    Field {
        name: "digest",
        kind: "string",
        omit: false,
    },
];
const MAILARGS: &[Field] = &[
    Field {
        name: "id",
        kind: "string",
        omit: true,
    },
    Field {
        name: "to",
        kind: "[]string",
        omit: true,
    },
    Field {
        name: "cc",
        kind: "[]string",
        omit: true,
    },
    Field {
        name: "subject",
        kind: "string",
        omit: true,
    },
    Field {
        name: "body",
        kind: "string",
        omit: true,
    },
    Field {
        name: "attachments",
        kind: "[]string",
        omit: true,
    },
    Field {
        name: "reply_to",
        kind: "string",
        omit: true,
    },
    Field {
        name: "forward_of",
        kind: "string",
        omit: true,
    },
    Field {
        name: "mailbox",
        kind: "string",
        omit: true,
    },
    Field {
        name: "account",
        kind: "string",
        omit: true,
    },
    Field {
        name: "read",
        kind: "*bool",
        omit: true,
    },
    Field {
        name: "flagged",
        kind: "*bool",
        omit: true,
    },
];
const MACARGS: &[Field] = &[
    Field {
        name: "document_blob",
        kind: "string",
        omit: true,
    },
    Field {
        name: "format",
        kind: "string",
        omit: true,
    },
    Field {
        name: "app",
        kind: "string",
        omit: true,
    },
    Field {
        name: "title",
        kind: "string",
        omit: true,
    },
    Field {
        name: "body",
        kind: "string",
        omit: true,
    },
    Field {
        name: "action_url",
        kind: "string",
        omit: true,
    },
    Field {
        name: "pane",
        kind: "string",
        omit: true,
    },
    Field {
        name: "image_blob",
        kind: "string",
        omit: true,
    },
    Field {
        name: "text",
        kind: "*string",
        omit: true,
    },
    Field {
        name: "volume",
        kind: "*float64",
        omit: true,
    },
    Field {
        name: "muted",
        kind: "*bool",
        omit: true,
    },
    Field {
        name: "appearance",
        kind: "string",
        omit: true,
    },
    Field {
        name: "bundle_id",
        kind: "string",
        omit: true,
    },
    Field {
        name: "on",
        kind: "*bool",
        omit: true,
    },
    Field {
        name: "duration",
        kind: "float64",
        omit: true,
    },
];
const FILEARGS: &[Field] = &[
    Field {
        name: "root",
        kind: "string",
        omit: false,
    },
    Field {
        name: "path",
        kind: "string",
        omit: true,
    },
    Field {
        name: "parent",
        kind: "string",
        omit: true,
    },
    Field {
        name: "name",
        kind: "string",
        omit: true,
    },
    Field {
        name: "content_base64",
        kind: "string",
        omit: true,
    },
    Field {
        name: "digest",
        kind: "string",
        omit: true,
    },
    Field {
        name: "tags",
        kind: "[]string",
        omit: true,
    },
];
const FOCUSSETARGS: &[Field] = &[
    Field {
        name: "mode",
        kind: "string",
        omit: false,
    },
    Field {
        name: "on",
        kind: "*bool",
        omit: false,
    },
    Field {
        name: "until",
        kind: "string",
        omit: true,
    },
];
const SHORTCUTRUNARGS: &[Field] = &[
    Field {
        name: "shortcut",
        kind: "string",
        omit: false,
    },
    Field {
        name: "input",
        kind: "string",
        omit: true,
    },
    Field {
        name: "input_base64",
        kind: "string",
        omit: true,
    },
    Field {
        name: "timeout_ms",
        kind: "int",
        omit: true,
    },
];
const CAPTUREARGS: &[Field] = &[
    Field {
        name: "blob",
        kind: "raw",
        omit: true,
    },
    Field {
        name: "data_base64",
        kind: "string",
        omit: true,
    },
    Field {
        name: "display",
        kind: "*int",
        omit: true,
    },
    Field {
        name: "window",
        kind: "*int",
        omit: true,
    },
    Field {
        name: "seconds",
        kind: "float64",
        omit: true,
    },
    Field {
        name: "file",
        kind: "string",
        omit: true,
    },
    Field {
        name: "printer",
        kind: "string",
        omit: true,
    },
];
const PLAYBACKARGS: &[Field] = &[
    Field {
        name: "text",
        kind: "string",
        omit: true,
    },
    Field {
        name: "name",
        kind: "string",
        omit: true,
    },
    Field {
        name: "volume",
        kind: "*float64",
        omit: true,
    },
];
const CONTACTGROUPARGS: &[Field] = &[
    Field {
        name: "id",
        kind: "string",
        omit: true,
    },
    Field {
        name: "name",
        kind: "string",
        omit: true,
    },
    Field {
        name: "contact_id",
        kind: "string",
        omit: true,
    },
];
const IPHONETARGET: &[Field] = &[Field {
    name: "device_id",
    kind: "string",
    omit: false,
}];
const IPHONECAPTUREAUDIOARGS: &[Field] = &[
    Field {
        name: "device_id",
        kind: "string",
        omit: false,
    },
    Field {
        name: "seconds",
        kind: "*int",
        omit: true,
    },
];
const IPHONEALARMARGS: &[Field] = &[
    Field {
        name: "device_id",
        kind: "string",
        omit: false,
    },
    Field {
        name: "id",
        kind: "string",
        omit: true,
    },
    Field {
        name: "fire_at",
        kind: "float64",
        omit: true,
    },
    Field {
        name: "duration_seconds",
        kind: "float64",
        omit: true,
    },
    Field {
        name: "snooze_seconds",
        kind: "*float64",
        omit: true,
    },
    Field {
        name: "title",
        kind: "string",
        omit: true,
    },
];
const IPHONELIVEARGS: &[Field] = &[
    Field {
        name: "device_id",
        kind: "string",
        omit: false,
    },
    Field {
        name: "task_id",
        kind: "string",
        omit: false,
    },
    Field {
        name: "title",
        kind: "string",
        omit: false,
    },
    Field {
        name: "status",
        kind: "string",
        omit: false,
    },
    Field {
        name: "progress",
        kind: "float64",
        omit: false,
    },
];
const IPHONENOTIFYARGS: &[Field] = &[
    Field {
        name: "device_id",
        kind: "string",
        omit: false,
    },
    Field {
        name: "title",
        kind: "string",
        omit: false,
    },
    Field {
        name: "body",
        kind: "string",
        omit: false,
    },
    Field {
        name: "buttons",
        kind: "[]IPhoneNotificationButton",
        omit: true,
    },
    Field {
        name: "thread",
        kind: "string",
        omit: true,
    },
    Field {
        name: "time_sensitive",
        kind: "bool",
        omit: true,
    },
];
const IPHONENOTIFICATIONBUTTON: &[Field] = &[
    Field {
        name: "id",
        kind: "string",
        omit: false,
    },
    Field {
        name: "title",
        kind: "string",
        omit: false,
    },
];
const IPHONEGEOFENCE: &[Field] = &[
    Field {
        name: "device_id",
        kind: "string",
        omit: false,
    },
    Field {
        name: "name",
        kind: "string",
        omit: false,
    },
    Field {
        name: "latitude",
        kind: "*float64",
        omit: false,
    },
    Field {
        name: "longitude",
        kind: "*float64",
        omit: false,
    },
    Field {
        name: "radius",
        kind: "*float64",
        omit: false,
    },
];
const IPHONEGEOFENCENAME: &[Field] = &[
    Field {
        name: "device_id",
        kind: "string",
        omit: false,
    },
    Field {
        name: "name",
        kind: "string",
        omit: false,
    },
];
const IPHONESIGNIFICANT: &[Field] = &[
    Field {
        name: "device_id",
        kind: "string",
        omit: false,
    },
    Field {
        name: "enabled",
        kind: "*bool",
        omit: false,
    },
];
const IPHONEMOTIONHISTORYARGS: &[Field] = &[
    Field {
        name: "device_id",
        kind: "string",
        omit: false,
    },
    Field {
        name: "days",
        kind: "int",
        omit: false,
    },
];
const IPHONESCHEDULEARGS: &[Field] = &[
    Field {
        name: "device_id",
        kind: "string",
        omit: false,
    },
    Field {
        name: "start_minute",
        kind: "int",
        omit: false,
    },
    Field {
        name: "end_minute",
        kind: "int",
        omit: false,
    },
];
const IPHONEHEALTHARGS: &[Field] = &[
    Field {
        name: "device_id",
        kind: "string",
        omit: false,
    },
    Field {
        name: "kind",
        kind: "string",
        omit: false,
    },
    Field {
        name: "value",
        kind: "float64",
        omit: false,
    },
    Field {
        name: "start",
        kind: "string",
        omit: false,
    },
    Field {
        name: "tag",
        kind: "string",
        omit: false,
    },
    Field {
        name: "activity",
        kind: "string",
        omit: false,
    },
];
pub fn fields(name: &str) -> &'static [Field] {
    match name {
        "PDFAnnotateArgs" => &[
            Field {
                name: "data_base64",
                kind: "string",
                omit: true,
            },
            Field {
                name: "file",
                kind: "string",
                omit: true,
            },
            Field {
                name: "blob",
                kind: "raw",
                omit: true,
            },
            Field {
                name: "annotations",
                kind: "[]PDFAnnotation",
                omit: false,
            },
        ],
        "PDFAnnotation" => &[
            Field {
                name: "page",
                kind: "int",
                omit: false,
            },
            Field {
                name: "type",
                kind: "string",
                omit: false,
            },
            Field {
                name: "text",
                kind: "string",
                omit: false,
            },
            Field {
                name: "x",
                kind: "float64",
                omit: false,
            },
            Field {
                name: "y",
                kind: "float64",
                omit: false,
            },
            Field {
                name: "width",
                kind: "float64",
                omit: false,
            },
            Field {
                name: "height",
                kind: "float64",
                omit: false,
            },
        ],
        "ReminderArgs" => REMINDERARGS,
        "CalendarArgs" => CALENDARARGS,
        "CalendarRecurrence" => CALENDARRECURRENCE,
        "CalendarAlarm" => CALENDARALARM,
        "NoteArgs" => NOTEARGS,
        "MessagesArgs" => MESSAGESARGS,
        "MessageFile" => MESSAGEFILE,
        "NativeMessageFile" => NATIVEMESSAGEFILE,
        "MacArgs" => MACARGS,
        "MailArgs" => MAILARGS,
        "FileArgs" => FILEARGS,
        "FocusSetArgs" => FOCUSSETARGS,
        "ShortcutRunArgs" => SHORTCUTRUNARGS,
        "CaptureArgs" => CAPTUREARGS,
        "PlaybackArgs" => PLAYBACKARGS,
        "ContactGroupArgs" => CONTACTGROUPARGS,
        "IPhoneTarget" => IPHONETARGET,
        "IPhoneCaptureAudioArgs" => IPHONECAPTUREAUDIOARGS,
        "IPhoneAlarmArgs" => IPHONEALARMARGS,
        "IPhoneLiveArgs" => IPHONELIVEARGS,
        "IPhoneNotifyArgs" => IPHONENOTIFYARGS,
        "IPhoneNotificationButton" => IPHONENOTIFICATIONBUTTON,
        "IPhoneGeofence" => IPHONEGEOFENCE,
        "IPhoneGeofenceName" => IPHONEGEOFENCENAME,
        "IPhoneSignificant" => IPHONESIGNIFICANT,
        "IPhoneMotionHistoryArgs" => IPHONEMOTIONHISTORYARGS,
        "IPhoneScheduleArgs" => IPHONESCHEDULEARGS,
        "IPhoneHealthArgs" => IPHONEHEALTHARGS,
        _ => &[],
    }
}
