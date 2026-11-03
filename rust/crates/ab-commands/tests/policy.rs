use serde_json::Value;
#[test]
fn go_time_math_and_html_corpus() -> Result<(), Box<dyn std::error::Error>> {
    let rows: Vec<Value> = serde_json::from_str(include_str!("../policy-vectors.json"))?;
    for row in rows {
        let accepted = row["accepted"].as_bool().ok_or("accepted")?;
        match row["kind"].as_str().ok_or("kind")? {
            "expiry" => {
                let now = row["now"].as_str().ok_or("now")?.parse()?;
                let requested = row["requested"].as_str().map(str::parse).transpose()?;
                let got = ab_commands::expiry(row["op"].as_str().ok_or("op")?, now, requested);
                assert_eq!(got.is_ok(), accepted, "{row}");
                if let Some(expires) = row["expires"].as_str() {
                    assert_eq!(got?, expires.parse()?);
                }
            }
            "clock" => {
                let times = row["times"]
                    .as_array()
                    .ok_or("times")?
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .ok_or("time")
                            .and_then(|v| v.parse().map_err(|_| "time"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let got = ab_commands::clock_estimate(times.try_into().map_err(|_| "times")?);
                assert_eq!(got.is_ok(), accepted, "{row}");
                if let Ok((offset, uncertainty)) = got {
                    assert_eq!(offset, i128::from(row["offset"].as_i64().ok_or("offset")?));
                    assert_eq!(
                        uncertainty,
                        i128::from(row["uncertainty"].as_i64().ok_or("uncertainty")?)
                    );
                }
            }
            "html" => assert_eq!(
                ab_commands::note_html_compatible(
                    &serde_json::to_vec(&row["data"])?,
                    row["html"].as_str().ok_or("html")?
                ),
                accepted,
                "{row}"
            ),
            _ => return Err("kind".into()),
        }
    }
    Ok(())
}
