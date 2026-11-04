//! OpenRouter credits endpoint: GET /api/v1/credits (management key required).

use serde::Deserialize;
use std::fmt;
use std::time::Duration;
use ureq::Agent;

pub const DEFAULT_API_BASE: &str = "https://openrouter.ai/api/v1";
pub const MANAGEMENT_KEYS_URL: &str = "https://openrouter.ai/settings/management-keys";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Credits {
    pub total_credits: f64,
    pub total_usage: f64,
}

impl Credits {
    pub fn remaining(&self) -> f64 {
        self.total_credits - self.total_usage
    }

    /// Share of purchased credits already spent, clamped to 0..=1.
    /// `None` when nothing has been purchased, so percentages are meaningless.
    pub fn used_fraction(&self) -> Option<f64> {
        if self.total_credits > 0.0 {
            Some((self.total_usage / self.total_credits).clamp(0.0, 1.0))
        } else {
            None
        }
    }

    pub fn remaining_fraction(&self) -> Option<f64> {
        self.used_fraction().map(|used| 1.0 - used)
    }
}

#[derive(Debug)]
pub enum ApiError {
    Unauthorized,
    Forbidden,
    Status(u16),
    Network(String),
    Decode(String),
    /// The configured base URL would send the key in cleartext.
    InsecureBase(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized => write!(
                f,
                "key rejected (401) — create a management key at {MANAGEMENT_KEYS_URL}"
            ),
            Self::Forbidden => write!(
                f,
                "this key can't read credits (403) — inference keys are not allowed; create a management key at {MANAGEMENT_KEYS_URL}"
            ),
            Self::Status(code) => write!(f, "OpenRouter returned HTTP {code}"),
            Self::Network(msg) => write!(f, "network error: {msg}"),
            Self::Decode(msg) => write!(f, "unexpected response: {msg}"),
            Self::InsecureBase(base) => write!(
                f,
                "refusing to send the management key to {base} in cleartext — use https (plain http is allowed only for localhost)"
            ),
        }
    }
}

impl std::error::Error for ApiError {}

pub fn agent() -> Agent {
    Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent(concat!("open-router-credits/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Rejects any base URL that would put the bearer token on the wire in cleartext.
/// `http://` is allowed for loopback only, so the mock-server tests keep working.
pub fn check_base(api_base: &str) -> Result<(), ApiError> {
    let base = api_base.trim();
    let lower = base.to_ascii_lowercase();
    let rest = match lower.strip_prefix("https://") {
        Some(rest) => rest,
        None => match lower.strip_prefix("http://") {
            Some(rest) if is_loopback(rest) => return Ok(()),
            _ => return Err(ApiError::InsecureBase(base.to_string())),
        },
    };
    if rest.split('/').next().unwrap_or_default().is_empty() {
        return Err(ApiError::InsecureBase(base.to_string()));
    }
    Ok(())
}

fn is_loopback(rest: &str) -> bool {
    let authority = rest.split('/').next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default();
    let host = match host.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or_default(),
        None => host.split(':').next().unwrap_or_default(),
    };
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

pub fn fetch_credits(agent: &Agent, api_base: &str, api_key: &str) -> Result<Credits, ApiError> {
    check_base(api_base)?;
    let url = format!("{}/credits", api_base.trim().trim_end_matches('/'));
    let request = agent
        .get(&url)
        .header("Authorization", &format!("Bearer {api_key}"))
        .header("Accept", "application/json");

    let mut response = match request.call() {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(401)) => return Err(ApiError::Unauthorized),
        Err(ureq::Error::StatusCode(403)) => return Err(ApiError::Forbidden),
        Err(ureq::Error::StatusCode(code)) => return Err(ApiError::Status(code)),
        Err(err) => return Err(ApiError::Network(err.to_string())),
    };

    // 64 KiB is far more than {"data":{...}} needs and keeps a hostile or broken
    // server from streaming until memory runs out.
    let body = response
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_string()
        .map_err(|err| ApiError::Network(err.to_string()))?;
    parse(&body)
}

fn parse(body: &str) -> Result<Credits, ApiError> {
    #[derive(Deserialize)]
    struct Envelope {
        data: Data,
    }
    #[derive(Deserialize)]
    struct Data {
        total_credits: f64,
        total_usage: f64,
    }

    let envelope: Envelope =
        serde_json::from_str(body).map_err(|err| ApiError::Decode(err.to_string()))?;
    Ok(Credits {
        total_credits: envelope.data.total_credits,
        total_usage: envelope.data.total_usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_documented_response() {
        let credits = parse(r#"{"data":{"total_credits":100.5,"total_usage":25.75}}"#).unwrap();
        assert_eq!(credits.total_credits, 100.5);
        assert_eq!(credits.total_usage, 25.75);
        assert!((credits.remaining() - 74.75).abs() < f64::EPSILON);
        assert!((credits.used_fraction().unwrap() - 0.256218).abs() < 1e-5);
    }

    #[test]
    fn rejects_unexpected_body() {
        assert!(matches!(parse("{}"), Err(ApiError::Decode(_))));
        assert!(matches!(parse("not json"), Err(ApiError::Decode(_))));
    }

    #[test]
    fn accepts_https_and_loopback_only() {
        for base in [
            "https://openrouter.ai/api/v1",
            "HTTPS://openrouter.ai/api/v1",
            "https://example.com",
            "http://127.0.0.1:8080",
            "http://localhost:1",
            "http://[::1]:9000",
        ] {
            assert!(check_base(base).is_ok(), "{base} should be accepted");
        }
        for base in [
            "http://openrouter.ai/api/v1",
            "http://192.168.1.10:8080",
            "ftp://openrouter.ai",
            "openrouter.ai/api/v1",
            "",
        ] {
            assert!(
                matches!(check_base(base), Err(ApiError::InsecureBase(_))),
                "{base} must be refused"
            );
        }
    }

    #[test]
    fn refuses_to_send_the_key_to_plaintext_hosts() {
        let agent = agent();
        let err = fetch_credits(&agent, "http://example.com", "sk-or-v1-secret").unwrap_err();
        assert!(matches!(err, ApiError::InsecureBase(_)));
        assert!(err.to_string().contains("cleartext"));
    }

    #[test]
    fn zero_purchases_have_no_fraction() {
        let credits = Credits {
            total_credits: 0.0,
            total_usage: 0.0,
        };
        assert!(credits.used_fraction().is_none());
        assert!(credits.remaining_fraction().is_none());
    }

    #[test]
    fn overdraw_clamps_fraction() {
        let credits = Credits {
            total_credits: 10.0,
            total_usage: 15.0,
        };
        assert_eq!(credits.used_fraction(), Some(1.0));
        assert!((credits.remaining() + 5.0).abs() < f64::EPSILON);
    }
}
