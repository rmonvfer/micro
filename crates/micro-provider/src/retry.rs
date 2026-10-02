//! What a provider says about when to ask again.

use reqwest::header::HeaderMap;
use std::time::Duration;
use std::time::SystemTime;

/// Introduces the wait a provider asked for, at the end of the error its refusal is reported as.
const RETRY_AFTER_MARKER: &str = "[retry after ";

/// How long a provider asked to be left alone before the request is tried again.
///
/// `retry-after-ms` is read first, then `retry-after` as a number of seconds or an HTTP date. A
/// date already past asks for no wait at all. A value that is none of these says nothing.
pub fn retry_after(headers: &HeaderMap, now: SystemTime) -> Option<Duration> {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };

    if let Some(milliseconds) = header("retry-after-ms").and_then(non_negative) {
        return Some(Duration::from_secs_f64(milliseconds / 1000.0));
    }

    let value = header("retry-after")?;
    if let Some(seconds) = non_negative(value) {
        return Some(Duration::from_secs_f64(seconds));
    }
    let at = httpdate::parse_http_date(value).ok()?;
    Some(at.duration_since(now).unwrap_or_default())
}

fn non_negative(value: &str) -> Option<f64> {
    value
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite() && *number >= 0.0)
}

/// The error an unsuccessful response is reported as: who refused, with what status and body, and
/// how long it asked to be left alone when it said.
pub(crate) async fn refusal(label: &str, response: reqwest::Response) -> String {
    let status = response.status().as_u16();
    let wait = retry_after(response.headers(), SystemTime::now());
    let body = response.text().await.unwrap_or_default();
    let mut error = format!("{label} returned {status}: {}", body.trim());
    if let Some(wait) = wait {
        error.push_str(&format!(" {RETRY_AFTER_MARKER}{}ms]", wait.as_millis()));
    }
    error
}

/// The wait a provider asked for, as [`refusal`] recorded it in an error.
pub fn requested_retry_delay(error: &str) -> Option<Duration> {
    let start = error.rfind(RETRY_AFTER_MARKER)? + RETRY_AFTER_MARKER.len();
    let milliseconds = error[start..].strip_suffix("ms]")?.parse::<u64>().ok()?;
    Some(Duration::from_millis(milliseconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn seconds_are_read_as_a_wait() {
        let now = SystemTime::now();
        assert_eq!(
            retry_after(&headers(&[("retry-after", "7")]), now),
            Some(Duration::from_secs(7))
        );
        assert_eq!(
            retry_after(&headers(&[("retry-after", "1.5")]), now),
            Some(Duration::from_millis(1500))
        );
    }

    #[test]
    fn milliseconds_are_preferred_over_seconds() {
        let wait = retry_after(
            &headers(&[("retry-after-ms", "250"), ("retry-after", "9")]),
            SystemTime::now(),
        );
        assert_eq!(wait, Some(Duration::from_millis(250)));
    }

    #[test]
    fn an_http_date_is_read_as_the_time_until_it() {
        let now = httpdate::parse_http_date("Wed, 21 Oct 2026 07:28:00 GMT").unwrap();
        let wait = retry_after(
            &headers(&[("retry-after", "Wed, 21 Oct 2026 07:28:30 GMT")]),
            now,
        );
        assert_eq!(wait, Some(Duration::from_secs(30)));

        let past = retry_after(
            &headers(&[("retry-after", "Wed, 21 Oct 2026 07:27:00 GMT")]),
            now,
        );
        assert_eq!(past, Some(Duration::ZERO));
    }

    #[test]
    fn an_unreadable_value_says_nothing() {
        let now = SystemTime::now();
        assert_eq!(
            retry_after(&headers(&[("retry-after", "soon-ish")]), now),
            None
        );
        assert_eq!(retry_after(&headers(&[("retry-after", "-3")]), now), None);
        assert_eq!(retry_after(&HeaderMap::new(), now), None);
    }

    #[test]
    fn the_recorded_wait_is_read_back_from_the_error() {
        let error = "Anthropic returned 429: {\"error\":\"slow down\"} [retry after 12000ms]";
        assert_eq!(
            requested_retry_delay(error),
            Some(Duration::from_millis(12_000))
        );
        assert_eq!(
            requested_retry_delay("Anthropic returned 429: slow down"),
            None
        );
    }
}
