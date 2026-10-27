//! OAuth 2.0 sign-in for extensions (authorization code + PKCE, RFC 7636),
//! with the browser coming back to a loopback address (RFC 8252).
//!
//! The launcher does the whole dance so extensions never see a client
//! secret's storage, a redirect listener or a refresh token: they ask for an
//! access token and get one. Tokens live with the other secrets (keyring),
//! under the extension id and `oauth.<provider>`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::secrets;

/// How long the browser has to come back.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

/// An OAuth service, as described by the extension.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    /// Names the stored tokens: one sign-in per provider and extension.
    pub id: String,
    /// Shown on the page the browser lands on.
    #[serde(default)]
    pub title: Option<String>,
    pub authorize_url: String,
    pub token_url: String,
    pub client_id: String,
    /// Some services want one even with PKCE. It ships with the extension,
    /// so it is not a secret in the strict sense (as in any desktop app).
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub scope_separator: Option<String>,
    /// Extra query parameters for the authorize URL.
    #[serde(default)]
    pub params: serde_json::Map<String, Value>,
    /// Services that only accept a registered redirect need a fixed port.
    #[serde(default)]
    pub redirect_port: Option<u16>,
    #[serde(default)]
    pub redirect_path: Option<String>,
    /// `basic` sends the client credentials as HTTP Basic (Notion);
    /// the default puts them in the form.
    #[serde(default)]
    pub client_auth: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Seconds since the epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl Tokens {
    /// Still usable for a minute at least.
    pub fn is_fresh(&self) -> bool {
        self.expires_at.is_none_or(|at| at > now() + 60)
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn key(provider: &str) -> String {
    format!("oauth.{provider}")
}

pub fn stored(extension: &str, provider: &str) -> Option<Tokens> {
    serde_json::from_str(&secrets::get(extension, &key(provider))?).ok()
}

fn store(extension: &str, provider: &str, tokens: &Tokens) -> Result<()> {
    secrets::set(extension, &key(provider), &serde_json::to_string(tokens)?)
}

pub fn sign_out(extension: &str, provider: &str) -> Result<()> {
    secrets::delete(extension, &key(provider))
}

/// `https://api.linear.app/oauth/token` → `api.linear.app`.
fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    Some(host.split(':').next().unwrap_or(host)).filter(|h| !h.is_empty())
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

impl Provider {
    /// Refuses providers the extension did not declare: the token endpoint
    /// must be a host from its `net` permission, and both URLs use HTTPS
    /// (plain HTTP only for a local server).
    pub fn check(&self, allowed_hosts: &[String]) -> Result<()> {
        if self.id.is_empty() || !self.id.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) {
            bail!("invalid provider id `{}`", self.id);
        }
        if self.client_id.is_empty() {
            bail!("{}: no client id", self.id);
        }
        for url in [&self.authorize_url, &self.token_url] {
            let host = host_of(url).ok_or_else(|| anyhow!("invalid URL {url}"))?;
            if !url.starts_with("https://") && !(url.starts_with("http://") && is_loopback(host)) {
                bail!("{url}: OAuth needs HTTPS");
            }
        }
        let host = host_of(&self.token_url).unwrap_or_default();
        let declared = allowed_hosts.iter().any(|h| h == "*" || h == host || h.split(':').next() == Some(host));
        if !declared {
            bail!("{host} is not in the extension's net permission");
        }
        Ok(())
    }

    fn title(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.id)
    }
}

/// A valid access token without asking the user: the stored one, refreshed
/// when it expired. `None` when a sign-in is needed.
pub fn quiet_token(extension: &str, provider: &Provider) -> Result<Option<String>> {
    let Some(tokens) = stored(extension, &provider.id) else { return Ok(None) };
    if tokens.is_fresh() {
        return Ok(Some(tokens.access_token));
    }
    let Some(refresh) = tokens.refresh_token.clone() else { return Ok(None) };
    match exchange(provider, &[("grant_type", "refresh_token"), ("refresh_token", &refresh)]) {
        Ok(mut fresh) => {
            // Services that don't rotate refresh tokens omit them.
            fresh.refresh_token = fresh.refresh_token.or(Some(refresh));
            store(extension, &provider.id, &fresh)?;
            Ok(Some(fresh.access_token))
        }
        // Revoked: sign in again.
        Err(e) => {
            eprintln!("backslash: refreshing the {} token failed: {e:#}", provider.id);
            Ok(None)
        }
    }
}

fn random_token(bytes: usize) -> Result<String> {
    let mut buf = vec![0u8; bytes];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf))
}

fn challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Percent-encodes a query component (RFC 3986 unreserved characters kept).
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode(k), decode(v))
        })
        .collect()
}

/// A sign-in waiting for the browser: open [`SignIn::url`], then call
/// [`SignIn::finish`] (it blocks, so off the UI thread).
pub struct SignIn {
    pub url: String,
    extension: String,
    provider: Provider,
    listener: TcpListener,
    redirect_uri: String,
    verifier: String,
    state: String,
}

