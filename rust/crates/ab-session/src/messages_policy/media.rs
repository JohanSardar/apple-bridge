use super::{MEDIA_SCENARIOS, exact_tag, normalize, percent_decoded, tag_run};
use std::collections::BTreeSet;

fn last_path_component(path: &str) -> &str {
    path.rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(path)
}

#[derive(Clone, Debug)]
pub struct MediaProof {
    pub baseline: i64,
    pub names: BTreeSet<String>,
    display: std::collections::BTreeMap<String, BTreeSet<String>>,
}

impl MediaProof {
    #[must_use]
    pub fn new(registration: &serde_json::Value, tag: &str) -> Option<Self> {
        if !exact_tag(tag) {
            return None;
        }
        let run = tag_run(tag);
        let baseline = whole(&registration["baseline_max_row_id"]).filter(|row| *row >= 0)?;
        if !MEDIA_SCENARIOS.contains(&registration["scenario"].as_str()?)
            || registration["run_id"].as_str()? != run
        {
            return None;
        }
        let files = registration["media_attachment_names"].as_array()?;
        if files.is_empty() {
            return None;
        }
        let mut names = BTreeSet::new();
        let mut display = std::collections::BTreeMap::new();
        for (index, file) in files.iter().enumerate() {
            let name = file.as_str()?;
            if !name.starts_with(&format!("ab-{run}."))
                || name != last_path_component(name)
                || name.contains('%')
                || name.contains('\n')
            {
                return None;
            }
            let staged = format!("{index}-{name}");
            names.insert(name.to_owned());
            names.insert(staged.clone());
            let both: BTreeSet<String> = [name.to_owned(), staged.clone()].into();
            display.insert(name.to_owned(), both.clone());
            display.insert(staged, both);
        }
        Some(Self {
            baseline,
            names,
            display,
        })
    }

    #[must_use]
    pub fn authorizes(
        &self,
        row: i64,
        attachments: &[String],
        peers: &[String],
        own: &BTreeSet<String>,
    ) -> bool {
        row > self.baseline
            && !peers.is_empty()
            && peers.iter().all(|peer| own.contains(&normalize(peer)))
            && !attachments.is_empty()
            && attachments
                .iter()
                .all(|name| self.names.contains(last_path_component(name)))
    }

    #[must_use]
    pub fn target_names(&self, attachments: &[String]) -> Vec<String> {
        let mut names = BTreeSet::new();
        for attachment in attachments {
            if let Some(found) = self.display.get(last_path_component(attachment)) {
                names.extend(found.iter().cloned());
            }
        }
        names.into_iter().collect()
    }
}

fn whole(value: &serde_json::Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        let number = value.as_f64()?;
        #[allow(clippy::cast_possible_truncation)]
        (number.fract() == 0.0).then_some(number as i64)
    })
}

#[must_use]
pub fn media_value_matches(value: &str, names: &BTreeSet<String>) -> bool {
    if value.is_empty() {
        return false;
    }
    let decoded = percent_decoded(value).unwrap_or_else(|| value.to_owned());
    if names.contains(&decoded) {
        return true;
    }
    let fields: Vec<&str> = decoded.split(", ").collect();
    if fields.len() >= 3 && fields[1..].iter().any(|field| names.contains(*field)) {
        return true;
    }
    if value.starts_with("file:") {
        return names.contains(last_path_component(&decoded));
    }
    decoded.starts_with('/') && names.contains(last_path_component(&decoded))
}

#[must_use]
pub fn media_direction(description: &str) -> Option<bool> {
    let fields: Vec<&str> = description.split(", ").collect();
    if fields.len() < 2 || fields[0].is_empty() {
        return None;
    }
    if ["Your iMessage", "Your SMS", "Your RCS"].contains(&fields[0]) {
        return Some(true);
    }
    (!fields[0].starts_with("Your ")).then_some(false)
}

#[derive(Clone, Debug, Default)]
pub struct MediaBubble {
    pub outgoing: Option<bool>,
    pub values: Vec<String>,
}

#[must_use]
pub fn media_target(
    bubbles: &[MediaBubble],
    names: &BTreeSet<String>,
    outgoing: bool,
    allow_self_echo: bool,
    equivalent_owned: bool,
    owned_copies: usize,
) -> Option<usize> {
    let naming = |bubble: &MediaBubble| {
        bubble
            .values
            .iter()
            .any(|value| media_value_matches(value, names))
    };
    let targets: Vec<usize> = bubbles
        .iter()
        .enumerate()
        .filter(|(_, bubble)| bubble.outgoing == Some(outgoing) && naming(bubble))
        .map(|(index, _)| index)
        .collect();
    if !targets.is_empty() && (targets.len() <= owned_copies || equivalent_owned) {
        return targets.first().copied();
    }
    if !targets.is_empty() || !allow_self_echo {
        return None;
    }
    let echoes: Vec<usize> = bubbles
        .iter()
        .enumerate()
        .filter(|(_, bubble)| naming(bubble))
        .map(|(index, _)| index)
        .collect();
    if !echoes.is_empty() && (echoes.len() <= owned_copies || equivalent_owned) {
        echoes.first().copied()
    } else {
        None
    }
}

#[must_use]
pub fn media_cleanup_check(fixture: &serde_json::Value) -> Option<serde_json::Value> {
    let strings = |value: &serde_json::Value| -> Vec<String> {
        value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let proof = MediaProof::new(&fixture["registration"], fixture["tag"].as_str()?)?;
    let row = &fixture["row"];
    let attachments = strings(&row["attachments"]);
    let own: BTreeSet<String> = strings(&fixture["own"])
        .iter()
        .map(|handle| normalize(handle))
        .collect();
    let authorized = proof.authorizes(
        whole(&row["row_id"])?,
        &attachments,
        &strings(&row["peers"]),
        &own,
    );
    let names: BTreeSet<String> = proof.target_names(&attachments).into_iter().collect();
    let bubbles: Vec<MediaBubble> = fixture["candidates"]
        .as_array()?
        .iter()
        .map(|candidate| {
            let description = candidate["description"].as_str().unwrap_or_default();
            let mut values = strings(&candidate["values"]);
            let outgoing = if description.is_empty() {
                candidate["outgoing"].as_bool()
            } else {
                values.push(description.to_owned());
                media_direction(description)
            };
            MediaBubble { outgoing, values }
        })
        .collect();
    let copies = usize::try_from(whole(&fixture["owned_copies"]).unwrap_or(1))
        .unwrap_or(1)
        .max(1);
    let target = media_target(
        &bubbles,
        &names,
        row["is_from_me"] == true,
        fixture["allow_self_echo"] == true,
        fixture["equivalent_owned"] == true,
        copies,
    );
    Some(serde_json::json!({
        "authorized": authorized,
        "target_verified": authorized && target.is_some(),
    }))
}
