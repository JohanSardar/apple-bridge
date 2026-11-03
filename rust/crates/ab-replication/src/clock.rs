use crate::{Error, Writer, frame, raw};
use ab_trust::wire::messages::FeedFrame;
#[derive(Debug, Default)]
pub(crate) struct Probe {
    pending: Option<(String, jiff::Timestamp)>,
}
impl Probe {
    pub async fn ping(&mut self, writer: &Writer) -> Result<(), Error> {
        let now = jiff::Timestamp::now();
        if self
            .pending
            .as_ref()
            .is_some_and(|(_, t)| now.as_millisecond() - t.as_millisecond() < 15000)
        {
            return Ok(());
        }
        let id = format!("clock_{}", now.as_nanosecond());
        let mut ping = frame("ClockPing");
        ping.id = id.clone();
        ping.clock = Some(raw(
            &serde_json::json!({"t0":now.to_string(),"t1":"0001-01-01T00:00:00Z","t2":"0001-01-01T00:00:00Z","t3":"0001-01-01T00:00:00Z"}),
        )?);
        self.pending = Some((id, now));
        writer.send(ping).await
    }
    pub fn complete(
        &mut self,
        response: &FeedFrame,
        received: jiff::Timestamp,
    ) -> Result<serde_json::Value, Error> {
        let (id, sent) = self.pending.take().ok_or(Error::Invalid)?;
        let mut sample: serde_json::Value =
            serde_json::from_str(response.clock.as_ref().ok_or(Error::Invalid)?.get())?;
        if id != response.id
            || sample["t0"]
                .as_str()
                .ok_or(Error::Invalid)?
                .parse::<jiff::Timestamp>()
                .map_err(|_| Error::Invalid)?
                != sent
        {
            return Err(Error::Invalid);
        }
        let t1 = sample["t1"]
            .as_str()
            .ok_or(Error::Invalid)?
            .parse::<jiff::Timestamp>()
            .map_err(|_| Error::Invalid)?;
        let t2 = sample["t2"]
            .as_str()
            .ok_or(Error::Invalid)?
            .parse::<jiff::Timestamp>()
            .map_err(|_| Error::Invalid)?;
        if t2 < t1
            || received < sent
            || received.as_nanosecond() - sent.as_nanosecond()
                < t2.as_nanosecond() - t1.as_nanosecond()
        {
            return Err(Error::Invalid);
        }
        sample["t3"] = serde_json::json!(received.to_string());
        Ok(sample)
    }
}
