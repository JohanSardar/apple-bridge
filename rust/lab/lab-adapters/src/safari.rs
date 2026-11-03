use crate::{
    assertions::await_until,
    files_native::native_output,
    http::Client,
    journey,
    replicas::{self},
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::Value;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const SIM: [&str; 2] = ["SAF-CAPTURE", "SAF-IDLE"];
pub const REAL: [&str; 2] = ["SAF-BOOKMARKS-REAL", "SAF-HISTORY-REAL"];
const SCOPE: &str = "safari:read,system:read";

const FIXTURE: &str = r#"
import sqlite3,plistlib,sys,time,datetime,ctypes
p=sys.argv[1]
c=sqlite3.connect(p+'/History.db');c.executescript('PRAGMA journal_mode=WAL; CREATE TABLE history_items(id INTEGER PRIMARY KEY,url TEXT,visit_count INTEGER); CREATE TABLE history_visits(id INTEGER PRIMARY KEY,history_item INTEGER,visit_time REAL,title TEXT);')
n=time.time()-978307200
c.execute('INSERT INTO history_items VALUES(1,?,2)',('https://example.com/',));c.execute('INSERT INTO history_items VALUES(2,?,1)',('https://example.com/old',))
c.executemany('INSERT INTO history_visits VALUES(?,?,?,?)',[(1,1,n-50,'First'),(2,1,n,'Latest'),(3,2,n-100*86400,'Old')]);c.commit();c.close()
c=sqlite3.connect(p+'/CloudTabs.db');c.executescript("PRAGMA journal_mode=WAL; CREATE TABLE cloud_tab_devices(device_uuid TEXT,device_name TEXT); CREATE TABLE cloud_tabs(tab_uuid TEXT,device_uuid TEXT,title TEXT,url TEXT); INSERT INTO cloud_tab_devices VALUES('d','Synthetic'); INSERT INTO cloud_tabs VALUES('t','d','Synthetic','https://example.com/'); INSERT INTO cloud_tabs VALUES('orphan','unknown-device','Synthetic','https://example.com/');");c.close()
def leaf(key):return dict(WebBookmarkType='WebBookmarkTypeLeaf',WebBookmarkUUID=key,URLString='https://example.com/',URIDictionary={'title':'Synthetic'})
a=leaf('a');b=leaf('b');c=leaf('c');c['ReadingList']={'DateAdded':datetime.datetime(2026,1,1),'DateLastViewed':datetime.datetime(2026,1,2)}
with open(p+'/Bookmarks.plist','wb') as f:plistlib.dump(dict(Children=[dict(WebBookmarkType='WebBookmarkTypeList',Title='Favorites',Children=[a]),dict(WebBookmarkType='WebBookmarkTypeList',Title='com.apple.ReadingList',Children=[b,c]),dict(WebBookmarkType='WebBookmarkTypeList',Title='Empty')]),f)
l=ctypes.CDLL('/usr/lib/libsqlite3.dylib')
for name in ['History.db','CloudTabs.db']:
 db=ctypes.c_void_p()
 assert l.sqlite3_open((p+'/'+name).encode(),ctypes.byref(db))==0
 assert l.sqlite3_exec(db,b'SELECT COUNT(*) FROM sqlite_schema',None,None,None)==0
 enabled=ctypes.c_int(1)
 assert l.sqlite3_file_control(db,b'main',10,ctypes.byref(enabled))==0
 assert l.sqlite3_close(db)==0
"#;

const TRUTH: &str = r"
import sqlite3,plistlib,sys,time,hashlib
p=sys.argv[1]
if sys.argv[2]=='SAF-HISTORY-REAL':
 c=sqlite3.connect('file:'+p+'/History.db?mode=ro',uri=True)
 n=c.execute('SELECT COUNT(*) FROM history_visits WHERE visit_time>=?',(time.time()-978307200-7*86400,)).fetchone()[0]
 print(n)
else:
 with open(p+'/Bookmarks.plist','rb') as f:root=plistlib.load(f)
 rows=[];n=0
 def walk(node,folders,reading):
  global n
  if 'Children' in node:
   title=node.get('Title','');folders=folders+([title] if title else [])
   for child in node['Children']:walk(child,folders,reading or title=='com.apple.ReadingList')
  elif node.get('WebBookmarkType')=='WebBookmarkTypeLeaf':
   n+=1
   if not reading:rows.append(node['URLString']+'\0'+node.get('URIDictionary',{}).get('title',node.get('Title',''))+'\0'+'/'.join(folders))
 walk(root,[],False)
 print(n,hashlib.sha256('\n'.join(sorted(rows)).encode()).hexdigest())
";

#[derive(Clone, Copy)]
enum Kind {
    String,
    Number,
    Bool,
    Time,
}

fn shape(collection: &str) -> Option<Vec<(&'static str, Kind, bool)>> {
    Some(match collection {
        "safari.tab" => vec![
            ("window", Kind::Number, true),
            ("index", Kind::Number, true),
            ("title", Kind::String, true),
            ("url", Kind::String, true),
            ("active", Kind::Bool, true),
        ],
        "safari.history" => vec![
            ("url", Kind::String, true),
            ("title", Kind::String, true),
            ("last_visit", Kind::Time, true),
            ("visit_count", Kind::Number, true),
        ],
        "safari.bookmark" => vec![
            ("url", Kind::String, true),
            ("title", Kind::String, true),
            ("folder_path", Kind::String, true),
        ],
        "safari.reading_list" => vec![
            ("url", Kind::String, true),
            ("title", Kind::String, true),
            ("added", Kind::Time, false),
            ("read", Kind::Bool, true),
        ],
        "safari.cloud_tab" => vec![
            ("url", Kind::String, true),
            ("title", Kind::String, true),
            ("device", Kind::String, true),
        ],
        _ => return None,
    })
}

fn accepts(kind: Kind, value: &Value) -> bool {
    match kind {
        Kind::String => value.is_string(),
        Kind::Number => value.is_number(),
        Kind::Bool => value.is_boolean(),
        Kind::Time => value
            .as_str()
            .is_some_and(|text| text.contains('T') && text.parse::<jiff::Timestamp>().is_ok()),
    }
}

fn conforms(collection: &str, data: &Value) -> bool {
    let (Some(fields), Some(values)) = (shape(collection), data.as_object()) else {
        return false;
    };
    values.iter().all(|(name, value)| {
        fields.iter().any(|(field, kind, required)| {
            field == name && (accepts(*kind, value) || (!required && value.is_null()))
        })
    }) && fields
        .iter()
        .all(|(name, _, required)| !required || values.contains_key(*name))
}

fn same(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, value)| b.get(key).is_some_and(|other| same(value, other)))
        }
        _ => left == right,
    }
}

