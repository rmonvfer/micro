//! Signing a request the way AWS expects to be asked.

use hmac::Mac;
use sha2::Digest;
use sha2::Sha256;

type HmacSha256 = hmac::Hmac<Sha256>;

/// What AWS calls this signing scheme, sent as part of the credential scope.
const ALGORITHM: &str = "AWS4-HMAC-SHA256";
/// The suffix every scope ends with.
const TERMINATOR: &str = "aws4_request";

/// The credentials a request is signed with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    /// Present for credentials that were themselves issued temporarily, such as a role's.
    pub session_token: Option<String>,
}

/// One request, reduced to the parts that are signed.
pub struct Request<'a> {
    pub method: &'a str,
    /// The path, already URI-encoded, beginning with a slash.
    pub path: &'a str,
    /// The query string as it appears after `?`, or empty.
    pub query: &'a str,
    /// Header names and values.
    pub headers: Vec<(String, String)>,
    pub body: &'a [u8],
}

/// The headers that carry the signature, ready to be added to the request.
pub fn sign(
    request: &Request<'_>,
    credentials: &Credentials,
    region: &str,
    service: &str,
    timestamp: &str,
) -> Vec<(String, String)> {
    let date = &timestamp[..8];
    let scope = format!("{date}/{region}/{service}/{TERMINATOR}");

    let mut headers: Vec<(String, String)> = request
        .headers
        .iter()
        .map(|(name, value)| (name.to_lowercase(), value.trim().to_string()))
        .collect();
    headers.push(("x-amz-date".to_string(), timestamp.to_string()));
    if let Some(token) = &credentials.session_token {
        headers.push(("x-amz-security-token".to_string(), token.clone()));
    }
    headers.sort_by(|left, right| left.0.cmp(&right.0));
    headers.dedup_by(|left, right| left.0 == right.0);

    let signed_names: Vec<&str> = headers.iter().map(|(name, _)| name.as_str()).collect();
    let signed_headers = signed_names.join(";");

    let canonical_headers: String = headers
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect();
    let payload_hash = hex(&Sha256::digest(request.body));

    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        request.method,
        request.path,
        request.query,
        canonical_headers,
        signed_headers,
        payload_hash
    );

    let to_sign = format!(
        "{ALGORITHM}\n{timestamp}\n{scope}\n{}",
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );

    let signature = hex(&signing_key(credentials, date, region, service).chain(to_sign.as_bytes()));
    let authorization = format!(
        "{ALGORITHM} Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        credentials.access_key_id
    );

    let mut out = vec![
        ("x-amz-date".to_string(), timestamp.to_string()),
        ("x-amz-content-sha256".to_string(), payload_hash),
        ("authorization".to_string(), authorization),
    ];
    if let Some(token) = &credentials.session_token {
        out.push(("x-amz-security-token".to_string(), token.clone()));
    }
    out
}

/// The key the signature is made with, derived one step at a time so the secret itself never signs
/// anything directly.
fn signing_key(credentials: &Credentials, date: &str, region: &str, service: &str) -> Chained {
    let start = format!("AWS4{}", credentials.secret_access_key);
    Chained(start.into_bytes())
        .chain(date.as_bytes())
        .into_key()
        .chain(region.as_bytes())
        .into_key()
        .chain(service.as_bytes())
        .into_key()
        .chain(TERMINATOR.as_bytes())
        .into_key()
}

/// A key part-way through the derivation.
struct Chained(Vec<u8>);

impl Chained {
    fn chain(&self, message: &[u8]) -> Vec<u8> {
        let mut mac = HmacSha256::new_from_slice(&self.0).expect("hmac takes a key of any length");
        mac.update(message);
        mac.finalize().into_bytes().to_vec()
    }
}

trait IntoKey {
    fn into_key(self) -> Chained;
}

impl IntoKey for Vec<u8> {
    fn into_key(self) -> Chained {
        Chained(self)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The current moment, as AWS wants it written.
pub fn timestamp_now() -> String {
    format_timestamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_secs())
            .unwrap_or(0),
    )
}

