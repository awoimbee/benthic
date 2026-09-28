//! Native OAuth 2.0 for the desktop build.
//!
//! Desktop code runs outside the browser, so it cannot use Google Identity
//! Services. Instead it uses the standard installed-app flow (RFC 8252):
//! a short-lived loopback listener receives the redirect, and the authorization
//! code is exchanged with PKCE. The refresh token is kept so later syncs need
//! no interaction.

use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Fresh credentials from the token endpoint.
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
}

/// Run the interactive authorization-code flow.
///
/// Opens the system browser, waits for the redirect on a loopback port, and
/// exchanges the code for tokens. `auth_endpoint` and `token_endpoint` are
/// injectable so tests can point at a mock.
pub async fn sign_in(
    client_id: &str,
    client_secret: &str,
    auth_endpoint: &str,
    token_endpoint: &str,
    scope: &str,
) -> Result<Tokens, String> {
    let verifier = random_url_token(48);
    let challenge = code_challenge(&verifier);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    let mut auth_url = url::Url::parse(auth_endpoint).map_err(|error| error.to_string())?;
    auth_url
        .query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", scope)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");

    webbrowser::open(auth_url.as_str()).map_err(|error| error.to_string())?;

    let code = wait_for_code(&listener).await?;

    let client = reqwest::Client::new();
    let response = client
        .post(token_endpoint)
        .form(&[
            ("code", code.as_str()),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_tokens(response).await
}

/// Exchange a refresh token for a new access token.
pub async fn refresh(
    client_id: &str,
    client_secret: &str,
    token_endpoint: &str,
    refresh_token: &str,
) -> Result<Tokens, String> {
    let client = reqwest::Client::new();
    let response = client
        .post(token_endpoint)
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let mut tokens = parse_tokens(response).await?;
    if tokens.refresh_token.is_none() {
        tokens.refresh_token = Some(refresh_token.to_string());
    }
    Ok(tokens)
}

/// Accept a single redirect, reply with a closing page, and return the code.
async fn wait_for_code(listener: &TcpListener) -> Result<String, String> {
    let exchange = async {
        let (mut socket, _) = listener.accept().await.map_err(|error| error.to_string())?;
        let mut buffer = vec![0u8; 4096];
        let read = socket
            .read(&mut buffer)
            .await
            .map_err(|error| error.to_string())?;
        let request = String::from_utf8_lossy(&buffer[..read]);
        let target = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/");
        let _ = socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n\
                  <!doctype html><meta charset=utf-8><title>benthic</title>\
                  <p>Signed in. You can close this tab and return to benthic.</p>",
            )
            .await;
        let _ = socket.flush().await;

        let url =
            url::Url::parse(&format!("http://127.0.0.1{target}")).map_err(|e| e.to_string())?;
        let mut code = None;
        let mut error = None;
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "code" => code = Some(value.into_owned()),
                "error" => error = Some(value.into_owned()),
                _ => {}
            }
        }
        match (code, error) {
            (Some(code), _) => Ok(code),
            (None, Some(error)) => Err(format!("Google sign-in failed: {error}")),
            _ => Err("Google sign-in did not return an authorization code.".to_string()),
        }
    };

    tokio::time::timeout(Duration::from_secs(300), exchange)
        .await
        .map_err(|_| "Google sign-in timed out.".to_string())?
}

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

async fn parse_tokens(response: reqwest::Response) -> Result<Tokens, String> {
    let status = response.status();
    let text = response.text().await.map_err(|error| error.to_string())?;
    let parsed: TokenResponse =
        serde_json::from_str(&text).map_err(|error| format!("{error}: {text}"))?;
    if !status.is_success() {
        let message = parsed
            .error_description
            .or(parsed.error)
            .unwrap_or_else(|| format!("the token endpoint returned {status}"));
        return Err(message);
    }
    if parsed.access_token.is_empty() {
        return Err("The token endpoint returned no access token.".to_string());
    }
    Ok(Tokens {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token,
        expires_at: crate::platform::now_secs() + parsed.expires_in.max(60),
    })
}

/// A random, URL-safe PKCE code verifier / state value.
fn random_url_token(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).expect("the OS provides randomness");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buffer)
}

/// The PKCE `S256` challenge for a verifier.
fn code_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_challenge_matches_the_rfc7636_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            code_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    /// Serve one canned HTTP response, then close.
    async fn serve_once(status: &'static str, body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 8192];
            let _ = socket.read(&mut buffer).await;
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            let _ = socket.flush().await;
        });
        format!("http://{addr}/token")
    }

    #[tokio::test]
    async fn refresh_parses_the_response_and_keeps_the_refresh_token() {
        let url = serve_once("200 OK", r#"{"access_token":"new","expires_in":3600}"#).await;
        let tokens = refresh("id", "secret", &url, "keep-me").await.unwrap();
        assert_eq!(tokens.access_token, "new");
        assert_eq!(tokens.refresh_token.as_deref(), Some("keep-me"));
        assert!(tokens.expires_at > crate::platform::now_secs());
    }

    #[tokio::test]
    async fn refresh_surfaces_an_error_response() {
        let url = serve_once("400 Bad Request", r#"{"error":"invalid_grant"}"#).await;
        let error = match refresh("id", "secret", &url, "stale").await {
            Err(error) => error,
            Ok(_) => panic!("expected the refresh to fail"),
        };
        assert!(error.contains("invalid_grant"), "{error}");
    }
}
