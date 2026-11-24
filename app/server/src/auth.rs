//! Who may use the server: whoever presents the token, as a bearer header
//! (the Mac app) or as the cookie the login page sets (a phone's browser).
//! The server also refuses to listen on every interface, so it is reachable
//! only on the address it was given (the private network's).

use std::net::SocketAddr;

use crate::http::Request;

pub const COOKIE: &str = "finance_session";

/// A token the server accepts: at least 32 characters of `[A-Za-z0-9_-]`
/// (`openssl rand -hex 32` gives 64).
pub fn valid_token(token: &str) -> bool {
    token.len() >= 32
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Equal without an early exit, so the time taken says nothing about how
/// much of a guess was right.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The token the request carries, if any.
fn presented(request: &Request) -> Option<&str> {
    if let Some(bearer) = request
        .header("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        return Some(bearer.trim());
    }
    request.header("cookie")?.split(';').find_map(|pair| {
        pair.trim()
            .strip_prefix(COOKIE)
            .and_then(|rest| rest.strip_prefix('='))
    })
}

pub fn token_matches(given: &str, token: &str) -> bool {
    same(given.as_bytes(), token.as_bytes())
}

pub fn authorized(request: &Request, token: &str) -> bool {
    presented(request).is_some_and(|t| token_matches(t, token))
}

/// The token from the login form's body (`token=…`).
pub fn login_token(body: &[u8]) -> Option<&str> {
    let body = std::str::from_utf8(body).ok()?;
    body.split('&')
        .find_map(|pair| pair.strip_prefix("token="))
        .map(str::trim)
        .filter(|t| valid_token(t))
}

/// No other site's page may post here: a browser's `Origin`, when sent,
/// must be this server itself.
pub fn same_origin(request: &Request) -> bool {
    let Some(origin) = request.header("origin") else {
        return true;
    };
    let host = request.header("host").unwrap_or("");
    origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .is_some_and(|o| !host.is_empty() && o.eq_ignore_ascii_case(host))
}

/// The address to listen on. `0.0.0.0` and `::` are refused: they would
/// open the ledger on every network the machine is on, the public one too.
pub fn listen_address(value: &str) -> Result<SocketAddr, String> {
    let address: SocketAddr = value
        .parse()
        .map_err(|_| format!("FINANCE_LISTEN is not an address and port: {value}"))?;
    if address.ip().is_unspecified() {
        return Err(
            "FINANCE_LISTEN must be the private network's address (or 127.0.0.1), not every interface"
                .into(),
        );
    }
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn token_by_header_or_cookie_only_when_equal() {
        let bearer = format!("Bearer {TOKEN}");
        let cookie = format!("theme=dark; {COOKIE}={TOKEN}");
        assert!(authorized(
            &Request::with("POST", "/invoke/status", &[("authorization", &bearer)]),
            TOKEN
        ));
        assert!(authorized(
            &Request::with("GET", "/", &[("cookie", &cookie)]),
            TOKEN
        ));
        let wrong = format!("{COOKIE}={}x", &TOKEN[..31]);
        assert!(!authorized(
            &Request::with("GET", "/", &[("cookie", &wrong)]),
            TOKEN
        ));
        let prefix = format!("Bearer {}", &TOKEN[..20]);
        assert!(!authorized(
            &Request::with("GET", "/", &[("authorization", &prefix)]),
            TOKEN
        ));
        let other = "finance_session_old=0123456789abcdef0123456789abcdef";
        assert!(!authorized(
            &Request::with("GET", "/", &[("cookie", other)]),
            TOKEN
        ));
        assert!(!authorized(&Request::with("GET", "/", &[]), TOKEN));
    }

    #[test]
    fn login_form_takes_only_a_valid_token() {
        assert_eq!(
            login_token(format!("token={TOKEN}").as_bytes()),
            Some(TOKEN)
        );
        assert_eq!(login_token(b"token=short"), None);
        assert_eq!(
            login_token(b"token=%3Cscript%3E0123456789abcdef0123456789"),
            None
        );
        assert_eq!(login_token(b"user=me"), None);
    }

    #[test]
    fn other_sites_cannot_post() {
        let req = |origin| {
            Request::with(
                "POST",
                "/invoke/x",
                &[("host", "100.64.0.1:8443"), ("origin", origin)],
            )
        };
        assert!(same_origin(&req("http://100.64.0.1:8443")));
        assert!(same_origin(&req("https://100.64.0.1:8443")));
        assert!(!same_origin(&req("https://evil.example")));
        assert!(!same_origin(&req("null")));
        assert!(same_origin(&Request::with(
            "POST",
            "/invoke/x",
            &[("host", "h")]
        )));
    }

    #[test]
    fn never_every_interface() {
        assert!(listen_address("0.0.0.0:8443").is_err());
        assert!(listen_address("[::]:8443").is_err());
        assert!(listen_address("8443").is_err());
        assert!(listen_address("100.64.0.1:8443").is_ok());
        assert!(listen_address("127.0.0.1:8443").is_ok());
    }
}
