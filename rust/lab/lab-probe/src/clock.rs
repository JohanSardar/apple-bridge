use crate::{Error, Result};
use serde_json::Value;
use std::path::Path;

const ZERO_SECONDS: i64 = -62_135_596_800;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Estimate {
    pub offset_ns: i128,
    pub uncertainty_ns: i128,
}
impl Estimate {
    pub fn from_sample(sample: &Value) -> Result<Self> {
        let at = |name: &str| -> Result<i128> {
            let stamp: jiff::Timestamp = sample[name]
                .as_str()
                .and_then(|text| text.parse().ok())
                .ok_or(Error::Invalid("clock sample"))?;
            if stamp.as_second() <= ZERO_SECONDS {
                return Err(Error::Invalid("clock sample"));
            }
            Ok(stamp.as_nanosecond())
        };
        let (t0, t1, t2, t3) = (at("t0")?, at("t1")?, at("t2")?, at("t3")?);
        if t3 < t0 || t2 < t1 {
            return Err(Error::Invalid("clock sample"));
        }
        let rtt = (t3 - t0) - (t2 - t1);
        if rtt < 0 {
            return Err(Error::Invalid("clock sample"));
        }
        Ok(Self {
            offset_ns: i128::midpoint(t1 - t0, t2 - t3),
            uncertainty_ns: rtt / 2,
        })
    }
    pub fn load(home: &Path) -> Result<Self> {
        let raw = std::fs::read(home.join("gateway-clock.json"))?;
        Self::from_sample(&serde_json::from_slice(&raw)?)
    }
    #[must_use]
    pub fn uncertainty_ms(&self) -> f64 {
        to_ms(self.uncertainty_ns)
    }
    #[must_use]
    pub fn offset_ms(&self) -> f64 {
        to_ms(self.offset_ns)
    }
}
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn to_ms(nanoseconds: i128) -> f64 {
    nanoseconds as f64 / 1_000_000.0
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::Estimate;
    use serde_json::json;

    #[test]
    fn estimates_offset_and_uncertainty() {
        let sample = json!({
            "t0": "2026-01-01T00:00:00.000Z",
            "t1": "2026-01-01T00:00:00.110Z",
            "t2": "2026-01-01T00:00:00.112Z",
            "t3": "2026-01-01T00:00:00.022Z",
        });
        let estimate = Estimate::from_sample(&sample).unwrap();
        assert_eq!(estimate.offset_ns, 100_000_000);
        assert_eq!(estimate.uncertainty_ns, 10_000_000);
        assert!((estimate.uncertainty_ms() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_unordered_or_missing_samples() {
        let backwards = json!({
            "t0": "2026-01-01T00:00:01Z", "t1": "2026-01-01T00:00:01Z",
            "t2": "2026-01-01T00:00:01Z", "t3": "2026-01-01T00:00:00Z",
        });
        assert!(Estimate::from_sample(&backwards).is_err());
        let zero = json!({
            "t0": "0001-01-01T00:00:00Z", "t1": "2026-01-01T00:00:00Z",
            "t2": "2026-01-01T00:00:00Z", "t3": "2026-01-01T00:00:00Z",
        });
        assert!(Estimate::from_sample(&zero).is_err());
        assert!(Estimate::from_sample(&json!({})).is_err());
    }
}