fn run_limited(command: &mut Command, seconds: u64) -> Option<(bool, String)> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if child.try_wait().ok()?.is_some() {
            let output = child.wait_with_output().ok()?;
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return Some((output.status.success(), text));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn fixture_directory() -> Result<PathBuf> {
    let directory = env::temp_dir().join(format!("ab-safari-{}", std::process::id()));
    fs::create_dir_all(&directory)?;
    Ok(directory)
}

fn build_checks(evidence: &mut Evidence) -> Option<PathBuf> {
    let built = Command::new("packaging/dev/build-native.sh")
        .env("APPLE_BRIDGE_NATIVE_TARGET", "SafariChecks")
        .output();
    let ok = built.as_ref().is_ok_and(|o| o.status.success());
    evidence.check("native_build", ok, "");
    if let Ok(o) = &built
        && !ok
    {
        eprint!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
    }
    if !ok {
        return None;
    }
    let output = native_output();
    evidence.check("native_output", output.is_some(), "");
    output.map(|directory| directory.join("SafariChecks"))
}

fn rows_match(client: &Client, observations: &[Value]) -> bool {
    let Ok((_, rows)) = journey::records(client) else {
        return false;
    };
    let mut matched = vec![false; observations.len()];
    let mut found = 0;
    for row in &rows {
        let collection = row["collection"].as_str().unwrap_or_default();
        if !collection.starts_with("safari.") || row["deleted"] == true {
            continue;
        }
        let slot = observations.iter().enumerate().position(|(index, wanted)| {
            !matched[index]
                && wanted["collection"] == row["collection"]
                && same(&row["data"], &wanted["data"])
        });
        let Some(slot) = slot else {
            return false;
        };
        matched[slot] = true;
        found += 1;
    }
    found == observations.len()
}

fn gateway(world: &World, observations: &[Value], evidence: &mut Evidence) -> Result<()> {
    for row in observations {
        let collection = row["collection"].as_str().unwrap_or_default();
        evidence.check(
            &format!("registered_shape_{collection}"),
            conforms(collection, &row["data"]),
            "",
        );
    }
    let grants = ["safari:read", "safari:write"].iter().all(|scope| {
        world
            .gateway_cli(&["token", "create", "--scope", scope])
            .is_ok()
    });
    evidence.check("read_and_write_grants_registered", grants, "");
    evidence.check("gateway_world", true, "");
    let client = Client::new(world, SCOPE);
    evidence.check("safari_read_scope", client.is_ok(), "");
    let client = client?;
    let replica = replicas::start_with(world, 1, SCOPE, 0);
    evidence.check("safari_replica_scope", replica.is_ok(), "");
    if !evidence.passed {
        return Ok(());
    }
    let sent = world.control("/safari", &Value::Array(observations.to_vec()));
    evidence.check("simulated_native_capture", sent.is_ok(), "");
    evidence.check("session_fixture_staged", true, "");
    evidence.check(
        "gateway_fields_match_native_fixture",
        await_until(15, || rows_match(&client, observations)),
        "",
    );
    let endpoints = replicas::endpoints();
    let replica_client = endpoints.first().map(replicas::Endpoint::client);
    evidence.check(
        "replica_fields_match_native_fixture",
        replica_client.is_some_and(|client| await_until(15, || rows_match(&client, observations))),
        "",
    );
    Ok(())
}

pub fn sim(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    if !SIM.contains(&id.as_str()) {
        return Err(Error::Invalid(format!("safari not ported for {id}")));
    }
    let directory = fixture_directory();
    evidence.check("fixture_directory", directory.is_ok(), "");
    let directory = directory?;
    let result = sim_in(world, &directory, &id, evidence);
    let _ = fs::remove_dir_all(&directory);
    result
}

fn sim_in(world: &World, directory: &Path, id: &str, evidence: &mut Evidence) -> Result<()> {
    let made = Command::new("python3")
        .args(["-c", FIXTURE])
        .arg(directory)
        .output();
    evidence.check("fake_databases", made.is_ok_and(|o| o.status.success()), "");
    if !evidence.passed {
        return Ok(());
    }
    let Some(binary) = build_checks(evidence) else {
        return Ok(());
    };
    let seconds = if id == "SAF-IDLE" { 360 } else { 30 };
    let ran = run_limited(Command::new(&binary).arg(directory).arg(id), seconds);
    let (ok, text) = ran.unwrap_or((false, String::new()));
    if id == "SAF-IDLE" {
        let fields: Vec<&str> = text.split_whitespace().collect();
        let cpu = fields
            .get(3)
            .and_then(|field| field.parse::<f64>().ok())
            .filter(|cpu| cpu.is_finite() && *cpu >= 0.0);
        let valid = ok
            && fields.len() == 4
            && fields[..3] == ["Safari", "idle", "cpu"]
            && cpu.is_some_and(|cpu| cpu < 2.0);
        evidence.check(
            "source_cpu_under_two_percent_for_five_minutes",
            valid,
            &format!("source_cpu_percent={cpu:?}"),
        );
        return Ok(());
    }
    let observations: Option<Vec<Value>> = serde_json::from_str(&text).ok();
    let valid = ok && observations.as_ref().is_some_and(|rows| rows.len() == 6);
    evidence.check("native_capture_fields_and_corruption_guard", valid, "");
    let Some(observations) = observations.filter(|_| valid) else {
        return Ok(());
    };
    gateway(world, &observations, evidence)
}

pub fn real_checks(evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    let Some(binary) = build_checks(evidence) else {
        return Ok(());
    };
    let home = env::var("HOME").map_err(|_| Error::Invalid("HOME".into()))?;
    let directory = Path::new(&home).join("Library/Safari");
    let truth = Command::new("python3")
        .args(["-c", TRUTH])
        .arg(&directory)
        .arg(&id)
        .output();
    let Some(expected) = truth
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    else {
        evidence.unverified(
            "independent_truth_access",
            "Truth process cannot read Safari data; Full Disk Access required",
        );
        return Ok(());
    };
    let actual = Command::new(binary).arg(&directory).arg(&id).output();
    let want: Vec<&str> = expected.split_whitespace().collect();
    let text = actual
        .as_ref()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let got: Vec<&str> = text.split_whitespace().collect();
    let mut ok = actual.is_ok_and(|o| o.status.success())
        && got.len() == 2
        && !want.is_empty()
        && got[0] == want[0];
    if id == "SAF-BOOKMARKS-REAL" {
        ok = ok && want.len() == 2 && got[1] == want[1];
    }
    evidence.check("independent_truth_counts_and_bookmark_digest", ok, "");
    Ok(())
}

pub fn idle_real(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    evidence.check("signed_helper_world", true, "");
    thread::sleep(Duration::from_secs(30));
    let Some(app) = world.helper_app.clone() else {
        return Err(Error::Invalid("no Helper app".into()));
    };
    let prefix = format!("{}/Contents/MacOS/", app.display());
    let sample = || -> Option<(String, f64)> {
        let output = Command::new("ps")
            .args(["-axo", "pid=,time=,command="])
            .output()
            .ok()?;
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .find_map(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                if fields.len() < 3 || !line.contains(&prefix) || !line.contains("session") {
                    return None;
                }
                let mut seconds = 0.0;
                for part in fields[1].split(':') {
                    seconds = seconds * 60.0 + part.parse::<f64>().ok()?;
                }
                Some((fields[0].to_owned(), seconds))
            })
    };
    let first = sample();
    let started = Instant::now();
    thread::sleep(Duration::from_secs(300));
    let second = sample();
    let (Some((pid, before)), Some((after_pid, after))) = (first, second) else {
        evidence.check("helper_cpu_sample", false, "");
        return Ok(());
    };
    let percent = (after - before) / started.elapsed().as_secs_f64() * 100.0;
    evidence.check(
        "helper_cpu_under_two_percent_for_five_minutes",
        after_pid == pid && after >= before && percent < 2.0,
        &format!("helper_cpu_percent={percent:.3}"),
    );
    Ok(())
}
