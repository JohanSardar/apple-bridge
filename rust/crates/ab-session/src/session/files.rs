use super::owned::{Driver, Probe, Refusal, Settled, native_failure, refuse, text};
use crate::staged::digest_of;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::production::FILES_BLOB as BLOB_PREFIX;

const COMMANDS: [&str; 8] = [
    "files.create",
    "files.write",
    "files.move",
    "files.rename",
    "files.trash",
    "files.set_tags",
    "files.download",
    "files.evict",
];
const MAX_CONTENT: usize = 128 << 10;
const MAX_QUERY: usize = 4096;
const SEARCH_OUTPUT_LIMIT: u64 = 1 << 20;
const SEARCH_RESULTS: usize = 100;
const SEARCH_WAIT: Duration = Duration::from_secs(20);
const CLOUD_WAIT: Duration = Duration::from_secs(50);
const CLOUD_POLL: Duration = Duration::from_millis(200);
const MAX_TRANSFORM_INPUTS: usize = 100;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Denied {
    Invalid,
    Unavailable,
}

fn roots(probe: &dyn Probe) -> Result<Vec<PathBuf>, Refusal> {
    let answer = probe
        .call("files.roots", &json!({}))
        .map_err(|code| native_failure(&code))?;
    Ok(answer["roots"]
        .as_array()
        .map(|roots| {
            roots
                .iter()
                .filter_map(Value::as_str)
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default())
}

fn root_of(roots: &[PathBuf], root: &str) -> Result<PathBuf, Denied> {
    let path = Path::new(root);
    if !path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
        return Err(Denied::Invalid);
    }
    let clean: PathBuf = path.components().collect();
    roots
        .iter()
        .find(|candidate| **candidate == clean)
        .cloned()
        .ok_or(Denied::Invalid)
}

pub(super) fn confined(
    roots: &[PathBuf],
    root: &str,
    relative: &str,
    allow_root: bool,
) -> Result<PathBuf, Denied> {
    let root = root_of(roots, root)?;
    if relative.is_empty() {
        return if allow_root {
            Ok(root)
        } else {
            Err(Denied::Invalid)
        };
    }
    if relative.starts_with('/') || relative.contains('\0') {
        return Err(Denied::Invalid);
    }
    let mut url = root.clone();
    for part in relative.split('/') {
        if matches!(part, "" | "." | "..") {
            return Err(Denied::Invalid);
        }
        url.push(part);
        match fs::symlink_metadata(&url) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Err(Denied::Invalid),
            Ok(_) => {}
            Err(_) => return Err(Denied::Unavailable),
        }
    }
    let resolved = url.canonicalize().map_err(|_| Denied::Unavailable)?;
    if resolved != url || !resolved.starts_with(&root) || resolved == root {
        return Err(Denied::Invalid);
    }
    Ok(resolved)
}

fn valid_name(name: &str) -> bool {
    !matches!(name, "" | "." | "..") && !name.contains('/') && !name.contains('\0')
}

pub(super) fn destination(
    roots: &[PathBuf],
    root: &str,
    parent: &str,
    name: &str,
) -> Result<PathBuf, Denied> {
    let parent = confined(roots, root, parent, true)?;
    if !valid_name(name) {
        return Err(Denied::Invalid);
    }
    let url = parent.join(name);
    if fs::symlink_metadata(&url).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(Denied::Invalid);
    }
    Ok(url)
}

fn refused(denied: &Denied) -> Refusal {
    refuse(match denied {
        Denied::Invalid => "invalid",
        Denied::Unavailable => "unavailable",
    })
}

fn content(args: &Value) -> Result<Vec<u8>, Refusal> {
    let bytes = STANDARD
        .decode(text(args, "content_base64"))
        .map_err(|_| refuse("invalid"))?;
    if bytes.len() > MAX_CONTENT || digest_of(&bytes) != text(args, "digest") {
        return Err(refuse("invalid"));
    }
    Ok(bytes)
}

