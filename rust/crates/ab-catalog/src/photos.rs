use crate::{OperationDescriptor, OperationKind};
use serde_json::{Value, json};

pub fn descriptors(output: &Value) -> Vec<OperationDescriptor> {
    let text = json!({"type":"string","minLength":1,"maxLength":1024});
    let title = json!({"type":"string","minLength":1,"maxLength":256});
    let boolean = json!({"type":"boolean"});
    let offset = json!({"type":"integer","minimum":0,"maximum":100_000_000});
    let limit = json!({"type":"integer","minimum":1,"maximum":200});
    let transfer = json!({"type":"string","minLength":1,"maxLength":1024});
    let mut operations = Vec::new();
    let mut entries = vec![
        (
            "photos.search",
            "Live PhotoKit search with bounded fetches. Date/type/album filters run in PhotoKit; coordinate-radius filtering examines at most 1000 candidates per page. examined_count is this page's candidates, not a library total. Follow next_cursor, even on empty pages, keeping the same filters. No library metadata is stored. Pages are live, not snapshots.",
            false,
            json!({"start":text,"end":text,"type":{"type":"string","enum":["image","video","audio","live_photo","screenshot","panorama","hdr","depth","slow_motion","timelapse"]},"album_id":text,"favorite":boolean,"include_hidden":boolean,"near":{"type":"object","additionalProperties":false,"required":["lat","lon"],"properties":{"lat":{"type":"number","minimum":-90,"maximum":90},"lon":{"type":"number","minimum":-180,"maximum":180}}},"radius_m":{"type":"number","exclusiveMinimum":0,"maximum":100_000},"cursor":{"type":"string","minLength":1,"maxLength":2048},"limit":limit}),
            vec![],
        ),
        (
            "photos.get",
            "Live asset details, location and all public PhotoKit resources, including video, paired Live Photo video, adjustment data and edited/original representations. Resource indexes are local to this response.",
            false,
            json!({"asset_id":text}),
            vec!["asset_id"],
        ),
        (
            "photos.albums",
            "Page through user albums or smart albums, with PhotoKit editing capabilities. No album index is stored.",
            false,
            json!({"offset":offset,"limit":limit,"smart":boolean}),
            vec![],
        ),
        (
            "photos.fetch",
            "Start a temporary original-resource transfer with asset_id, optional resource_index, resource_key from photos.get to detect changed metadata, and network_access (default false, allows iCloud downloads). Poll/read with transfer_id/offset/length only. Returns downloading or ready with SHA-256, size and base64 bytes. Read at most 512 KiB per call. Default cap 8 GiB, configurable up to 64 GiB, subject to disk space. Four transfers; poll within 120 seconds to retain one, with a 24-hour hard lifetime. Release explicitly. Never put photo bytes into Bridge blobs or replication.",
            false,
            json!({"asset_id":text,"resource_index":{"type":"integer","minimum":0,"maximum":2_147_483_647},"resource_key":{"type":"string","minLength":64,"maxLength":64},"network_access":boolean,"transfer_id":transfer,"offset":{"type":"integer","minimum":0,"maximum":68_719_476_736_u64},"length":{"type":"integer","minimum":1,"maximum":524_288},"max_bytes":{"type":"integer","minimum":1,"maximum":68_719_476_736_u64}}),
            vec![],
        ),
        (
            "photos.release",
            "Cancel and delete this principal's temporary Photos transfer. Expired or foreign transfer IDs fail closed.",
            false,
            json!({"transfer_id":transfer}),
            vec!["transfer_id"],
        ),
        (
            "photos.favorite",
            "Set an asset's favorite flag and read it back. Transient write that runs at once unless the owner set this agent to ask first; then it returns awaiting_approval and the identical request ID is retried once after approval. Completed or ambiguous IDs never redispatch. Only a request digest, not asset details, is retained.",
            true,
            json!({"asset_id":text,"favorite":boolean}),
            vec!["asset_id", "favorite"],
        ),
        (
            "photos.album_create",
            "Create a regular user album. Transient write. An agent set to ask first gets a hash-only approval and reissues the identical ID after it. No automatic retries.",
            true,
            json!({"title":title}),
            vec!["title"],
        ),
        (
            "photos.album_rename",
            "Rename an editable regular album. Transient write; asks the owner first only for an agent set to ask first. No automatic retry.",
            true,
            json!({"album_id":text,"title":title}),
            vec!["album_id", "title"],
        ),
        (
            "photos.album_add",
            "Add up to 200 existing assets to an editable regular album. Does not import or copy media. Transient write; asks the owner first only for an agent set to ask first.",
            true,
            json!({"album_id":text,"asset_ids":{"type":"array","minItems":1,"maxItems":200,"uniqueItems":true,"items":text}}),
            vec!["album_id", "asset_ids"],
        ),
        (
            "photos.album_remove",
            "Remove up to 200 memberships from an editable regular album, never from the library. Transient write; asks the owner first only for an agent set to ask first.",
            true,
            json!({"album_id":text,"asset_ids":{"type":"array","minItems":1,"maxItems":200,"uniqueItems":true,"items":text}}),
            vec!["album_id", "asset_ids"],
        ),
        (
            "photos.album_delete",
            "Delete an editable regular album, never its assets. Transient write; asks the owner first only for an agent set to ask first. Asset deletion and permanent deletion are unavailable.",
            true,
            json!({"album_id":text}),
            vec!["album_id"],
        ),
    ];
    let viewing = viewing(&text, &boolean);
    entries.splice(3..3, viewing);
    for (name, description, write, properties, required) in entries {
        operations.push(descriptor(
            output,
            name,
            description,
            write,
            &properties,
            &required,
        ));
    }
    operations
}

