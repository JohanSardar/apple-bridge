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

#[derive(serde::Deserialize, serde::Serialize)]
struct Sample {
    t0: ab_trust::wire::gojson::GoTime,
    t1: ab_trust::wire::gojson::GoTime,
    t2: ab_trust::wire::gojson::GoTime,
    t3: ab_trust::wire::gojson::GoTime,
}

pub(crate) fn respond(
    request: &FeedFrame,
    received: jiff::Timestamp,
) -> Result<Option<FeedFrame>, Error> {
    let mut sample: Sample =
        serde_json::from_str(request.clock.as_ref().ok_or(Error::Invalid)?.get())?;
    if request.t == "ClockAck" {
        let times = [&sample.t0, &sample.t1, &sample.t2, &sample.t3].map(|t| {
            if t.as_str() == "0001-01-01T00:00:00Z" {
                return Err(Error::Invalid);
            }
            t.as_str()
                .parse::<jiff::Timestamp>()
                .map_err(|_| Error::Invalid)
        });
        let [t0, t1, t2, t3] = times;
        let (t0, t1, t2, t3) = (t0?, t1?, t2?, t3?);
        if t3 < t0
            || t2 < t1
            || t3.as_nanosecond() - t0.as_nanosecond() < t2.as_nanosecond() - t1.as_nanosecond()
        {
            return Err(Error::Invalid);
        }
        return Ok(None);
    }
    if request.t != "ClockPing"
        || request.id.is_empty()
        || sample.t0.as_str() == "0001-01-01T00:00:00Z"
        || sample.t1.as_str() != "0001-01-01T00:00:00Z"
        || sample.t2.as_str() != "0001-01-01T00:00:00Z"
        || sample.t3.as_str() != "0001-01-01T00:00:00Z"
    {
        return Err(Error::Invalid);
    }
    sample.t1 =
        ab_trust::wire::gojson::GoTime::parse(&received.to_string()).map_err(|_| Error::Invalid)?;
    sample.t2 = ab_trust::wire::gojson::GoTime::parse(&jiff::Timestamp::now().to_string())
        .map_err(|_| Error::Invalid)?;
    let mut response = frame("ClockPong");
    response.id.clone_from(&request.id);
    response.clock = Some(raw(&sample)?);
    Ok(Some(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(kind: &str, times: [&str; 4]) -> Result<FeedFrame, Error> {
        let mut request = frame(kind);
        request.id = "probe".into();
        request.clock = Some(raw(
            &serde_json::json!({"t0":times[0],"t1":times[1],"t2":times[2],"t3":times[3]}),
        )?);
        Ok(request)
    }

    #[test]
    fn pong_preserves_probe_and_stamps_only_gateway_times() -> Result<(), Error> {
        let zero = "0001-01-01T00:00:00Z";
        let sent = "2026-11-02T00:00:00Z";
        let request = request("ClockPing", [sent, zero, zero, zero])?;
        let received = jiff::Timestamp::now();
        let response = respond(&request, received)?.ok_or(Error::Invalid)?;
        let sample: Sample = serde_json::from_str(response.clock.ok_or(Error::Invalid)?.get())?;
        assert_eq!(response.t, "ClockPong");
        assert_eq!(response.id, request.id);
        assert_eq!(sample.t0.as_str(), sent);
        assert_eq!(sample.t1.as_str(), received.to_string());
        assert!(
            sample
                .t2
                .as_str()
                .parse::<jiff::Timestamp>()
                .map_err(|_| Error::Invalid)?
                >= received
        );
        assert_eq!(sample.t3.as_str(), zero);
        Ok(())
    }

    #[test]
    fn clock_ack_rejects_impossible_round_trip() -> Result<(), Error> {
        let t0 = "2026-11-02T00:00:00Z";
        let t1 = "2026-11-02T00:00:01Z";
        let t2 = "2026-11-02T00:00:03Z";
        let invalid = request("ClockAck", [t0, t1, t2, t1])?;
        assert!(respond(&invalid, jiff::Timestamp::now()).is_err());
        let valid = request("ClockAck", [t0, t1, t2, t2])?;
        assert!(respond(&valid, jiff::Timestamp::now())?.is_none());
        Ok(())
    }
}
