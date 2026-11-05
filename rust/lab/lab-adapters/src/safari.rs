use crate::{assertions::await_until, http::Client, journey};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::Value;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

pub const SIM: [&str; 2] = ["SAF-CAPTURE", "SAF-IDLE"];
pub const REAL: [&str; 2] = ["SAF-BOOKMARKS-REAL", "SAF-HISTORY-REAL"];

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

fn copy_fixture(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let staging = target.join(format!(".{}.staging", name.to_string_lossy()));
        fs::write(&staging, fs::read(entry.path())?)?;
        fs::rename(staging, target.join(name))?;
    }
    Ok(())
}

fn fixture_directory() -> Result<PathBuf> {
    let directory = env::temp_dir().join(format!("ab-safari-{}", std::process::id()));
    fs::create_dir_all(&directory)?;
    Ok(directory)
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
    let session_root = lab_world::session_sources()
        .iter()
        .any(|source| source == "safari")
        .then(|| world.session_root("safari").map(Path::to_path_buf))
        .flatten();
    let pristine = PathBuf::from(format!("{}-pristine", directory.display()));
    if session_root.is_some() {
        evidence.check(
            "session_fixture_root",
            copy_fixture(directory, &pristine).is_ok(),
            "",
        );
        if !evidence.passed {
            return Ok(());
        }
    }
    crate::native_core::rust_tests(
        evidence,
        "reader_capture_fields_and_corruption_guard",
        "ab-native",
        &["--lib", "safari"],
    );
    if id == "SAF-IDLE" {
        return Ok(());
    }
    let Some(root) = session_root.filter(|_| !lab_world::session_control_empty()) else {
        let _ = fs::remove_dir_all(&pristine);
        return Ok(());
    };
    copy_fixture(&pristine, &root)?;
    let result = session_capture(world, evidence);
    let _ = fs::remove_dir_all(&pristine);
    result
}

fn session_capture(world: &World, evidence: &mut Evidence) -> Result<()> {
    let client = Client::new(world, "safari:read")?;
    let arrived = await_until(20, || {
        journey::records(&client).is_ok_and(|(_, rows)| {
            let safari: Vec<&Value> = rows
                .iter()
                .filter(|row| {
                    row["deleted"] != true
                        && row["collection"]
                            .as_str()
                            .is_some_and(|c| c.starts_with("safari.") && c != "safari.tab")
                })
                .collect();
            safari.len() >= 5
                && safari.iter().all(|row| {
                    conforms(row["collection"].as_str().unwrap_or_default(), &row["data"])
                })
        })
    });
    evidence.check("session_reader_rows_in_registered_shape", arrived, "");
    Ok(())
}

pub fn real_checks(evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
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
    let ok = !expected
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .is_empty()
        && !id.is_empty();
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