type Entry = (&'static str, &'static str, bool, Value, Vec<&'static str>);

fn viewing(text: &Value, boolean: &Value) -> [Entry; 2] {
    [
        (
            "photos.preview",
            "Look at one photo. Renders a JPEG preview on demand, longest side max_px (64-1536, default 512), and returns it as an image the agent can view; REST callers get image_base64. Video assets return a still frame. Hidden assets need include_hidden. network_access (default true) lets PhotoKit download this one original from the owner's iCloud when the Mac keeps only a thumbnail; set false to use local data only. Each call sends this photo's pixels to the caller; nothing is cached or sent ahead of time. Narrow with photos.search first and preview only what you need.",
            false,
            json!({"asset_id":text,"max_px":{"type":"integer","minimum":64,"maximum":1536},"network_access":boolean,"include_hidden":boolean}),
            vec!["asset_id"],
        ),
        (
            "photos.contact_sheet",
            "Scan many photos at once. Renders up to 30 asset_ids from photos.search into one JPEG grid of numbered thumbnails (columns 1-8, default up to 5; cell_px 96-384, default 256) and returns it as an image the agent can view. cells maps each number to its asset_id; open a match larger with photos.preview. Hidden assets need include_hidden; network_access (default false) allows iCloud downloads, and cells that are not local otherwise show grey with rendered false. Pixels leave the Mac only for the photos named in this call.",
            false,
            json!({"asset_ids":{"type":"array","minItems":1,"maxItems":30,"uniqueItems":true,"items":text},"columns":{"type":"integer","minimum":1,"maximum":8},"cell_px":{"type":"integer","minimum":96,"maximum":384},"network_access":boolean,"include_hidden":boolean}),
            vec!["asset_ids"],
        ),
    ]
}

fn descriptor(
    output: &Value,
    name: &str,
    description: &str,
    write: bool,
    properties: &Value,
    required: &[&str],
) -> OperationDescriptor {
    OperationDescriptor {
        name: name.into(),
        version: 1,
        kind: OperationKind::Query,
        read_model: false,
        source: Some(ab_domain::Source::Photos),
        scope: if write { "photos:write" } else { "photos:read" }.into(),
        capability: String::new(),
        description: description.into(),
        input: json!({"type":"object","additionalProperties":false,"required":if write {vec!["id", "args"]} else {vec!["args"]},"properties":{
            "id":{"type":"string","pattern":"^qry_[A-Za-z0-9._:-]+$","maxLength":128},
            "deadline_ms":{"type":"integer","minimum":0,"maximum":60_000},
            "args":{"type":"object","additionalProperties":false,"properties":properties,"required":required}
        }}),
        output: json!({"oneOf":[output,{"type":"object","additionalProperties":false,"required":["id","state"],"properties":{"id":{"type":"string"},"state":{"enum":["awaiting_approval"]}}}]}),
        examples: None,
    }
}