fn checked(probe: &dyn Probe, action: &Value) -> Result<Vec<PathBuf>, Refusal> {
    let operation = text(action, "op");
    if operation == "files.transform" {
        return roots(probe);
    }
    if !COMMANDS.contains(&operation) {
        return Err(refuse("unsupported"));
    }
    let args = &action["args"];
    let roots = roots(probe)?;
    let root = text(args, "root");
    if matches!(
        operation,
        "files.create" | "files.write" | "files.move" | "files.rename"
    ) {
        destination(&roots, root, text(args, "parent"), text(args, "name"))
            .map_err(|denied| refused(&denied))?;
    }
    if matches!(operation, "files.create" | "files.write") {
        content(args)?;
    } else {
        confined(&roots, root, text(args, "path"), false).map_err(|denied| refused(&denied))?;
    }
    Ok(roots)
}

fn prepare(probe: &dyn Probe, action: &Value, _expected: &Value) -> Result<Value, Refusal> {
    checked(probe, action)?;
    Ok(Value::Null)
}

fn validate(probe: &dyn Probe, action: &Value, _state: &Value) -> Result<(), Refusal> {
    checked(probe, action).map(|_| ())
}

fn code_of(error: &std::io::Error) -> String {
    match error.kind() {
        ErrorKind::AlreadyExists | ErrorKind::NotFound => "conflict",
        _ => "unavailable",
    }
    .to_owned()
}

fn create_file(url: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(ab_sys::O_NOFOLLOW)
        .open(url)
        .map_err(|error| code_of(&error))?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    if let Err(error) = written {
        let _ = fs::remove_file(url);
        return Err(code_of(&error));
    }
    Ok(())
}

fn replace_file(url: &Path, bytes: &[u8]) -> Result<(), String> {
    let metadata = fs::symlink_metadata(url).map_err(|error| code_of(&error))?;
    if !metadata.is_file() {
        return Err("conflict".into());
    }
    let parent = url.parent().ok_or("invalid")?;
    let name = url.file_name().unwrap_or_else(|| OsStr::new("file"));
    let temporary = parent.join(format!(
        ".{}.ab-{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(ab_sys::O_NOFOLLOW)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.set_permissions(fs::Permissions::from_mode(metadata.permissions().mode()))?;
        file.sync_all()?;
        fs::rename(&temporary, url)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(code_of(&error));
    }
    Ok(())
}

fn path_text(url: &Path) -> String {
    url.to_string_lossy().into_owned()
}

fn call(probe: &dyn Probe, operation: &str, path: &Path, more: &Value) -> Result<Value, String> {
    let mut args = json!({"path": path_text(path)});
    if let (Some(fields), Some(extra)) = (args.as_object_mut(), more.as_object()) {
        fields.extend(extra.clone());
    }
    probe.call(operation, &args)
}

fn await_cloud(
    probe: &dyn Probe,
    url: &Path,
    mut settled: impl FnMut(&Value) -> Result<bool, String>,
) -> Result<(), String> {
    let deadline = Instant::now() + CLOUD_WAIT;
    loop {
        let state = call(probe, "files.cloud_state", url, &json!({}))?;
        if settled(&state)? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("unavailable".into());
        }
        thread::sleep(CLOUD_POLL);
    }
}

fn cloud(probe: &dyn Probe, operation: &str, url: &Path) -> Result<(), String> {
    let state = call(probe, "files.cloud_state", url, &json!({}))?;
    if state["ubiquitous"] != true {
        return Err("unsupported".into());
    }
    if operation == "files.download" {
        call(probe, "files.start_download", url, &json!({}))?;
    } else {
        await_cloud(probe, url, |state| {
            if state["upload_error"] == true {
                return Err("unavailable".to_owned());
            }
            Ok(state["uploaded"] == true)
        })?;
        call(probe, "files.evict_item", url, &json!({}))?;
    }
    let wanted = operation == "files.download";
    await_cloud(probe, url, |state| {
        let status = text(state, "status");
        if status.is_empty() {
            return Err("unavailable".to_owned());
        }
        Ok((status != "NSURLUbiquitousItemDownloadingStatusNotDownloaded") == wanted)
    })
}

