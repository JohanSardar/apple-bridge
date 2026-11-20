use super::support;

use ab_session::{Error, production::Options, readers};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug)]
struct Probe {
    root: PathBuf,
    scanned: mpsc::Sender<String>,
}

impl readers::CaptureReader for Probe {
    fn source(&self) -> &'static str {
        "mail"
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn next(&mut self, _context: &readers::ScanContext) -> Result<Option<Value>, Error> {
        let text = std::fs::read_to_string(self.root.join("state")).unwrap_or_default();
        let _ = self.scanned.send(text);
        Ok(None)
    }

    fn commit(&mut self) -> Result<(), Error> {
        Ok(())
    }

    fn token(&self) -> Option<String> {
        None
    }

    fn resume(&mut self, _token: &str) -> bool {
        false
    }
}

fn scratch() -> Fallible<PathBuf> {
    support::temporary("pacing")
}

#[test]
fn constant_churn_is_paced_and_the_last_change_is_still_captured() -> Fallible<()> {
    let directory = scratch()?;
    std::fs::write(directory.join("state"), "start")?;
    let (scanned, scans) = mpsc::channel();
    let mut sources: BTreeMap<String, Box<dyn readers::CaptureReader>> = BTreeMap::new();
    sources.insert(
        "mail".into(),
        Box::new(Probe {
            root: directory.clone(),
            scanned,
        }),
    );
    let options = Options {
        poll: Duration::from_secs(60),
        watch: true,
        state: None,
        ..Options::default()
    };
    let support::Wire {
        gateway,
        link: helper,
        worker,
        home: _,
    } = support::wire(sources, options, json!({}))?;
    assert_eq!(scans.recv_timeout(Duration::from_secs(4))?, "start");

    let churn_window = Duration::from_millis(3200);
    let churn = {
        let directory = directory.clone();
        std::thread::spawn(move || -> std::io::Result<()> {
            let deadline = Instant::now() + churn_window;
            let mut tick = 0_u64;
            while Instant::now() < deadline {
                tick += 1;
                std::fs::write(directory.join("state"), format!("tick-{tick}"))?;
                std::fs::write(directory.join("swap.tmp"), "x")?;
                std::fs::rename(directory.join("swap.tmp"), directory.join("swap"))?;
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(())
        })
    };
    churn.join().map_err(|_| "churn thread panicked")??;
    let during = scans.try_iter().count();
    assert!(
        during <= 6,
        "{during} scans in a 3.2 s churn window with a 60 s poll"
    );

    std::fs::write(directory.join("state"), "final")?;
    let deadline = Instant::now() + Duration::from_secs(4);
    while scans.recv_timeout(deadline.saturating_duration_since(Instant::now()))? != "final" {}

    drop(gateway);
    drop(helper);
    let _ = worker.join();
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn an_authenticated_restart_reaches_the_helper_before_the_link_closes() -> Fallible<()> {
    let mut wire = support::wire(BTreeMap::new(), Options::default(), json!({}))?;
    support::send(&mut wire.gateway, &json!({"t":"Restart"}))?;
    assert_eq!(support::receive(&mut wire.link)?.1["t"], "Restart");
    drop(wire.gateway);
    drop(wire.link);
    wire.worker.join().map_err(|_| "worker panicked")??;
    Ok(())
}

#[test]
fn usage_watches_only_the_streams_it_reads() -> Fallible<()> {
    let directory = scratch()?;
    let sources = readers::from_config(&json!({"usage":{"root":directory}}).to_string())?;
    let roots = sources.get("usage").ok_or("usage")?.watch_paths();
    assert_eq!(
        roots,
        [
            directory.join("sync"),
            directory.join("streams/restricted/App.InFocus")
        ]
    );
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
