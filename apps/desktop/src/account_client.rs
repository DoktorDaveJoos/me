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
    match easy.response_code().map_err(|_| setup())? {
        200 | 201 => {
            let result: AccountResponse = serde_json::from_slice(&response)
                .map_err(|_| "The account service returned an invalid response.".to_owned())?;
            if !result.vault.valid()
                || result.account.revision < 1
                || uuid::Uuid::parse_str(&result.account.id).is_err()
            {
                return Err("The account service returned an invalid response.".into());
            }
            Ok(result)
        }
        400 | 413 | 422 => Err("Check your email, password and recovery details.".into()),
        401 => Err("The email and password or recovery code could not be verified.".into()),
        409 => {
            Err("This account already exists or has changed. Sign in or restart recovery.".into())
        }
        429 => Err("Too many attempts. Wait a minute and try again.".into()),
        _ => Err("ME Cloud is unavailable. Try again shortly.".into()),
    }
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
    credentials(server, "sign-in", email, &secret)
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
    let request = Credentials {
        email: email.into(),
        secret: secret.into(),
    };
    let body = Zeroizing::new(
        serde_json::to_vec(&request).map_err(|_| "Could not prepare sign-in.".to_owned())?,
    );
    checked_identity(post(server, route, &body)?, email)
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
}
