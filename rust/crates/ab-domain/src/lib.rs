#![forbid(unsafe_code)]
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Source {
    #[serde(rename = "location")]
    Location,
    #[serde(rename = "audio")]
    Audio,
    #[serde(rename = "calendar")]
    Calendar,
    #[serde(rename = "capture")]
    Capture,
    #[serde(rename = "contacts")]
    Contacts,
    #[serde(rename = "files")]
    Files,
    #[serde(rename = "focus")]
    Focus,
    #[serde(rename = "health")]
    Health,
    #[serde(rename = "iphone")]
    Iphone,
    #[serde(rename = "mac")]
    Mac,
    #[serde(rename = "mail")]
    Mail,
    #[serde(rename = "maps")]
    Maps,
    #[serde(rename = "messages")]
    Messages,
    #[serde(rename = "notes")]
    Notes,
    #[serde(rename = "pdf")]
    Pdf,
    #[serde(rename = "phone")]
    Phone,
    #[serde(rename = "print")]
    Print,
    #[serde(rename = "reminders")]
    Reminders,
    #[serde(rename = "safari")]
    Safari,
    #[serde(rename = "shortcuts")]
    Shortcuts,
    #[serde(rename = "speech")]
    Speech,
    #[serde(rename = "system")]
    System,
    #[serde(rename = "usage")]
    Usage,
    #[serde(rename = "vision")]
    Vision,
    #[serde(rename = "voice_memos")]
    VoiceMemos,
    #[serde(rename = "voicemail")]
    Voicemail,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Collection {
    #[serde(rename = "calendar.calendar")]
    CalendarCalendar,
    #[serde(rename = "calendar.event")]
    CalendarEvent,
    #[serde(rename = "contacts.contact")]
    ContactsContact,
    #[serde(rename = "contacts.container")]
    ContactsContainer,
    #[serde(rename = "contacts.group")]
    ContactsGroup,
    #[serde(rename = "files.item")]
    FilesItem,
    #[serde(rename = "focus.status")]
    FocusStatus,
    #[serde(rename = "health.sample")]
    HealthSample,
    #[serde(rename = "iphone.location")]
    IphoneLocation,
    #[serde(rename = "mac.app")]
    MacApp,
    #[serde(rename = "mac.audio_device")]
    MacAudioDevice,
    #[serde(rename = "mac.bluetooth_device")]
    MacBluetoothDevice,
    #[serde(rename = "mac.display")]
    MacDisplay,
    #[serde(rename = "mac.network")]
    MacNetwork,
    #[serde(rename = "mac.power")]
    MacPower,
    #[serde(rename = "mail.account")]
    MailAccount,
    #[serde(rename = "mail.attachment")]
    MailAttachment,
    #[serde(rename = "mail.mailbox")]
    MailMailbox,
    #[serde(rename = "mail.message")]
    MailMessage,
    #[serde(rename = "messages.attachment")]
    MessagesAttachment,
    #[serde(rename = "messages.chat")]
    MessagesChat,
    #[serde(rename = "messages.handle")]
    MessagesHandle,
    #[serde(rename = "messages.message")]
    MessagesMessage,
    #[serde(rename = "messages.reaction")]
    MessagesReaction,
    #[serde(rename = "notes.folder")]
    NotesFolder,
    #[serde(rename = "notes.note")]
    NotesNote,
    #[serde(rename = "pdf.artifact")]
    PdfArtifact,
    #[serde(rename = "phone.call")]
    PhoneCall,
    #[serde(rename = "reminders.attachment")]
    RemindersAttachment,
    #[serde(rename = "reminders.list")]
    RemindersList,
    #[serde(rename = "reminders.reminder")]
    RemindersReminder,
    #[serde(rename = "safari.bookmark")]
    SafariBookmark,
    #[serde(rename = "safari.cloud_tab")]
    SafariCloudTab,
    #[serde(rename = "safari.history")]
    SafariHistory,
    #[serde(rename = "safari.reading_list")]
    SafariReadingList,
    #[serde(rename = "safari.tab")]
    SafariTab,
    #[serde(rename = "shortcuts.folder")]
    ShortcutsFolder,
    #[serde(rename = "shortcuts.shortcut")]
    ShortcutsShortcut,
    #[serde(rename = "system.command")]
    SystemCommand,
    #[serde(rename = "system.replica")]
    SystemReplica,
    #[serde(rename = "system.source")]
    SystemSource,
    #[serde(rename = "usage.app_day")]
    UsageAppDay,
    #[serde(rename = "usage.device")]
    UsageDevice,
    #[serde(rename = "voice_memos.folder")]
    VoiceMemosFolder,
    #[serde(rename = "voice_memos.recording")]
    VoiceMemosRecording,
    #[serde(rename = "voicemail.message")]
    VoicemailMessage,
}
impl Collection {
    pub const ALL: &'static [Self] = &[
        Self::CalendarCalendar,
        Self::CalendarEvent,
        Self::ContactsContact,
        Self::ContactsContainer,
        Self::ContactsGroup,
        Self::FilesItem,
        Self::FocusStatus,
        Self::HealthSample,
        Self::IphoneLocation,
        Self::MacApp,
        Self::MacAudioDevice,
        Self::MacBluetoothDevice,
        Self::MacDisplay,
        Self::MacNetwork,
        Self::MacPower,
        Self::MailAccount,
        Self::MailAttachment,
        Self::MailMailbox,
        Self::MailMessage,
        Self::MessagesAttachment,
        Self::MessagesChat,
        Self::MessagesHandle,
        Self::MessagesMessage,
        Self::MessagesReaction,
        Self::NotesFolder,
        Self::NotesNote,
        Self::PdfArtifact,
        Self::PhoneCall,
        Self::RemindersAttachment,
        Self::RemindersList,
        Self::RemindersReminder,
        Self::SafariBookmark,
        Self::SafariCloudTab,
        Self::SafariHistory,
        Self::SafariReadingList,
        Self::SafariTab,
        Self::ShortcutsFolder,
        Self::ShortcutsShortcut,
        Self::SystemCommand,
        Self::SystemReplica,
        Self::SystemSource,
        Self::UsageAppDay,
        Self::UsageDevice,
        Self::VoiceMemosFolder,
        Self::VoiceMemosRecording,
        Self::VoicemailMessage,
    ];
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::CalendarCalendar => "cal",
            Self::CalendarEvent => "evt",
            Self::ContactsContact => "con",
            Self::ContactsContainer => "ctn",
            Self::ContactsGroup => "grp",
            Self::FilesItem => "fil",
            Self::FocusStatus => "fcs",
            Self::HealthSample => "hsm",
            Self::IphoneLocation => "ipl",
            Self::MacApp => "map",
            Self::MacAudioDevice => "mad",
            Self::MacBluetoothDevice => "mbt",
            Self::MacDisplay => "mdp",
            Self::MacNetwork => "mnw",
            Self::MacPower => "mpw",
            Self::MailAccount => "mac",
            Self::MailAttachment => "mat",
            Self::MailMailbox => "mbx",
            Self::MailMessage => "mml",
            Self::MessagesAttachment => "att",
            Self::MessagesChat => "cht",
            Self::MessagesHandle => "hdl",
            Self::MessagesMessage => "msg",
            Self::MessagesReaction => "rxn",
            Self::NotesFolder => "fld",
            Self::NotesNote => "nte",
            Self::PdfArtifact => "pda",
            Self::PhoneCall => "call",
            Self::RemindersAttachment => "rat",
            Self::RemindersList => "lst",
            Self::RemindersReminder => "rmd",
            Self::SafariBookmark => "sfb",
            Self::SafariCloudTab => "sfc",
            Self::SafariHistory => "sfh",
            Self::SafariReadingList => "sfr",
            Self::SafariTab => "sft",
            Self::ShortcutsFolder => "scf",
            Self::ShortcutsShortcut => "sct",
            Self::SystemCommand => "cmd",
            Self::SystemReplica => "rpl",
            Self::SystemSource => "src",
            Self::UsageAppDay => "usd",
            Self::UsageDevice => "dev",
            Self::VoiceMemosFolder => "folder",
            Self::VoiceMemosRecording => "recording",
            Self::VoicemailMessage => "message",
        }
    }
    pub const fn source(self) -> Source {
        match self {
            Self::CalendarCalendar | Self::CalendarEvent => Source::Calendar,
            Self::ContactsContact | Self::ContactsContainer | Self::ContactsGroup => {
                Source::Contacts
            }
            Self::FilesItem => Source::Files,
            Self::FocusStatus => Source::Focus,
            Self::HealthSample => Source::Health,
            Self::IphoneLocation => Source::Iphone,
            Self::MacApp
            | Self::MacAudioDevice
            | Self::MacBluetoothDevice
            | Self::MacDisplay
            | Self::MacNetwork
            | Self::MacPower => Source::Mac,
            Self::MailAccount | Self::MailAttachment | Self::MailMailbox | Self::MailMessage => {
                Source::Mail
            }
            Self::MessagesAttachment
            | Self::MessagesChat
            | Self::MessagesHandle
            | Self::MessagesMessage
            | Self::MessagesReaction => Source::Messages,
            Self::NotesFolder | Self::NotesNote => Source::Notes,
            Self::PdfArtifact => Source::Pdf,
            Self::PhoneCall => Source::Phone,
            Self::RemindersAttachment | Self::RemindersList | Self::RemindersReminder => {
                Source::Reminders
            }
            Self::SafariBookmark
            | Self::SafariCloudTab
            | Self::SafariHistory
            | Self::SafariReadingList
            | Self::SafariTab => Source::Safari,
            Self::ShortcutsFolder | Self::ShortcutsShortcut => Source::Shortcuts,
            Self::SystemCommand | Self::SystemReplica | Self::SystemSource => Source::System,
            Self::UsageAppDay | Self::UsageDevice => Source::Usage,
            Self::VoiceMemosFolder | Self::VoiceMemosRecording => Source::VoiceMemos,
            Self::VoicemailMessage => Source::Voicemail,
        }
    }
}
fn valid_id(value: &str, prefix: &str) -> bool {
    value
        .strip_prefix(prefix)
        .and_then(|v| v.strip_prefix('_'))
        .is_some_and(|body| {
            body.len() == 26
                && body.as_bytes()[0] <= b'7'
                && body
                    .bytes()
                    .all(|b| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b))
        })
}
macro_rules! typed_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);
        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl FromStr for $name {
            type Err = ErrorCode;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                if valid_id(value, $prefix) {
                    Ok(Self(value.to_owned()))
                } else {
                    Err(ErrorCode::Invalid)
                }
            }
        }
        impl TryFrom<String> for $name {
            type Error = ErrorCode;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                value.parse()
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}
typed_id!(CalendarCalendarId, "cal");
typed_id!(CalendarEventId, "evt");
typed_id!(ContactsContactId, "con");
typed_id!(ContactsContainerId, "ctn");
typed_id!(ContactsGroupId, "grp");
typed_id!(FilesItemId, "fil");
typed_id!(FocusStatusId, "fcs");
typed_id!(HealthSampleId, "hsm");
typed_id!(IphoneLocationId, "ipl");
typed_id!(MacAppId, "map");
typed_id!(MacAudioDeviceId, "mad");
typed_id!(MacBluetoothDeviceId, "mbt");
typed_id!(MacDisplayId, "mdp");
typed_id!(MacNetworkId, "mnw");
typed_id!(MacPowerId, "mpw");
typed_id!(MailAccountId, "mac");
typed_id!(MailAttachmentId, "mat");
typed_id!(MailMailboxId, "mbx");
typed_id!(MailMessageId, "mml");
typed_id!(MessagesAttachmentId, "att");
typed_id!(MessagesChatId, "cht");
typed_id!(MessagesHandleId, "hdl");
typed_id!(MessagesMessageId, "msg");
typed_id!(MessagesReactionId, "rxn");
typed_id!(NotesFolderId, "fld");
typed_id!(NotesNoteId, "nte");
typed_id!(PdfArtifactId, "pda");
typed_id!(PhoneCallId, "call");
typed_id!(RemindersAttachmentId, "rat");
typed_id!(RemindersListId, "lst");
typed_id!(RemindersReminderId, "rmd");
typed_id!(SafariBookmarkId, "sfb");
typed_id!(SafariCloudTabId, "sfc");
typed_id!(SafariHistoryId, "sfh");
typed_id!(SafariReadingListId, "sfr");
typed_id!(SafariTabId, "sft");
typed_id!(ShortcutsFolderId, "scf");
typed_id!(ShortcutsShortcutId, "sct");
typed_id!(SystemCommandId, "cmd");
typed_id!(SystemReplicaId, "rpl");
typed_id!(SystemSourceId, "src");
typed_id!(UsageAppDayId, "usd");
typed_id!(UsageDeviceId, "dev");
typed_id!(VoiceMemosFolderId, "folder");
typed_id!(VoiceMemosRecordingId, "recording");
typed_id!(VoicemailMessageId, "message");
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CommandState {
    #[serde(rename = "awaiting_approval")]
    AwaitingApproval,
    #[serde(rename = "rejected_by_owner")]
    RejectedByOwner,
    #[serde(rename = "admitted")]
    Admitted,
    #[serde(rename = "prepared")]
    Prepared,
    #[serde(rename = "dispatched")]
    Dispatched,
    #[serde(rename = "observed")]
    Observed,
    #[serde(rename = "delivered")]
    Delivered,
    #[serde(rename = "delivery_failed")]
    DeliveryFailed,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "indeterminate")]
    Indeterminate,
    #[serde(rename = "rejected")]
    Rejected,
    #[serde(rename = "expired")]
    Expired,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Disposition {
    #[serde(rename = "awaiting_approval")]
    AwaitingApproval,
    #[serde(rename = "rejected_by_owner")]
    RejectedByOwner,
    #[serde(rename = "rejected_before_dispatch")]
    RejectedBeforeDispatch,
    #[serde(rename = "failed_no_effect")]
    FailedNoEffect,
    #[serde(rename = "locally_observed")]
    LocallyObserved,
    #[serde(rename = "delivered")]
    Delivered,
    #[serde(rename = "delivery_failed")]
    DeliveryFailed,
    #[serde(rename = "indeterminate")]
    Indeterminate,
    #[serde(rename = "expired")]
    Expired,
    #[serde(rename = "absent_confirmed")]
    AbsentConfirmed,
}
impl CommandState {
    pub fn transition(self, next: Self) -> Result<Self, ErrorCode> {
        use CommandState::{
            Admitted, AwaitingApproval, Delivered, DeliveryFailed, Dispatched, Expired, Failed,
            Indeterminate, Observed, Prepared, Rejected, RejectedByOwner,
        };
        let valid = matches!(
            (self, next),
            (
                AwaitingApproval,
                Admitted | RejectedByOwner | Expired | Rejected
            ) | (Admitted, Prepared | Rejected | Expired)
                | (Prepared, Dispatched | Rejected | Expired)
                | (
                    Dispatched,
                    Observed | Indeterminate | Failed | Rejected | Delivered | DeliveryFailed
                )
                | (Observed, Delivered | DeliveryFailed)
                | (
                    Indeterminate,
                    Observed | Delivered | Failed | DeliveryFailed
                )
        );
        if valid {
            Ok(next)
        } else {
            Err(ErrorCode::Conflict)
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationCapability {
    pub operation: String,
    pub proven: bool,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pack_version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<OperationCapability>,
    pub fields: String,
    pub missing: Option<Vec<String>>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Record {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tombstone_reason: String,
    pub collection: Collection,
    pub id: RecordId,
    pub version: u64,
    pub source: Source,
    pub observed_at: String,
    pub deleted: bool,
    pub coverage: Coverage,
    pub data: Box<serde_json::value::RawValue>,
}
impl<'de> Deserialize<'de> for Record {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Fields {
            #[serde(default)]
            tombstone_reason: String,
            collection: Collection,
            id: String,
            version: u64,
            source: Source,
            observed_at: String,
            deleted: bool,
            coverage: Coverage,
            data: Box<serde_json::value::RawValue>,
        }
        let fields = Fields::deserialize(deserializer)?;
        let id = if fields.collection == Collection::SystemCommand {
            CommandId::try_from(fields.id.clone()).map_err(serde::de::Error::custom)?;
            RecordId(fields.id)
        } else {
            RecordId::try_from(fields.id).map_err(serde::de::Error::custom)?
        };
        Ok(Self {
            tombstone_reason: fields.tombstone_reason,
            collection: fields.collection,
            id,
            version: fields.version,
            source: fields.source,
            observed_at: fields.observed_at,
            deleted: fields.deleted,
            coverage: fields.coverage,
            data: fields.data,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCode {
    #[serde(rename = "deadline")]
    Deadline,
    #[serde(rename = "forbidden")]
    Forbidden,
    #[serde(rename = "unsupported")]
    Unsupported,
    #[serde(rename = "unauthorized")]
    Unauthorized,
    #[serde(rename = "unavailable")]
    Unavailable,
    #[serde(rename = "conflict")]
    Conflict,
    #[serde(rename = "reset")]
    Reset,
    #[serde(rename = "expired")]
    Expired,
    #[serde(rename = "corrupt")]
    Corrupt,
    #[serde(rename = "capacity")]
    Capacity,
    #[serde(rename = "indeterminate")]
    Indeterminate,
    #[serde(rename = "invalid")]
    Invalid,
    #[serde(rename = "not_found")]
    NotFound,
}
impl ErrorCode {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Deadline => "query deadline exceeded",
            Self::Forbidden => "query grant rejected",
            Self::Unsupported => "operation unsupported",
            Self::Unauthorized => "credential or scope rejected",
            Self::Unavailable => "peer unavailable",
            Self::Conflict => "state conflict",
            Self::Reset => "history must be reset",
            Self::Expired => "credential expired",
            Self::Corrupt => "invalid integrity evidence",
            Self::Capacity => "capacity exceeded",
            Self::Indeterminate => "effect cannot be established",
            Self::Invalid => "invalid input",
            Self::NotFound => "record not found",
        }
    }
    pub const fn code(self) -> &'static str {
        match self {
            Self::Deadline => "deadline",
            Self::Forbidden => "forbidden",
            Self::Unsupported => "unsupported",
            Self::Unauthorized => "unauthorized",
            Self::Unavailable => "unavailable",
            Self::Conflict => "conflict",
            Self::Reset => "reset",
            Self::Expired => "expired",
            Self::Corrupt => "corrupt",
            Self::Capacity => "capacity",
            Self::Indeterminate => "indeterminate",
            Self::Invalid => "invalid",
            Self::NotFound => "not_found",
        }
    }
    pub const fn retryable(self) -> bool {
        matches!(self, Self::Capacity | Self::Unavailable)
    }
}
impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}
impl std::error::Error for ErrorCode {}
#[derive(Clone, Debug, thiserror::Error, Serialize, Deserialize)]
#[error("{code}")]
pub struct ContractError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<Box<Record>>,
    pub retryable: bool,
}
impl From<ErrorCode> for ContractError {
    fn from(code: ErrorCode) -> Self {
        Self {
            code,
            message: code.message().into(),
            current: None,
            retryable: code.retryable(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RecordId(String);
impl RecordId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn collection(&self) -> Result<Collection, ErrorCode> {
        Collection::ALL
            .iter()
            .copied()
            .find(|c| valid_id(&self.0, c.prefix()))
            .ok_or(ErrorCode::Invalid)
    }
}
impl TryFrom<String> for RecordId {
    type Error = ErrorCode;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let id = Self(value);
        id.collection()?;
        Ok(id)
    }
}
impl From<RecordId> for String {
    fn from(value: RecordId) -> Self {
        value.0
    }
}
impl FromStr for RecordId {
    type Err = ErrorCode;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CommandId(String);
impl TryFrom<String> for CommandId {
    type Error = ErrorCode;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
        {
            return Err(ErrorCode::Invalid);
        }
        Ok(Self(value))
    }
}
impl From<CommandId> for String {
    fn from(value: CommandId) -> Self {
        value.0
    }
}
impl FromStr for CommandId {
    type Err = ErrorCode;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}
impl CommandId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Record {
    pub fn validate(&self) -> Result<(), ErrorCode> {
        if self.collection == Collection::SystemCommand {
            CommandId::try_from(self.id.as_str().to_owned())?;
            return Ok(());
        }
        if self.id.collection()? != self.collection || self.source != self.collection.source() {
            return Err(ErrorCode::Invalid);
        }
        Ok(())
    }
}
