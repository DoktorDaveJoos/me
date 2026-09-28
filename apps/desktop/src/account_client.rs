//! Blocking HTTP client: all callers run on the background executor.
use curl::easy::{Easy, List};
use me_protocol::{AccountResponse, Credentials, Recovery, Registration};
use std::time::Duration;
use zeroize::Zeroizing;

pub(super) fn server() -> Result<String, String> {
    let value = std::env::var("ME_ACCOUNT_API_URL")
        .map_err(|_| "Account setup is not available in this build yet.".to_owned())?;
    checked_server(&value)
}
fn checked_server(value: &str) -> Result<String, String> {
    let url = url::Url::parse(value).map_err(|_| "Invalid account service address.".to_owned())?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
    if (url.scheme() != "https" && !(url.scheme() == "http" && local))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("The account service requires HTTPS (HTTP is allowed only on loopback for development).".into());
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}
fn post(server: &str, route: &str, body: &[u8]) -> Result<AccountResponse, String> {
    post_status(server, route, body).map_err(|(_, message)| message)
}
fn transfer(server: &str, route: &str, body: &[u8]) -> Result<(u32, Zeroizing<Vec<u8>>), String> {
    let server = checked_server(server)?;
    let mut easy = Easy::new();
    let setup = || "Could not prepare the account request.".to_owned();
    easy.url(&format!("{server}/v1/accounts/{route}"))
        .map_err(|_| setup())?;
    easy.post(true).map_err(|_| setup())?;
    easy.follow_location(false).map_err(|_| setup())?;
    easy.connect_timeout(Duration::from_secs(8))
        .map_err(|_| setup())?;
    easy.timeout(Duration::from_secs(30)).map_err(|_| setup())?;
    easy.post_field_size(body.len() as u64)
        .map_err(|_| setup())?;
    let mut headers = List::new();
    headers
        .append("Content-Type: application/json")
        .map_err(|_| setup())?;
    easy.http_headers(headers).map_err(|_| setup())?;
    let mut response = Zeroizing::new(Vec::new());
    let mut sent = 0;
    {
        let mut transfer = easy.transfer();
        transfer
            .read_function(|buffer| {
                let count = buffer.len().min(body.len() - sent);
                buffer[..count].copy_from_slice(&body[sent..sent + count]);
                sent += count;
                Ok(count)
            })
            .map_err(|_| setup())?;
        transfer
            .write_function(|bytes| {
                if response.len() + bytes.len() > 16_384 {
                    return Ok(0);
                }
                response.extend_from_slice(bytes);
                Ok(bytes.len())
            })
            .map_err(|_| setup())?;
        transfer.perform().map_err(|_| {
            "Could not reach ME Cloud. Check your connection and try again.".to_owned()
        })?;
    }
    Ok((easy.response_code().map_err(|_| setup())?, response))
}
/// Errors carry the HTTP status (0 before a response) for credential fallback.
fn post_status(server: &str, route: &str, body: &[u8]) -> Result<AccountResponse, (u32, String)> {
    let (status, response) = transfer(server, route, body).map_err(|message| (0, message))?;
    let message = match status {
        200 | 201 => match serde_json::from_slice::<AccountResponse>(&response) {
            Ok(result)
                if result.vault.valid()
                    && result.account.revision >= 1
                    && uuid::Uuid::parse_str(&result.account.id).is_ok() =>
            {
                return Ok(result);
            }
            _ => "The account service returned an invalid response.",
        },
        400 | 413 | 422 => "Check your email, password and recovery details.",
        401 => "The email and password or recovery code could not be verified.",
        409 => "This account already exists or has changed. Sign in or restart recovery.",
        429 => "Too many attempts. Wait a minute and try again.",
        _ => "ME Cloud is unavailable. Try again shortly.",
    };
    Err((status, message.into()))
}
fn checked_identity(result: AccountResponse, email: &str) -> Result<AccountResponse, String> {
    if result.account.email != email {
        return Err("The account service returned a different account.".into());
    }
    Ok(result)
}
pub(super) fn register(server: &str, request: &Registration) -> Result<AccountResponse, String> {
    let body = Zeroizing::new(
        serde_json::to_vec(request).map_err(|_| "Could not prepare registration.".to_owned())?,
    );
    let result = checked_identity(post(server, "register", &body)?, &request.email)?;
    if result.vault != request.vault {
        return Err("The account service returned different vault keys.".into());
    }
    Ok(result)
}
pub(super) fn sign_in(
    server: &str,
    email: &str,
    password: &str,
) -> Result<AccountResponse, String> {
    let secret =
        me_core::account::authentication_secret(email, password).map_err(|e| e.to_string())?;
    match credentials_status(server, "sign-in", email, &secret) {
        // Accounts registered before normalization with decomposed input.
        Err((401, message)) => {
            match me_core::account::legacy_authentication_secret(email, password)
                .map_err(|e| e.to_string())?
            {
                Some(legacy) => credentials(server, "sign-in", email, &legacy),
                None => Err(message),
            }
        }
        result => result.map_err(|(_, message)| message),
    }
}
pub(super) fn recovery_account(
    server: &str,
    email: &str,
    code: &str,
) -> Result<AccountResponse, String> {
    let secret = me_core::account::recovery_secret(code).map_err(|e| e.to_string())?;
    credentials(server, "recovery/prepare", email, &secret)
}
fn credentials(
    server: &str,
    route: &str,
    email: &str,
    secret: &str,
) -> Result<AccountResponse, String> {
    credentials_status(server, route, email, secret).map_err(|(_, message)| message)
}
fn credentials_status(
    server: &str,
    route: &str,
    email: &str,
    secret: &str,
) -> Result<AccountResponse, (u32, String)> {
    let request = Credentials {
        email: email.into(),
        secret: secret.into(),
    };
    let body = Zeroizing::new(
        serde_json::to_vec(&request).map_err(|_| (0, "Could not prepare sign-in.".to_owned()))?,
    );
    checked_identity(post_status(server, route, &body)?, email).map_err(|message| (0, message))
}
pub(super) fn recover(server: &str, request: &Recovery) -> Result<AccountResponse, String> {
    let body = Zeroizing::new(
        serde_json::to_vec(request).map_err(|_| "Could not prepare recovery.".to_owned())?,
    );
    // If the response was lost after commit, the new credential and exact key
    // envelope prove that this recovery already succeeded. No old code is reused.
    let result = post(server, "recovery/complete", &body).or_else(|original| {
        credentials(server, "sign-in", &request.email, &request.auth_secret)
            .and_then(|r| {
                if r.vault == request.vault && r.account.revision == request.expected_revision + 1 {
                    Ok(r)
                } else {
                    Err("Recovery did not complete.".into())
                }
            })
            .map_err(|_| original)
    })?;
    let result = checked_identity(result, &request.email)?;
    if result.vault != request.vault {
        return Err("The account service returned different vault keys.".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protects_credentials_in_transit() {
        assert!(checked_server("https://accounts.example.com").is_ok());
        assert!(checked_server("http://127.0.0.1:8787").is_ok());
        for url in [
            "http://example.com",
            "https://user:pass@example.com",
            "https://example.com/?key=secret",
            "ftp://localhost",
            "https://example.com/path",
        ] {
            assert!(checked_server(url).is_err());
        }
    }

    /// Serves each response to one POST, then closes; returns the sent secrets.
    fn stub(responses: Vec<(u32, String)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let handle = std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let mut secrets = Vec::new();
            for (status, body) in responses {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(_) if start.elapsed() < Duration::from_secs(120) => {
                            std::thread::sleep(Duration::from_millis(10))
                        }
                        Err(_) => return secrets,
                    }
                };
                stream.set_nonblocking(false).unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                }
                let mut request = vec![0; length];
                reader.read_exact(&mut request).unwrap();
                let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
                secrets.push(request["secret"].as_str().unwrap().to_owned());
                write!(stream, "HTTP/1.1 {status} Stub\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            secrets
        });
        (server, handle)
    }
    const EMAIL: &str = "person@example.test";
    fn accepted() -> String {
        let (salt, wrapped, recovery) = ([1u8; 16], vec![2u8; 112], vec![3u8; 112]);
        serde_json::json!({
            "account": {"id": uuid::Uuid::new_v4().to_string(), "email": EMAIL, "email_verified": false, "revision": 1},
            "vault": {"vault_id": uuid::Uuid::new_v4().to_string(), "salt": salt, "wrapped_keys": wrapped, "recovery_keys": recovery}
        })
        .to_string()
    }
    #[test]
    fn sign_in_falls_back_to_pre_normalization_secret_after_rejection() {
        let decomposed = "Gru\u{308}ße aus Ko\u{308}ln";
        let (server, requests) = stub(vec![(401, "{}".into()), (200, accepted())]);
        sign_in(&server, EMAIL, decomposed).unwrap();
        let secrets = requests.join().unwrap();
        assert_eq!(
            secrets[0],
            *me_core::account::authentication_secret(EMAIL, decomposed).unwrap()
        );
        assert_eq!(
            secrets[1],
            *me_core::account::legacy_authentication_secret(EMAIL, decomposed)
                .unwrap()
                .unwrap()
        );
    }
    #[test]
    fn sign_in_never_retries_normalized_input() {
        let (server, requests) = stub(vec![(401, "{}".into())]);
        assert_eq!(
            sign_in(&server, EMAIL, "Grüße aus Köln").err().as_deref(),
            Some("The email and password or recovery code could not be verified.")
        );
        assert_eq!(requests.join().unwrap().len(), 1);
    }
}