fn transform(probe: &dyn Probe, args: &Value) -> Result<Value, String> {
    let blobs = probe.staged(BLOB_PREFIX).ok_or("unavailable")?;
    let inputs = args["inputs"].as_array().ok_or("invalid")?;
    if inputs.is_empty() || inputs.len() > MAX_TRANSFORM_INPUTS {
        return Err("invalid".into());
    }
    let mut paths = Vec::new();
    for input in inputs {
        let path = blobs
            .path_of(input.as_str().ok_or("invalid")?)
            .map_err(|_| "unavailable")?
            .ok_or("invalid")?;
        paths.push(path_text(&path));
    }
    let mut request = args.clone();
    request["inputs"] = json!(paths);
    request["directory"] = json!(path_text(blobs.directory()));
    let answer = probe.call("files.transform", &request)?;
    let mut outputs = Vec::new();
    for output in answer["outputs"].as_array().ok_or("corrupt")? {
        let digest = text(output, "digest");
        blobs
            .remember(digest, PathBuf::from(text(output, "file")))
            .map_err(|_| "corrupt")?;
        outputs.push(json!({
            "digest": digest,
            "size": output["size"],
            "handle": blobs.handle(digest),
            "media_type": output["media_type"],
        }));
    }
    Ok(json!({"observed": true, "outputs": outputs}))
}

fn write(
    probe: &dyn Probe,
    _command: &str,
    action: &Value,
    _state: &Value,
) -> Result<Value, String> {
    let operation = text(action, "op");
    if operation == "files.transform" {
        return transform(probe, &action["args"]);
    }
    let roots = roots(probe).map_err(|refusal| refusal.code)?;
    let args = &action["args"];
    let root = text(args, "root");
    let denied = |denied: Denied| {
        match &denied {
            Denied::Invalid => "invalid",
            Denied::Unavailable => "unavailable",
        }
        .to_owned()
    };
    let target =
        || destination(&roots, root, text(args, "parent"), text(args, "name")).map_err(denied);
    if matches!(operation, "files.create" | "files.write") {
        let url = target()?;
        let bytes = content(args).map_err(|refusal| refusal.code)?;
        if operation == "files.create" {
            create_file(&url, &bytes)?;
        } else {
            replace_file(&url, &bytes)?;
        }
        let read = fs::read(&url).map_err(|error| code_of(&error))?;
        if read != bytes {
            return Err("corrupt".into());
        }
        return Ok(
            json!({"observed": true, "path": path_text(&url), "native_key": path_text(&url), "digest": digest_of(&read)}),
        );
    }
    let url = confined(&roots, root, text(args, "path"), false).map_err(denied)?;
    match operation {
        "files.move" | "files.rename" => {
            let goal = target()?;
            ab_sys::rename_exclusive(&url, &goal).map_err(|error| code_of(&error))?;
            if fs::symlink_metadata(&url).is_ok() || fs::symlink_metadata(&goal).is_err() {
                return Err("corrupt".into());
            }
            Ok(json!({"observed": true, "path": path_text(&goal), "native_key": path_text(&goal)}))
        }
        "files.trash" => {
            let answer = call(probe, "files.trash_item", &url, &json!({}))?;
            let trash = text(&answer, "trash_path");
            let inside = Path::new(trash).components().any(|part| {
                matches!(part, Component::Normal(name) if name == ".Trash" || name == ".Trashes")
            });
            if !inside || fs::symlink_metadata(trash).is_err() || fs::symlink_metadata(&url).is_ok()
            {
                return Err("corrupt".into());
            }
            Ok(
                json!({"observed": true, "trashed": true, "native_key": path_text(&url), "trash_path": trash}),
            )
        }
        "files.set_tags" => {
            let answer = call(
                probe,
                "files.set_tags",
                &url,
                &json!({"tags": args["tags"]}),
            )?;
            let wanted: BTreeSet<&str> = args["tags"]
                .as_array()
                .map(|tags| tags.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let actual: BTreeSet<&str> = answer["tags"]
                .as_array()
                .map(|tags| tags.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            if wanted != actual {
                return Err("corrupt".into());
            }
            Ok(
                json!({"observed": true, "path": path_text(&url), "native_key": path_text(&url), "tags": answer["tags"]}),
            )
        }
        "files.download" | "files.evict" => {
            cloud(probe, operation, &url)?;
            Ok(
                json!({"observed": true, "path": path_text(&url), "native_key": path_text(&url), "downloaded": operation == "files.download"}),
            )
        }
        _ => Err("unsupported".into()),
    }
}

#[allow(clippy::unnecessary_wraps)]
fn settle(_operation: &str, written: &Value) -> Option<Settled> {
    if written["observed"] != true {
        return None;
    }
    Some(Settled {
        disposition: "locally_observed",
        evidence: written.clone(),
    })
}

fn resolved(_probe: &dyn Probe, _evidence: &Value) -> Option<Value> {
    None
}

fn reconcile_args(_found: &Value, _operation: &str) -> Value {
    json!({})
}

pub(super) const DRIVER: Driver = Driver {
    name: "files",
    prepare,
    prepare_command: None,
    validate,
    write,
    settle,
    resolved,
    reconcile_args,
};

fn search_predicate(args: &Value) -> Result<String, &'static str> {
    let query = text(args, "query");
    if query.is_empty() || query.len() > MAX_QUERY {
        return Err("invalid");
    }
    let attribute = match text(args, "field") {
        "" | "name" => "kMDItemFSName",
        "content" => "kMDItemTextContent",
        "kind" => "kMDItemContentType",
        "tag" => "kMDItemUserTags",
        "date" => "kMDItemFSContentChangeDate",
        _ => return Err("invalid"),
    };
    let value = query.replace('\\', "\\\\").replace('"', "\\\"");
    Ok(format!("{attribute} == \"{value}\"cd"))
}

fn run_mdfind(roots: &[PathBuf], predicate: &str) -> Result<Vec<u8>, &'static str> {
    let directory = tempdir().map_err(|_| "unavailable")?;
    let output = directory.join("stdout");
    let handle = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&output)
        .map_err(|_| "unavailable")?;
    let mut command = Command::new("/usr/bin/mdfind");
    command.arg("-0");
    for root in roots {
        command.arg("-onlyin").arg(root);
    }
    let mut child = command
        .arg(predicate)
        .stdin(Stdio::null())
        .stdout(handle)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "unavailable")?;
    let deadline = Instant::now() + SEARCH_WAIT;
    let result = loop {
        if let Some(status) = child.try_wait().map_err(|_| "unavailable")? {
            break if status.success() {
                Ok(())
            } else {
                Err("unavailable")
            };
        }
        let size = fs::metadata(&output).map_or(0, |metadata| metadata.len());
        if Instant::now() >= deadline || size >= SEARCH_OUTPUT_LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            break Err("unavailable");
        }
        thread::sleep(Duration::from_millis(50));
    };
    let bytes = result.and_then(|()| fs::read(&output).map_err(|_| "unavailable"));
    let _ = fs::remove_dir_all(&directory);
    bytes
}