/// Seconds since the epoch as `YYYYMMDDTHHMMSSZ`.
pub fn format_timestamp(epoch_seconds: u64) -> String {
    let days = epoch_seconds / 86_400;
    let seconds_today = epoch_seconds % 86_400;
    let (hour, minute, second) = (
        seconds_today / 3600,
        (seconds_today % 3600) / 60,
        seconds_today % 60,
    );

    let mut year = 1970;
    let mut left = days;
    loop {
        let in_year = match is_leap(year) {
            true => 366,
            false => 365,
        };
        if left < in_year {
            break;
        }
        left -= in_year;
        year += 1;
    }

    let lengths = month_lengths(year);
    let mut month = 1;
    for length in lengths {
        if left < length {
            break;
        }
        left -= length;
        month += 1;
    }
    let day = left + 1;

    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z")
}

fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

fn month_lengths(year: u64) -> [u64; 12] {
    [
        31,
        match is_leap(year) {
            true => 29,
            false => 28,
        },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AWS publishes a worked example for this scheme.
    #[test]
    fn it_matches_the_published_example() {
        let credentials = Credentials {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        let request = Request {
            method: "GET",
            path: "/",
            query: "",
            headers: vec![("host".into(), "example.amazonaws.com".into())],
            body: b"",
        };

        let signed = sign(
            &request,
            &credentials,
            "us-east-1",
            "service",
            "20150830T123600Z",
        );
        let authorization = signed
            .iter()
            .find(|(name, _)| name == "authorization")
            .map(|(_, value)| value.clone())
            .expect("an authorization header");

        assert!(
            authorization
                .contains("Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request"),
            "{authorization}"
        );
        assert!(
            authorization.contains("SignedHeaders=host;x-amz-date"),
            "{authorization}"
        );
        assert!(
            authorization.contains(
                "Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
            ),
            "{authorization}"
        );
    }

    /// Temporary credentials carry a token, and it is signed along with everything else.
    #[test]
    fn a_session_token_is_signed_and_sent() {
        let credentials = Credentials {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "secret".into(),
            session_token: Some("token-value".into()),
        };
        let request = Request {
            method: "POST",
            path: "/model/x/converse-stream",
            query: "",
            headers: vec![("host".into(), "bedrock.example".into())],
            body: b"{}",
        };

        let signed = sign(
            &request,
            &credentials,
            "eu-west-1",
            "bedrock",
            "20260101T000000Z",
        );
        let names: Vec<&str> = signed.iter().map(|(name, _)| name.as_str()).collect();
        assert!(names.contains(&"x-amz-security-token"));

        let authorization = &signed
            .iter()
            .find(|(name, _)| name == "authorization")
            .unwrap()
            .1;
        assert!(
            authorization.contains("x-amz-security-token"),
            "the token is part of what was signed: {authorization}"
        );
    }

    /// The body is hashed, so changing it changes the signature.
    #[test]
    fn the_body_is_part_of_the_signature() {
        let credentials = Credentials {
            access_key_id: "AKID".into(),
            secret_access_key: "secret".into(),
            session_token: None,
        };
        let make = |body: &[u8]| {
            let request = Request {
                method: "POST",
                path: "/",
                query: "",
                headers: vec![("host".into(), "example".into())],
                body,
            };
            sign(
                &request,
                &credentials,
                "us-east-1",
                "bedrock",
                "20260101T000000Z",
            )
            .into_iter()
            .find(|(name, _)| name == "authorization")
            .unwrap()
            .1
        };
        assert_ne!(make(b"{\"a\":1}"), make(b"{\"a\":2}"));
    }

    #[test]
    fn a_timestamp_is_written_the_way_aws_reads_it() {
        assert_eq!(format_timestamp(0), "19700101T000000Z");

        assert_eq!(format_timestamp(1_440_938_160), "20150830T123600Z");

        assert_eq!(format_timestamp(1_709_208_000), "20240229T120000Z");
    }
}
