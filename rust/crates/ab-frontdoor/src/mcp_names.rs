pub(crate) const NOTES_SEARCH: &str = "notes.search";
pub(crate) const NOTES_GET: &str = "notes.get";
pub(crate) const MAIL_SEARCH: &str = "mail.search";

pub(crate) const RENAMED: &[(&str, &str)] = &[
    ("calendar.agenda", "calendar.list"),
    ("calendar.range", "calendar.list_grouped"),
    ("calendar.calendars", "calendar.list_calendars"),
    ("calendar.free_slots", "calendar.list_free_slots"),
    ("reminders.items", "reminders.list"),
    ("reminders.lists", "reminders.list_lists"),
    ("messages.chats", "messages.list_chats"),
    ("messages.thread", "messages.list"),
    ("messages.handles", "messages.resolve_handles"),
    ("messages.collection", "messages.list_media"),
    ("messages.shared", "messages.list_shared"),
    ("search", "bridge.search"),
    ("records.get", "bridge.get_record"),
    ("records.query", "bridge.list_records"),
    ("sources.status", "bridge.list_sources"),
    ("replica.status", "bridge.get_replica_status"),
    ("commands.status", "bridge.get_change"),
    ("commands.list", "bridge.list_changes"),
    ("helper.version", "bridge.get_version"),
    ("blobs.read", "bridge.read_blob"),
    ("blobs.stage", "bridge.stage_blob"),
    ("contacts.lookup", "contacts.search"),
    ("usage.summary", "screentime.summary"),
    ("pdf.text", "pdf.read_text"),
    ("vision.ocr", "vision.read_text"),
    ("files.evict", "files.remove_local_copy"),
    ("maps.nearby", "maps.search_nearby"),
    ("maps.directions", "maps.get_directions"),
    ("maps.eta", "maps.get_eta"),
    ("location.current", "location.get"),
    ("mail.body", "mail.get_body"),
    ("photos.albums", "photos.list_albums"),
    ("photos.places", "photos.list_places"),
    ("photos.contact_sheet", "photos.preview_grid"),
    ("photos.album_create", "photos.create_album"),
    ("photos.album_rename", "photos.rename_album"),
    ("photos.album_add", "photos.add_to_album"),
    ("photos.album_remove", "photos.remove_from_album"),
    ("photos.album_delete", "photos.delete_album"),
    ("capture.camera_photo", "capture.take_photo"),
    ("capture.screenshot", "capture.take_screenshot"),
    ("mac.screen_record", "capture.record_screen"),
    ("iphone.alarm.cancel", "iphone.cancel_alarm"),
    ("iphone.alarm.create", "iphone.create_alarm"),
    ("iphone.alarm.list", "iphone.list_alarms"),
    ("iphone.alarm.update", "iphone.update_alarm"),
    ("iphone.capture.audio", "iphone.record_audio"),
    ("iphone.capture.photo", "iphone.take_photo"),
    ("iphone.device.status", "iphone.get_status"),
    ("iphone.devices", "iphone.list_devices"),
    ("iphone.geofence.add", "iphone.add_geofence"),
    ("iphone.geofence.list", "iphone.list_geofences"),
    ("iphone.geofence.remove", "iphone.remove_geofence"),
    ("iphone.health.log", "iphone.log_health"),
    ("iphone.live.end", "iphone.end_live_activity"),
    ("iphone.live.start", "iphone.start_live_activity"),
    ("iphone.live.update", "iphone.update_live_activity"),
    ("iphone.location.current", "iphone.get_location"),
    (
        "iphone.location.significant",
        "iphone.set_location_tracking",
    ),
    ("iphone.motion.activity", "iphone.get_motion"),
    ("iphone.motion.history", "iphone.list_motion"),
    ("iphone.screentime.cancel", "iphone.cancel_screentime"),
    ("iphone.screentime.schedule", "iphone.schedule_screentime"),
    ("iphone.screentime.shield", "iphone.shield_apps"),
    ("iphone.screentime.unshield", "iphone.unshield_apps"),
    ("iphone.timer.cancel", "iphone.cancel_timer"),
    ("iphone.timer.start", "iphone.start_timer"),
    ("photos.fetch", "photos.download"),
    ("photos.release", "photos.release_download"),
];

const APPS: &[(&str, &str)] = &[
    ("screen", "screentime"),
    ("usage", "screentime"),
    ("imessage", "messages"),
    ("sms", "messages"),
    ("text", "messages"),
    ("message", "messages"),
    ("chat", "messages"),
    ("email", "mail"),
    ("note", "notes"),
    ("reminder", "reminders"),
    ("todo", "reminders"),
    ("contact", "contacts"),
    ("people", "contacts"),
    ("photo", "photos"),
    ("picture", "photos"),
    ("file", "files"),
    ("event", "calendar"),
    ("cal", "calendar"),
    ("map", "maps"),
    ("phone", "iphone"),
    ("records", "bridge"),
    ("record", "bridge"),
    ("search", "bridge"),
    ("commands", "bridge"),
    ("blobs", "bridge"),
    ("sources", "bridge"),
    ("status", "bridge"),
    ("helper", "bridge"),
];

const VERBS: &[(&str, &str)] = &[
    ("fetch", "get"),
    ("show", "get"),
    ("view", "get"),
    ("find", "search"),
    ("lookup", "search"),
    ("query", "search"),
    ("all", "list"),
    ("ls", "list"),
    ("items", "list"),
    ("new", "create"),
    ("make", "create"),
    ("rm", "delete"),
    ("del", "delete"),
    ("edit", "update"),
    ("modify", "update"),
    ("change", "update"),
    ("evict", "remove"),
];

const FILLER: &[&str] = &["to", "from", "by", "with", "of", "the", "a", "my", "time"];

pub(crate) fn renamed(internal: &str) -> Option<&'static str> {
    RENAMED
        .iter()
        .find(|(old, _)| *old == internal)
        .map(|(_, public)| *public)
}

pub(crate) fn retired(name: &str) -> Option<&'static str> {
    let name = name.trim();
    RENAMED
        .iter()
        .find(|(old, _)| *old == name || old.replace('.', "_") == name)
        .map(|(_, public)| *public)
}

fn singular(word: &str) -> String {
    if word.len() > 3 && word.ends_with("ies") {
        return format!("{}y", &word[..word.len() - 3]);
    }
    if word.len() > 3 && word.ends_with('s') && !word.ends_with("ss") {
        return word[..word.len() - 1].to_owned();
    }
    word.to_owned()
}

pub(crate) fn key(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    let mut words: Vec<String> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    if words.first().is_some_and(|w| w == "screen") && words.get(1).is_some_and(|w| w == "time") {
        words.remove(1);
    }
    let mut bag = Vec::new();
    for (index, word) in words.iter().enumerate() {
        let word = singular(word);
        if FILLER.contains(&word.as_str()) {
            continue;
        }
        let word = if index == 0 {
            APPS.iter()
                .find(|(from, _)| *from == word || singular(from) == word)
                .map_or_else(|| word.clone(), |(_, to)| singular(to))
        } else {
            word
        };
        let word = VERBS
            .iter()
            .find(|(from, _)| *from == word)
            .map_or(word, |(_, to)| (*to).to_owned());
        bag.push(word);
    }
    bag.sort_unstable();
    bag.join(" ")
}