fn tempdir() -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let path = std::env::temp_dir().join(format!(
        "ab-files-{}-{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos())
    ));
    fs::DirBuilder::new().mode(0o700).create(&path)?;
    Ok(path)
}

fn search_items(roots: &[PathBuf], output: &[u8]) -> Vec<Value> {
    output
        .split(|byte| *byte == 0)
        .take(SEARCH_RESULTS)
        .filter_map(|path| std::str::from_utf8(path).ok())
        .filter(|path| {
            let path = Path::new(path);
            !path.components().any(|part| part == Component::ParentDir)
                && roots
                    .iter()
                    .any(|root| path.starts_with(root) && path != root)
        })
        .map(|path| json!({"path": path}))
        .collect()
}

fn search(probe: &dyn Probe, args: &Value) -> Result<Value, String> {
    let predicate = search_predicate(args).map_err(str::to_owned)?;
    let roots = roots(probe).map_err(|refusal| refusal.code)?;
    let output = run_mdfind(&roots, &predicate).map_err(str::to_owned)?;
    Ok(json!({"items": search_items(&roots, &output)}))
}

fn readable(roots: &[PathBuf], root: &str, relative: &str) -> Result<PathBuf, Denied> {
    let root = root_of(roots, root)?;
    if relative.is_empty() || relative.starts_with('/') || relative.contains('\0') {
        return Err(Denied::Invalid);
    }
    let resolved = root
        .join(relative)
        .canonicalize()
        .map_err(|_| Denied::Unavailable)?;
    if resolved == root || !resolved.starts_with(&root) {
        return Err(Denied::Invalid);
    }
    Ok(resolved)
}

