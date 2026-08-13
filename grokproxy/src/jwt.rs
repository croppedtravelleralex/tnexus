//! Unverified JWT/date helpers.
//!
//! Expiry is read purely to schedule refreshes; the upstream is the only
//! authority on whether a token is actually valid, so no signature check here.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;

/// `exp` claim in unix seconds, or `None` when the token is not a readable JWT.
pub fn access_token_expiry(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload.as_bytes()).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    claims.get("exp")?.as_i64()
}

/// Minimal RFC3339 → unix seconds. Only the shapes xAI actually emits
/// (`2026-08-13T02:00:00Z`, optional fractional seconds) are supported.
pub fn parse_rfc3339_secs(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date, rest) = text.split_once('T')?;
    let time = rest.split(['Z', '+']).next()?.split('.').next()?;

    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;

    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next().unwrap_or("0").parse().ok()?;

    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Howard Hinnant's civil-from-days, inverted. Avoids a date crate for one call.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_shift = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_shift + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_jwt(exp: i64) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let payload = URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#).as_bytes());
        format!("{header}.{payload}.sig")
    }

    #[test]
    fn reads_exp_from_a_jwt() {
        assert_eq!(access_token_expiry(&fake_jwt(1786000000)), Some(1786000000));
    }

    #[test]
    fn non_jwt_input_is_none_not_a_panic() {
        assert_eq!(access_token_expiry(""), None);
        assert_eq!(access_token_expiry("not-a-jwt"), None);
        assert_eq!(access_token_expiry("a.!!!.c"), None);
    }

    #[test]
    fn jwt_without_exp_is_none() {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let payload = URL_SAFE_NO_PAD.encode(br#"{"sub":"x"}"#);
        assert_eq!(access_token_expiry(&format!("{header}.{payload}.s")), None);
    }

    #[test]
    fn rfc3339_matches_known_epochs() {
        assert_eq!(parse_rfc3339_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_secs("2000-03-01T00:00:00Z"), Some(951868800));
        assert_eq!(parse_rfc3339_secs("2026-08-13T02:00:00Z"), Some(1786586400));
        // Fractional seconds and offsets must not shift the whole-second value.
        assert_eq!(
            parse_rfc3339_secs("2026-08-13T02:00:00.123Z"),
            Some(1786586400)
        );
    }

    #[test]
    fn leap_day_does_not_shift_the_next_day() {
        let feb29 = parse_rfc3339_secs("2024-02-29T00:00:00Z").unwrap();
        let mar01 = parse_rfc3339_secs("2024-03-01T00:00:00Z").unwrap();
        assert_eq!(mar01 - feb29, 86_400);
    }

    #[test]
    fn malformed_dates_are_none() {
        assert_eq!(parse_rfc3339_secs("2026-08-13"), None);
        assert_eq!(parse_rfc3339_secs(""), None);
    }
}