impl SignIn {
    pub fn start(extension: &str, provider: &Provider) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", provider.redirect_port.unwrap_or(0)))
            .context("cannot listen for the browser's answer")?;
        let port = listener.local_addr()?.port();
        let path = provider.redirect_path.as_deref().unwrap_or("/callback");
        let redirect_uri = format!("http://127.0.0.1:{port}{path}");
        let verifier = random_token(48)?;
        let state = random_token(16)?;

        let mut query = vec![
            ("response_type".to_string(), "code".to_string()),
            ("client_id".into(), provider.client_id.clone()),
            ("redirect_uri".into(), redirect_uri.clone()),
            ("state".into(), state.clone()),
            ("code_challenge".into(), challenge(&verifier)),
            ("code_challenge_method".into(), "S256".into()),
        ];
        if !provider.scopes.is_empty() {
            let sep = provider.scope_separator.as_deref().unwrap_or(" ");
            query.push(("scope".into(), provider.scopes.join(sep)));
        }
        for (k, v) in &provider.params {
            let v = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
            query.push((k.clone(), v));
        }
        let query = query.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect::<Vec<_>>().join("&");
        let sep = if provider.authorize_url.contains('?') { '&' } else { '?' };
        Ok(Self {
            url: format!("{}{sep}{query}", provider.authorize_url),
            extension: extension.to_string(),
            provider: provider.clone(),
            listener,
            redirect_uri,
            verifier,
            state,
        })
    }

    /// Waits for the browser, trades the code for tokens and stores them.
    pub fn finish(self) -> Result<String> {
        let code = self.wait_for_code()?;
        let tokens = exchange(
            &self.provider,
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", &self.redirect_uri),
                ("code_verifier", &self.verifier),
            ],
        )?;
        store(&self.extension, &self.provider.id, &tokens)?;
        Ok(tokens.access_token)
    }

    fn wait_for_code(&self) -> Result<String> {
        let deadline = Instant::now() + SIGN_IN_TIMEOUT;
        self.listener.set_nonblocking(true)?;
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(result) = self.serve(stream) {
                        return result;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() > deadline {
                        bail!("the sign-in timed out");
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Answers one request. `None` for requests that aren't the redirect
    /// (favicon, a stray tab), so the wait goes on.
    fn serve(&self, mut stream: TcpStream) -> Option<Result<String>> {
        stream.set_nonblocking(false).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).ok()?;
        let target = line.split_whitespace().nth(1)?.to_string();
        let path = self.provider.redirect_path.as_deref().unwrap_or("/callback");
        let (req_path, query) = target.split_once('?').unwrap_or((&target, ""));
        if req_path != path {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            return None;
        }
        let params = query_pairs(query);
        let get = |k: &str| params.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        let result = if get("state").as_deref() != Some(self.state.as_str()) {
            Err(anyhow!("the browser came back with an unexpected state"))
        } else if let Some(code) = get("code") {
            Ok(code)
        } else {
            let error = get("error").unwrap_or_else(|| "no code".into());
            Err(anyhow!("{}", get("error_description").map_or(error.clone(), |d| format!("{error}: {d}"))))
        };
        let (heading, text) = match &result {
            Ok(_) => ("Signed in".to_string(), "You can close this tab and go back to Backslash.".to_string()),
            Err(e) => ("Sign-in failed".to_string(), e.to_string()),
        };
        let page = page(&heading, self.provider.title(), &text);
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
            page.len()
        );
        Some(result)
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn page(heading: &str, service: &str, text: &str) -> String {
    format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\"><title>Backslash</title>\
<style>body{{font:16px system-ui,sans-serif;display:grid;place-items:center;min-height:90vh;margin:0;background:#f6f6f7;color:#222}}\
main{{text-align:center;padding:2rem}}h1{{font-size:1.4rem;margin:.5rem 0}}p{{color:#666}}b{{font-size:2.4rem}}\
@media(prefers-color-scheme:dark){{body{{background:#1c1c1f;color:#eee}}p{{color:#aaa}}}}</style>\
<main><b>\\</b><h1>{} · {}</h1><p>{}</p></main>",
        escape(heading),
        escape(service),
        escape(text)
    )
}

/// Calls the token endpoint.
fn exchange(provider: &Provider, form: &[(&str, &str)]) -> Result<Tokens> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .build()
        .into();
    let mut form: Vec<(&str, &str)> = form.to_vec();
    let basic = provider.client_auth.as_deref() == Some("basic");
    let mut request = agent.post(&provider.token_url).header("Accept", "application/json");
    if basic {
        let credentials = format!("{}:{}", provider.client_id, provider.client_secret.as_deref().unwrap_or_default());
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
        request = request.header("Authorization", &format!("Basic {encoded}"));
    } else {
        form.push(("client_id", &provider.client_id));
        if let Some(secret) = &provider.client_secret {
            form.push(("client_secret", secret));
        }
    }
    let mut response = request.send_form(form).with_context(|| format!("cannot reach {}", provider.token_url))?;
    let status = response.status();
    let body = response.body_mut().read_to_string()?;
    // Usually JSON; some services answer with a form despite `Accept`.
    let value: Value = serde_json::from_str(&body).unwrap_or_else(|_| {
        Value::Object(query_pairs(&body).into_iter().map(|(k, v)| (k, Value::String(v))).collect())
    });
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        let description = value.get("error_description").and_then(Value::as_str);
        bail!("{}", description.map_or(error.to_string(), |d| format!("{error}: {d}")));
    }
    if !status.is_success() {
        bail!("{} answered {status}", provider.token_url);
    }
    let text = |k: &str| value.get(k).and_then(Value::as_str).map(str::to_owned);
    let access_token = text("access_token").ok_or_else(|| anyhow!("no access token in the answer"))?;
    let expires_in = value.get("expires_in").and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()));
    Ok(Tokens {
        access_token,
        refresh_token: text("refresh_token"),
        expires_at: expires_in.map(|s| now() + s),
        scope: text("scope"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(token_url: &str) -> Provider {
        serde_json::from_value(serde_json::json!({
            "id": "demo",
            "authorizeUrl": "https://example.com/authorize?prompt=consent",
            "tokenUrl": token_url,
            "clientId": "client 1",
            "scopes": ["read", "write"],
            "params": { "actor": "user" },
        }))
        .unwrap()
    }

    #[test]
    fn checks_hosts_and_schemes() {
        let p = provider("https://api.example.com/token");
        assert!(p.check(&["api.example.com".into()]).is_ok());
        assert!(p.check(&["*".into()]).is_ok());
        assert!(p.check(&["example.org".into()]).is_err());
        assert!(provider("http://api.example.com/token").check(&["*".into()]).is_err());
        assert!(provider("http://127.0.0.1:9/token").check(&["127.0.0.1".into()]).is_ok());
        assert_eq!(host_of("https://u:p@a.b:8080/x?y"), Some("a.b"));
    }

    #[test]
    fn encoding() {
        assert_eq!(encode("a b/é"), "a%20b%2F%C3%A9");
        assert_eq!(decode("a+b%2F%C3%A9%"), "a b/é%");
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    /// The whole flow against a fake service: browser redirect, then the
    /// token endpoint, then a refresh.
    #[test]
    fn sign_in_and_refresh() {
        let _env = secrets::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", dir.path());
        std::env::set_var("BACKSLASH_SECRETS", "file");
        std::env::set_var("NO_PROXY", "127.0.0.1");

        // Token endpoint: first a code exchange, then a refresh.
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let token_url = format!("http://127.0.0.1:{}/token", server.local_addr().unwrap().port());
        let seen = std::thread::spawn(move || {
            let mut bodies = Vec::new();
            for (n, stream) in server.incoming().take(2).enumerate() {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(v) = line.to_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                bodies.push(String::from_utf8(body).unwrap());
                let answer = if n == 0 {
                    r#"{"access_token":"at1","refresh_token":"rt1","expires_in":0}"#
                } else {
                    r#"{"access_token":"at2","expires_in":3600}"#
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{answer}",
                    answer.len()
                )
                .unwrap();
            }
            bodies
        });

        let p = provider(&token_url);
        let sign_in = SignIn::start("ext", &p).unwrap();
        assert!(sign_in
            .url
            .starts_with("https://example.com/authorize?prompt=consent&response_type=code&client_id=client%201"));
        assert!(sign_in.url.contains("scope=read%20write") && sign_in.url.contains("actor=user"));
        let pairs = query_pairs(sign_in.url.split_once('?').unwrap().1);
        let get = |k: &str| pairs.iter().find(|(key, _)| key == k).unwrap().1.clone();
        let (redirect, state) = (get("redirect_uri"), get("state"));

        // The browser: a stray request, then the redirect.
        let browser = std::thread::spawn(move || {
            let address = redirect.trim_start_matches("http://").split('/').next().unwrap().to_string();
            for target in ["/favicon.ico".to_string(), format!("/callback?code=c%2F1&state={state}")] {
                let mut s = TcpStream::connect(&address).unwrap();
                write!(s, "GET {target} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
                let mut page = String::new();
                s.read_to_string(&mut page).unwrap();
                if target.starts_with("/callback") {
                    assert!(page.contains("Signed in"), "{page}");
                }
            }
        });
        assert_eq!(sign_in.finish().unwrap(), "at1");
        browser.join().unwrap();

        // Expired (expires_in 0): refreshed quietly, the refresh token kept.
        assert_eq!(quiet_token("ext", &p).unwrap().as_deref(), Some("at2"));
        let bodies = seen.join().unwrap();
        assert!(bodies[0].contains("grant_type=authorization_code") && bodies[0].contains("code=c%2F1"));
        assert!(bodies[0].contains("code_verifier=") && bodies[0].contains("client_id=client+1"));
        assert!(bodies[1].contains("grant_type=refresh_token&refresh_token=rt1"));
        assert_eq!(stored("ext", "demo").unwrap().refresh_token.as_deref(), Some("rt1"));
        assert_eq!(quiet_token("ext", &p).unwrap().as_deref(), Some("at2"));

        sign_out("ext", "demo").unwrap();
        assert_eq!(quiet_token("ext", &p).unwrap(), None);
    }
}