fn read(probe: &dyn Probe, args: &Value) -> Result<(Value, Value), String> {
    let roots = roots(probe).map_err(|refusal| refusal.code)?;
    let url = readable(&roots, text(args, "root"), text(args, "path")).map_err(|denied| {
        match denied {
            Denied::Invalid => "invalid",
            Denied::Unavailable => "unavailable",
        }
        .to_owned()
    })?;
    let state = call(probe, "files.cloud_state", &url, &json!({}))?;
    if state["ubiquitous"] == true {
        call(probe, "files.start_download", &url, &json!({}))?;
        await_cloud(probe, &url, |state| {
            let status = text(state, "status");
            if status.is_empty() {
                return Err("unavailable".to_owned());
            }
            Ok(status != "NSURLUbiquitousItemDownloadingStatusNotDownloaded")
        })?;
    }
    let blobs = probe.staged(BLOB_PREFIX).ok_or("unavailable")?;
    let staged = call(
        probe,
        "files.stage",
        &url,
        &json!({"directory": path_text(blobs.directory())}),
    )?;
    let digest = text(&staged, "digest").to_owned();
    blobs
        .remember(&digest, PathBuf::from(text(&staged, "file")))
        .map_err(|_| "unavailable".to_owned())?;
    let size = staged["size"].clone();
    Ok((
        json!({"digest": digest, "size": size}),
        json!([{"digest": digest, "size": size, "handle": blobs.handle(&digest)}]),
    ))
}

#[must_use]
pub(super) fn handles_query(operation: &str) -> bool {
    matches!(operation, "files.search" | "files.read")
}

pub(super) fn query(probe: &dyn Probe, frame: &Value) -> Value {
    let id = &frame["id"];
    let answer = match text(frame, "op") {
        "files.search" => search(probe, &frame["args"]).map(|result| (result, json!([]))),
        _ => read(probe, &frame["args"]),
    };
    match answer {
        Ok((result, blobs)) => {
            json!({"t": "QueryResult", "id": id, "result_json": result, "blobs": blobs})
        }
        Err(code) => json!({"t": "QueryError", "id": id, "code": code}),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps
)]
mod tests {
    use super::*;
    use std::{cell::RefCell, os::unix::fs::symlink};

    struct World {
        base: PathBuf,
        root: PathBuf,
        outside: PathBuf,
    }

    impl Drop for World {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    fn world(name: &str) -> World {
        let base = std::env::temp_dir()
            .join(format!("ab-files-test-{}-{name}", std::process::id()))
            .canonicalize()
            .unwrap_or_else(|_| {
                let path = std::env::temp_dir()
                    .join(format!("ab-files-test-{}-{name}", std::process::id()));
                fs::create_dir_all(&path).unwrap();
                path.canonicalize().unwrap()
            });
        let root = base.join("root");
        let outside = base.join("outside");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret"), "s").unwrap();
        fs::write(root.join("sub/a.txt"), "a").unwrap();
        symlink(&outside, root.join("away")).unwrap();
        symlink(outside.join("secret"), root.join("away-file")).unwrap();
        World {
            base,
            root,
            outside,
        }
    }

    struct Probed {
        roots: Vec<PathBuf>,
        calls: RefCell<Vec<String>>,
    }

    impl Probe for Probed {
        fn call(&self, operation: &str, args: &Value) -> Result<Value, String> {
            if operation == "files.roots" {
                let roots: Vec<String> = self.roots.iter().map(|root| path_text(root)).collect();
                return Ok(json!({"roots": roots}));
            }
            self.calls
                .borrow_mut()
                .push(format!("{operation} {}", text(args, "path")));
            Ok(json!({}))
        }
    }

    fn probe(world: &World) -> Probed {
        Probed {
            roots: vec![world.root.clone()],
            calls: RefCell::default(),
        }
    }

    fn action(op: &str, args: &Value) -> Value {
        json!({"op": op, "args": args})
    }

    fn body(text: &str) -> Value {
        json!({"content_base64": STANDARD.encode(text), "digest": digest_of(text.as_bytes())})
    }

