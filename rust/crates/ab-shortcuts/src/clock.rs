use crate::{
    Error,
    cli::{Cli, Input, Runner},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashSet, time::Duration};

pub const NAME: &str = "AB Clock";
const OUTPUT_LIMIT: usize = 32 << 10;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Alarm {
    pub id: String,
    pub label: String,
    pub time: String,
    pub repeat_days: Option<Vec<i64>>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Timer {
    pub id: String,
    pub label: String,
    pub duration: i64,
    pub running: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Request {
    pub operation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm: Option<Alarm>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timer: Option<Timer>,
    #[serde(skip_serializing_if = "is_zero")]
    pub duration: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub city: String,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(value: &i64) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Readback {
    pub version: i64,
    pub capabilities: Option<Vec<String>>,
    pub timers: Option<Vec<Timer>>,
    pub operation: String,
    pub alarms: Option<Vec<Alarm>>,
    pub timer_running: bool,
    pub duration: i64,
    pub label: String,
    pub city: String,
    pub local_time: String,
}

pub struct Pack<'a> {
    pub runner: Option<&'a dyn Runner>,
    pub timeout: Duration,
}

impl std::fmt::Debug for Pack<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pack")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

fn parse_clock(text: &str, fixed_minutes: bool) -> Option<(u32, u32)> {
    let (hour, minute) = text.split_once(':')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(hour) || !digits(minute) || hour.len() > 2 || minute.len() > 2 {
        return None;
    }
    if fixed_minutes && minute.len() != 2 {
        return None;
    }
    let (hour, minute) = (hour.parse().ok()?, minute.parse().ok()?);
    (hour < 24 && minute < 60).then_some((hour, minute))
}

#[must_use]
pub fn valid_rfc3339(text: &str) -> bool {
    text.as_bytes().get(10) == Some(&b'T')
        && !text.contains(['[', ' '])
        && text.parse::<jiff::Timestamp>().is_ok()
}

fn valid_alarm(alarm: &Alarm) -> bool {
    if alarm.id.is_empty() || parse_clock(&alarm.time, true).is_none() {
        return false;
    }
    let mut seen = HashSet::new();
    alarm
        .repeat_days
        .iter()
        .flatten()
        .all(|day| (0..=6).contains(day) && seen.insert(*day))
}

fn tagged(label: &str) -> bool {
    label.starts_with("[ab-test]")
}

fn unqualified<T>() -> Result<T, Error> {
    Err(Error::ClockUnqualified)
}

fn required_fields(items: Option<&Vec<Value>>, fields: &[&str]) -> bool {
    items.is_none_or(|items| {
        items.iter().all(|item| {
            item.as_object().is_some_and(|object| {
                fields
                    .iter()
                    .all(|field| object.get(*field).is_some_and(|value| !value.is_null()))
            })
        })
    })
}

fn validate(request: &Request) -> Result<(), Error> {
    let invalid = |text: &str| Err(Error::message(text));
    match request.operation.as_str() {
        "alarms" | "timers" | "recent_timers" | "capabilities" => Ok(()),
        "delete_timer" | "delete_recent_timer" => {
            if request
                .timer
                .as_ref()
                .is_some_and(|t| !t.id.is_empty() && tagged(&t.label))
            {
                Ok(())
            } else {
                invalid("valid tagged timer required")
            }
        }
        "create_alarm" | "toggle_alarm" | "delete_alarm" => {
            if request
                .alarm
                .as_ref()
                .is_some_and(|a| tagged(&a.label) && valid_alarm(a))
            {
                Ok(())
            } else {
                invalid("valid tagged alarm required")
            }
        }
        "start_timer" => {
            if (1..=86400).contains(&request.duration) && tagged(&request.label) {
                Ok(())
            } else {
                invalid("bounded tagged timer required")
            }
        }
        "world_time" => {
            if request.city.trim().is_empty() || request.city.len() > 128 {
                invalid("city required")
            } else {
                Ok(())
            }
        }
        _ => unqualified(),
    }
}

fn alarm_mutation_matches(request: &Request, alarms: &[Alarm]) -> bool {
    let Some(requested) = request.alarm.as_ref() else {
        return false;
    };
    let creating = request.operation == "create_alarm";
    let mut found = false;
    for alarm in alarms {
        let skip = if creating {
            alarm.label != requested.label
        } else {
            alarm.id != requested.id
        };
        if skip {
            continue;
        }
        if found {
            return false;
        }
        found = true;
        let mut expected = requested.clone();
        if let Some(days) = expected.repeat_days.as_mut() {
            days.sort_unstable();
        }
        if creating {
            expected.id.clone_from(&alarm.id);
        }
        if expected != *alarm {
            return false;
        }
    }
    found == (request.operation != "delete_alarm")
}

impl Pack<'_> {
    #[must_use]
    pub fn new(runner: &dyn Runner) -> Pack<'_> {
        Pack {
            runner: Some(runner),
            timeout: Duration::ZERO,
        }
    }

    pub fn inspect(&self) -> Result<Vec<u8>, Error> {
        let default = Cli::default();
        let runner: &dyn Runner = self.runner.unwrap_or(&default);
        let result = runner.run(&Input {
            shortcut: NAME.into(),
            timeout: Duration::from_secs(15),
            ..Input::default()
        })?;
        if result.timed_out || result.exit_status != 0 || result.data.len() > OUTPUT_LIMIT {
            return unqualified();
        }
        Ok(result.data)
    }

    pub fn alarms(&self) -> Result<(), Error> {
        self.inspect()?;
        unqualified()
    }

    pub fn exchange(&self, request: &Request) -> Result<Readback, Error> {
        match self.exchange_partial(request) {
            (readback, None) => Ok(readback),
            (_, Some(error)) => Err(error),
        }
    }

    #[must_use]
    pub fn exchange_partial(&self, request: &Request) -> (Readback, Option<Error>) {
        match self.exchange_inner(request) {
            Ok(readback) => (readback, None),
            Err(failure) => (failure.0, Some(failure.1)),
        }
    }

    fn exchange_inner(&self, request: &Request) -> Result<Readback, Box<(Readback, Error)>> {
        let bare = |error: Error| Box::new((Readback::default(), error));
        validate(request).map_err(bare)?;
        let runner = self.runner.ok_or_else(|| bare(Error::ClockUnqualified))?;
        let data = serde_json::to_vec(request).map_err(|e| bare(e.into()))?;
        let timeout = if self.timeout.is_zero() {
            Duration::from_secs(15)
        } else {
            self.timeout
        };
        let result = runner
            .run(&Input {
                shortcut: NAME.into(),
                timeout,
                data: Some(data),
                ..Input::default()
            })
            .map_err(bare)?;
        if result.timed_out || result.exit_status != 0 || result.data.len() > OUTPUT_LIMIT {
            return Err(bare(Error::ClockUnqualified));
        }
        let Ok(mut out) = serde_json::from_slice::<Readback>(&result.data) else {
            return Err(bare(Error::ClockUnqualified));
        };
        if out.version != 1 || out.operation != request.operation {
            return Err(bare(Error::ClockUnqualified));
        }
        let Ok(fields) = serde_json::from_slice::<Value>(&result.data) else {
            return Err(bare(Error::ClockUnqualified));
        };
        let listed = |key: &str| fields.get(key).and_then(Value::as_array).cloned();
        if !required_fields(
            listed("alarms").as_ref(),
            &["id", "label", "time", "repeat_days", "enabled"],
        ) || !required_fields(
            listed("timers").as_ref(),
            &["id", "label", "duration", "running"],
        ) {
            return Err(bare(Error::ClockUnqualified));
        }
        let mut ids = HashSet::new();
        for alarm in out.alarms.iter_mut().flatten() {
            if let Some((hour, minute)) = parse_clock(&alarm.time, false) {
                alarm.time = format!("{hour:02}:{minute:02}");
            }
            if !valid_alarm(alarm) || !ids.insert(alarm.id.clone()) {
                return Err(bare(Error::ClockUnqualified));
            }
            if let Some(days) = alarm.repeat_days.as_mut() {
                days.sort_unstable();
            }
        }
        self.confirm(request, out)
    }

    fn confirm(
        &self,
        request: &Request,
        mut out: Readback,
    ) -> Result<Readback, Box<(Readback, Error)>> {
        let bare = |error: Error| Box::new((Readback::default(), error));
        let deleting = matches!(
            request.operation.as_str(),
            "delete_timer" | "delete_recent_timer"
        );
        match request.operation.as_str() {
            "capabilities" => {
                if out.capabilities.is_none() {
                    return Err(bare(Error::ClockUnqualified));
                }
            }
            "timers" | "delete_timer" | "recent_timers" | "delete_recent_timer" => {
                let Some(timers) = out.timers.as_ref() else {
                    return Err(bare(Error::ClockUnqualified));
                };
                let mut seen = HashSet::new();
                for timer in timers {
                    if timer.id.is_empty()
                        || !seen.insert(timer.id.clone())
                        || !(1..=86400).contains(&timer.duration)
                    {
                        return Err(bare(Error::ClockUnqualified));
                    }
                    if deleting && request.timer.as_ref().is_some_and(|t| t.id == timer.id) {
                        return Err(bare(Error::ClockUnqualified));
                    }
                }
            }
            "alarms" => {
                if out.alarms.is_none() {
                    return Err(bare(Error::ClockUnqualified));
                }
            }
            "create_alarm" | "toggle_alarm" | "delete_alarm" => {
                let Some(alarms) = out.alarms.as_ref() else {
                    return Err(bare(Error::ClockUnqualified));
                };
                if !alarm_mutation_matches(request, alarms) {
                    let fresh = self.exchange(&Request {
                        operation: "alarms".into(),
                        ..Request::default()
                    });
                    let Ok(fresh) = fresh else {
                        return Err(Box::new((out, Error::ClockUnqualified)));
                    };
                    out.alarms = fresh.alarms;
                    if !alarm_mutation_matches(request, out.alarms.as_deref().unwrap_or_default()) {
                        return Err(Box::new((out, Error::ClockUnqualified)));
                    }
                }
            }
            "start_timer" => {
                if !out.timer_running
                    || out.duration != request.duration
                    || out.label != request.label
                {
                    return Err(bare(Error::ClockUnqualified));
                }
            }
            "world_time" if out.city != request.city || !valid_rfc3339(&out.local_time) => {
                return Err(bare(Error::ClockUnqualified));
            }
            _ => {}
        }
        Ok(out)
    }
}
