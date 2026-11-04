use ab_remote::{
    onboarding::{self, Opener},
    state::{self, Config, Status},
};
use std::{
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

type Check = Result<(), Box<dyn std::error::Error>>;

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| std::io::Error::other("lock"))?
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Buffer {
    fn text(&self) -> String {
        self.0
            .lock()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()
    }
}

fn opener() -> Opener {
    Arc::new(|_| true)
}

async fn publish_after(home: &Path, steps: Vec<Status>) -> ab_remote::Result<()> {
    for _ in 0..200 {
        if let Ok(config) = state::read::<Config>(home, "config") {
            for mut step in steps {
                step.id.clone_from(&config.id);
                state::write(home, "status", &step)?;
                tokio::time::sleep(Duration::from_millis(80)).await;
            }
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err(ab_remote::Error::Contract("config never written"))
}

#[tokio::test]
async fn remote_on_waits_for_on_then_prints_the_url_and_mcp() -> Check {
    let home = tempfile::tempdir()?;
    let publisher = tokio::spawn({
        let home = home.path().to_owned();
        async move {
            publish_after(
                &home,
                vec![
                    Status {
                        state: "starting".into(),
                        ..Status::default()
                    },
                    Status {
                        state: "starting".into(),
                        message: "Waiting for approval".into(),
                        ..Status::default()
                    },
                    Status {
                        state: "on".into(),
                        url: "https://h.example".into(),
                        ..Status::default()
                    },
                ],
            )
            .await
        }
    });
    let mut out = Buffer::default();
    let observed = out.clone();
    onboarding::command(
        home.path(),
        "on",
        "relay",
        &mut out,
        opener(),
        Duration::from_millis(20),
    )
    .await?;
    publisher.await??;
    assert_eq!(
        observed.text(),
        "Waiting for approval\nhttps://h.example\nhttps://h.example/mcp\n"
    );
    Ok(())
}

#[tokio::test]
async fn remote_on_fails_with_the_remote_error() -> Check {
    let home = tempfile::tempdir()?;
    let publisher = tokio::spawn({
        let home = home.path().to_owned();
        async move {
            publish_after(
                &home,
                vec![Status {
                    state: "error".into(),
                    last_error: "relay refused".into(),
                    ..Status::default()
                }],
            )
            .await
        }
    });
    let mut out = Buffer::default();
    let result = onboarding::command(
        home.path(),
        "on",
        "relay",
        &mut out,
        opener(),
        Duration::from_millis(20),
    )
    .await;
    publisher.await??;
    let Err(error) = result else {
        return Err("remote on must fail on an error state".into());
    };
    assert_eq!(error.to_string(), "remote: relay refused");
    Ok(())
}

#[tokio::test]
async fn remote_off_prints_the_state_without_a_url() -> Check {
    let home = tempfile::tempdir()?;
    let publisher = tokio::spawn({
        let home = home.path().to_owned();
        async move {
            publish_after(
                &home,
                vec![Status {
                    state: "off".into(),
                    ..Status::default()
                }],
            )
            .await
        }
    });
    let mut out = Buffer::default();
    let observed = out.clone();
    onboarding::command(
        home.path(),
        "off",
        "relay",
        &mut out,
        opener(),
        Duration::from_millis(20),
    )
    .await?;
    publisher.await??;
    assert_eq!(observed.text(), "off\n");
    Ok(())
}