    #[test]
    fn nothing_outside_a_configured_root_is_reachable() {
        let world = world("escape");
        let root = path_text(&world.root);
        let roots = [world.root.clone()];
        for relative in [
            "../outside/secret",
            "sub/../../outside/secret",
            "/etc/passwd",
            "./sub",
            "sub//a.txt",
            "sub/.",
            "sub/a.txt/",
        ] {
            assert_eq!(
                confined(&roots, &root, relative, false),
                Err(Denied::Invalid),
                "{relative}"
            );
        }
        for relative in ["away", "away/secret", "away-file"] {
            assert_eq!(
                confined(&roots, &root, relative, false),
                Err(Denied::Invalid),
                "{relative}"
            );
        }
        assert_eq!(confined(&roots, &root, "", false), Err(Denied::Invalid));
        assert_eq!(
            confined(&roots, &root, "missing", false),
            Err(Denied::Unavailable)
        );
        assert_eq!(
            confined(&roots, &path_text(&world.outside), "secret", false),
            Err(Denied::Invalid)
        );
        assert_eq!(
            confined(&roots, &format!("{root}/../outside"), "secret", false),
            Err(Denied::Invalid)
        );
        assert_eq!(
            confined(&roots, "relative/root", "secret", false),
            Err(Denied::Invalid)
        );
        assert_eq!(
            confined(&roots, &root, "sub/a.txt", false).unwrap(),
            world.root.join("sub/a.txt")
        );
        assert_eq!(confined(&roots, &root, "", true).unwrap(), world.root);
    }

    #[test]
    fn a_destination_stays_in_its_parent() {
        let world = world("destination");
        let root = path_text(&world.root);
        let roots = [world.root.clone()];
        for name in ["", ".", "..", "a/b", "../x", "x\0y"] {
            assert_eq!(
                destination(&roots, &root, "sub", name),
                Err(Denied::Invalid),
                "{name}"
            );
        }
        assert_eq!(
            destination(&roots, &root, "away", "x"),
            Err(Denied::Invalid)
        );
        assert_eq!(
            destination(&roots, &root, "", "away-file"),
            Err(Denied::Invalid)
        );
        assert_eq!(
            destination(&roots, &root, "", "new").unwrap(),
            world.root.join("new")
        );
    }

    #[test]
    fn create_write_move_and_rename_work_inside_and_never_replace_by_accident() {
        let world = world("effects");
        let probe = probe(&world);
        let root = path_text(&world.root);
        let mut args = json!({"root": root, "parent": "sub", "name": "n.txt"});
        args.as_object_mut()
            .unwrap()
            .extend(body("one").as_object().unwrap().clone());
        let made = write(
            &probe,
            "c",
            &action("files.create", &args.clone()),
            &Value::Null,
        )
        .unwrap();
        let file = world.root.join("sub/n.txt");
        assert_eq!(made["path"], path_text(&file));
        assert_eq!(made["digest"], digest_of(b"one"));
        assert_eq!(fs::read(&file).unwrap(), b"one");
        assert_eq!(
            write(
                &probe,
                "c",
                &action("files.create", &args.clone()),
                &Value::Null
            ),
            Err("conflict".to_owned())
        );
        let mut update = json!({"root": root, "parent": "sub", "name": "n.txt"});
        update
            .as_object_mut()
            .unwrap()
            .extend(body("two").as_object().unwrap().clone());
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
        write(&probe, "c", &action("files.write", &update), &Value::Null).unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"two");
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let mut missing = json!({"root": root, "parent": "sub", "name": "none.txt"});
        missing
            .as_object_mut()
            .unwrap()
            .extend(body("x").as_object().unwrap().clone());
        assert_eq!(
            write(&probe, "c", &action("files.write", &missing), &Value::Null),
            Err("conflict".to_owned())
        );
        let moved = action(
            "files.move",
            &json!({"root": root, "path": "sub/n.txt", "parent": "", "name": "m.txt"}),
        );
        write(&probe, "c", &moved, &Value::Null).unwrap();
        assert!(!file.exists() && world.root.join("m.txt").exists());
        fs::write(world.root.join("taken"), "keep").unwrap();
        let onto = action(
            "files.rename",
            &json!({"root": root, "path": "m.txt", "parent": "", "name": "taken"}),
        );
        assert_eq!(
            write(&probe, "c", &onto, &Value::Null),
            Err("conflict".to_owned())
        );
        assert_eq!(fs::read(world.root.join("taken")).unwrap(), b"keep");
        assert!(world.root.join("m.txt").exists());
    }

