use crate::Error;
pub fn parse(raw: &str) -> Result<i64, Error> {
    let (negative, mut raw) = raw.strip_prefix('-').map_or_else(
        || (false, raw.strip_prefix('+').unwrap_or(raw)),
        |s| (true, s),
    );
    if raw.is_empty() {
        return Err(Error::Invalid);
    }
    if raw == "0" {
        return Ok(0);
    }
    let mut total = 0u128;
    while !raw.is_empty() {
        let end = raw
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .ok_or(Error::Invalid)?;
        let number = &raw[..end];
        raw = &raw[end..];
        let (unit, scale) = [
            ("ns", 1u128),
            ("us", 1000),
            ("µs", 1000),
            ("μs", 1000),
            ("ms", 1_000_000),
            ("s", 1_000_000_000),
            ("m", 60_000_000_000),
            ("h", 3_600_000_000_000),
        ]
        .into_iter()
        .find(|(unit, _)| raw.starts_with(unit))
        .ok_or(Error::Invalid)?;
        raw = &raw[unit.len()..];
        let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
        if whole.is_empty() && fraction.is_empty() {
            return Err(Error::Invalid);
        }
        let whole = if whole.is_empty() {
            0
        } else {
            whole.parse::<u128>().map_err(|_| Error::Invalid)?
        };
        let mut value = whole.checked_mul(scale).ok_or(Error::Invalid)?;
        if !fraction.is_empty() {
            let digits = u32::try_from(fraction.len()).map_err(|_| Error::Invalid)?;
            value = value
                .checked_add(
                    fraction
                        .parse::<u128>()
                        .map_err(|_| Error::Invalid)?
                        .checked_mul(scale)
                        .ok_or(Error::Invalid)?
                        / 10u128.checked_pow(digits).ok_or(Error::Invalid)?,
                )
                .ok_or(Error::Invalid)?;
        }
        total = total.checked_add(value).ok_or(Error::Invalid)?;
    }
    let signed = i128::try_from(total).map_err(|_| Error::Invalid)?;
    i64::try_from(if negative { -signed } else { signed }).map_err(|_| Error::Invalid)
}
#[cfg(test)]
mod tests {
    #[test]
    fn go_duration_units_and_overflow() -> Result<(), crate::Error> {
        assert_eq!(super::parse("1h2m3.5s")?, 3_723_500_000_000);
        assert_eq!(super::parse("-.5ms")?, -500_000);
        assert_eq!(super::parse("0")?, 0);
        assert!(super::parse("9223372036854775808ns").is_err());
        assert!(super::parse("1.2.3s").is_err());
        Ok(())
    }
}
