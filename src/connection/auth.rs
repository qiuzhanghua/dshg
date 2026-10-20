use url::Url;

use crate::error::AppError;
use crate::secrets::SessionCookie;

/// A parsed `dsh web` launch URL: loopback base plus the one-shot token.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthTarget {
    pub base: Url,
    pub token: String,
}

impl AuthTarget {
    pub fn authority(&self) -> String {
        match self.base.port() {
            Some(port) => format!(
                "{}:{}",
                self.base.host_str().unwrap_or("127.0.0.1"),
                port
            ),
            None => self.base.host_str().unwrap_or("127.0.0.1").to_string(),
        }
    }
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "::1" | "[::1]") || host.starts_with("127.")
}

/// Parse the URL printed by `dsh web` (also accepts a bare `host:port/?token=…`).
pub fn parse_auth_url(input: &str) -> Result<AuthTarget, AppError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(AppError::LocalIo("URL is empty".to_string()));
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    let url = Url::parse(&with_scheme)?;
    if url.scheme() != "http" {
        return Err(AppError::LocalIo(
            "only http:// loopback URLs are supported".to_string(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| AppError::LocalIo("URL has no host".to_string()))?;
    if !is_loopback(host) {
        return Err(AppError::LocalIo(format!(
            "refusing non-loopback host {host:?}"
        )));
    }
    if url.path() != "/" {
        return Err(AppError::LocalIo(
            "expected the index path `/` from the dsh web URL".to_string(),
        ));
    }
    let tokens: Vec<String> = url
        .query_pairs()
        .filter(|(key, _)| key == "token")
        .map(|(_, value)| value.into_owned())
        .collect();
    if tokens.len() != 1 || tokens[0].is_empty() {
        return Err(AppError::LocalIo(
            "URL must carry exactly one non-empty `token`".to_string(),
        ));
    }
    let mut base = url.clone();
    base.set_query(None);
    base.set_fragment(None);
    Ok(AuthTarget {
        base,
        token: tokens.into_iter().next().expect("checked length"),
    })
}

struct CookieParts {
    name: String,
    value: String,
    max_age: Option<i64>,
}

fn parse_set_cookie(raw: &str) -> Result<CookieParts, AppError> {
    let mut segments = raw.split(';');
    let first = segments
        .next()
        .ok_or_else(|| AppError::Decode("empty Set-Cookie".to_string()))?
        .trim();
    let (name, value) = first
        .split_once('=')
        .ok_or_else(|| AppError::Decode("Set-Cookie without `=`".to_string()))?;
    if name.is_empty() || value.is_empty() {
        return Err(AppError::Decode("Set-Cookie has an empty name or value".to_string()));
    }
    let mut max_age = None;
    for attribute in segments {
        let attribute = attribute.trim();
        if attribute.len() > 8 && attribute[..8].eq_ignore_ascii_case("max-age=") {
            max_age = attribute[8..].parse::<i64>().ok();
        }
    }
    Ok(CookieParts {
        name: name.to_string(),
        value: value.to_string(),
        max_age,
    })
}

/// Exchange the launch token for a signed cookie (PROTOCOL.md §1.1 step ②).
///
/// The client must be built with `redirect::Policy::none()` so the `303` and its
/// `Set-Cookie` header are visible instead of being auto-followed.
pub async fn exchange_token(
    client: &reqwest::Client,
    target: &AuthTarget,
) -> Result<SessionCookie, AppError> {
    let mut url = target.base.clone();
    url.query_pairs_mut().append_pair("token", &target.token);
    let response = client.get(url).send().await?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(AppError::Unauthorized);
    }
    if status == reqwest::StatusCode::FORBIDDEN {
        return Err(AppError::TrustFence("request authority rejected".to_string()));
    }
    if status != reqwest::StatusCode::SEE_OTHER {
        return Err(AppError::Decode(format!(
            "unexpected token exchange status {status}"
        )));
    }
    let raw = response
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .ok_or_else(|| AppError::Decode("token exchange returned no Set-Cookie".to_string()))?
        .to_str()
        .map_err(|error| AppError::Decode(error.to_string()))?;
    let parts = parse_set_cookie(raw)?;
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let max_age = parts.max_age.unwrap_or(30 * 24 * 3600);
    Ok(SessionCookie {
        authority: target.authority(),
        name: parts.name,
        value: parts.value,
        expires_at: now + max_age,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_printed_url() {
        let target =
            parse_auth_url("http://127.0.0.1:52631/?token=gmgVWjJQ21LdvGwqQGCpZVWsjLDiTEsGsiLfQpLe_C0")
                .unwrap();
        assert_eq!(target.token, "gmgVWjJQ21LdvGwqQGCpZVWsjLDiTEsGsiLfQpLe_C0");
        assert_eq!(target.authority(), "127.0.0.1:52631");
        assert_eq!(target.base.as_str(), "http://127.0.0.1:52631/");
    }

    #[test]
    fn accepts_bare_host_port() {
        let target = parse_auth_url("127.0.0.1:3080/?token=t").unwrap();
        assert_eq!(target.token, "t");
    }

    #[test]
    fn rejects_non_loopback() {
        assert!(parse_auth_url("http://10.0.0.5:3080/?token=t").is_err());
    }

    #[test]
    fn rejects_missing_or_duplicate_token() {
        assert!(parse_auth_url("http://127.0.0.1:3080/").is_err());
        assert!(parse_auth_url("http://127.0.0.1:3080/?token=a&token=b").is_err());
    }

    #[test]
    fn parses_the_measured_set_cookie() {
        let raw = "dsh-auth-Z7QivDIbBx8xNqPnxV3cOsZSzOc83UP-QS3BviON668=v1.eyJ2ZXJzaW9uIjoxLCJhdXRob3JpdHkiOiIxMjcuMC4wLjE6NTI2MzEiLCJpc3N1ZWRBdCI6MTc5MjQ5MzU3ODkxMSwiZXhwaXJlc0F0IjoxNzk1MDg1NTc4OTExfQ.pCXswMVY9_VIXgadIT7uHqTk6pN6yjXczLk9P8aB7EQ; Max-Age=2592000; Path=/; Expires=Thu, 19 Nov 2026 10:52:58 GMT; HttpOnly; SameSite=Strict";
        let parts = parse_set_cookie(raw).unwrap();
        assert_eq!(parts.name, "dsh-auth-Z7QivDIbBx8xNqPnxV3cOsZSzOc83UP-QS3BviON668");
        assert!(parts.value.starts_with("v1."));
        assert_eq!(parts.max_age, Some(2_592_000));
    }
}