    #[test]
    fn a_digest_or_size_mismatch_refuses_before_anything_is_written() {
        let world = world("content");
        let probe = probe(&world);
        let root = path_text(&world.root);
        let wrong = action(
            "files.create",
            &json!({"root": root, "parent": "", "name": "w", "content_base64": STANDARD.encode("x"), "digest": digest_of(b"y")}),
        );
        assert_eq!(
            prepare(&probe, &wrong, &Value::Null)
                .expect_err("refused")
                .code,
            "invalid"
        );
        let large = "x".repeat(MAX_CONTENT + 1);
        let big = action(
            "files.create",
            &json!({"root": root, "parent": "", "name": "w", "content_base64": STANDARD.encode(&large), "digest": digest_of(large.as_bytes())}),
        );
        assert_eq!(
            prepare(&probe, &big, &Value::Null)
                .expect_err("refused")
                .code,
            "invalid"
        );
        assert!(!world.root.join("w").exists());
    }

    #[test]
    fn framework_actions_only_receive_a_confined_path() {
        let world = world("framework");
        let probe = probe(&world);
        let root = path_text(&world.root);
        for op in [
            "files.trash",
            "files.set_tags",
            "files.download",
            "files.evict",
        ] {
            let escape = action(
                op,
                &json!({"root": root, "path": "away/secret", "tags": []}),
            );
            assert_eq!(
                prepare(&probe, &escape, &Value::Null)
                    .expect_err("refused")
                    .code,
                "invalid"
            );
            assert_eq!(
                write(&probe, "c", &escape, &Value::Null),
                Err("invalid".to_owned())
            );
        }
        assert!(probe.calls.borrow().is_empty());
        let other = action("files.chmod", &json!({"root": root, "path": "sub/a.txt"}));
        assert_eq!(
            prepare(&probe, &other, &Value::Null)
                .expect_err("refused")
                .code,
            "unsupported"
        );
    }

    #[test]
    fn a_trash_path_outside_a_trash_folder_is_not_proof() {
        struct Lying(Vec<PathBuf>);
        impl Probe for Lying {
            fn call(&self, operation: &str, _args: &Value) -> Result<Value, String> {
                if operation == "files.roots" {
                    let roots: Vec<String> = self.0.iter().map(|root| path_text(root)).collect();
                    return Ok(json!({"roots": roots}));
                }
                Ok(json!({"trash_path": "/tmp/elsewhere"}))
            }
        }
        let world = world("trash");
        let probe = Lying(vec![world.root.clone()]);
        let action = action(
            "files.trash",
            &json!({"root": path_text(&world.root), "path": "sub/a.txt"}),
        );
        assert_eq!(
            write(&probe, "c", &action, &Value::Null),
            Err("corrupt".to_owned())
        );
    }

    #[test]
    fn a_read_may_follow_a_link_only_inside_the_root() {
        let world = world("read");
        symlink(world.root.join("sub/a.txt"), world.root.join("inner")).unwrap();
        let root = path_text(&world.root);
        let roots = [world.root.clone()];
        assert!(readable(&roots, &root, "inner").is_ok());
        assert_eq!(readable(&roots, &root, "away/secret"), Err(Denied::Invalid));
        assert_eq!(
            readable(&roots, &root, "../outside/secret"),
            Err(Denied::Invalid)
        );
        assert_eq!(readable(&roots, &root, "/etc/passwd"), Err(Denied::Invalid));
        assert_eq!(readable(&roots, &root, ""), Err(Denied::Invalid));
    }

    #[test]
    fn a_search_cannot_break_out_of_its_predicate_or_return_outside_paths() {
        assert_eq!(
            search_predicate(&json!({"query": "a\"b\\c"})).unwrap(),
            "kMDItemFSName == \"a\\\"b\\\\c\"cd"
        );
        assert_eq!(
            search_predicate(&json!({"query": "x", "field": "tag"})).unwrap(),
            "kMDItemUserTags == \"x\"cd"
        );
        assert!(search_predicate(&json!({"query": ""})).is_err());
        assert!(search_predicate(&json!({"query": "x".repeat(MAX_QUERY + 1)})).is_err());
        assert!(search_predicate(&json!({"query": "x", "field": "other"})).is_err());
        let roots = [PathBuf::from("/r/root")];
        let output = b"/r/root/a\0/r/rootish/b\0/etc/passwd\0/r/root/../x\0/r/root\0";
        assert_eq!(
            search_items(&roots, output),
            vec![json!({"path": "/r/root/a"})]
        );
    }
}
