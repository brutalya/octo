/// Parses `YYYY-MM-DD[T| ]HH:MM:SS[.fraction][Z|+HH:MM|-HH:MM]` to unix milliseconds.
/// A missing offset means UTC.
pub fn parse_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 19 || !(b[10] == b'T' || b[10] == b' ') {
        return None;
    }
    let num = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if b[4] != b'-' || b[7] != b'-' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut rest = &s[19..];
    let mut millis = 0;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits: String = frac.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            return None;
        }
        let ms: String = digits.chars().chain("000".chars()).take(3).collect();
        millis = ms.parse::<i64>().ok()?;
        rest = &frac[digits.len()..];
    }
    let offset_min = match rest {
        "" | "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let (oh, om) = rest.get(1..)?.split_once(':')?;
            sign * (oh.parse::<i64>().ok()? * 60 + om.parse::<i64>().ok()?)
        }
    };
    let secs = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + sec - offset_min * 60;
    Some(secs * 1000 + millis)
}

/// Days since 1970-01-01 (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::parse_ms;

    #[test]
    fn parses_common_shapes() {
        assert_eq!(parse_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_ms("2026-03-01T10:00:00.250Z"),
            Some(1_772_359_200_250)
        );
        assert_eq!(
            parse_ms("2026-03-01 10:05:00.123456789+00:00"),
            Some(1_772_359_500_123)
        );
        assert_eq!(
            parse_ms("2026-03-01 12:00:00+02:00"),
            Some(1_772_359_200_000)
        );
        assert_eq!(parse_ms("2026-03-01T10:00:00"), Some(1_772_359_200_000));
    }

    #[test]
    fn rejects_garbage() {
        for s in [
            "",
            "yesterday",
            "2026-13-01T00:00:00Z",
            "2026-03-01T10:00:00+x",
        ] {
            assert_eq!(parse_ms(s), None, "{s}");
        }
    }
}
