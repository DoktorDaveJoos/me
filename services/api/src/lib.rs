//! Minimal account service. No vault decryption, sync, or long-lived sessions.
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use me_protocol::{
    Account, AccountResponse, Credentials, Recovery, Registration, VaultEnvelope, normalize_email,
    valid_secret,
};
use rand_core::OsRng;
mod sqlx {
    pub use sqlx_core::{error::Error, query::query, raw_sql::raw_sql, row::Row, types};
    pub use sqlx_postgres::{PgPool, PgPoolOptions};
}
use sqlx::{PgPool, PgPoolOptions, Row};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct Service {
    pool: PgPool,
    work: Arc<Semaphore>,
    rates: Arc<Mutex<HashMap<IpAddr, (Instant, u32)>>>,
    dummy_hash: Arc<String>,
}
#[derive(Debug)]
struct ApiError(StatusCode, &'static str);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({"error": self.1}))).into_response()
    }
}
const INTERNAL: ApiError = ApiError(
    StatusCode::SERVICE_UNAVAILABLE,
    "Account service is unavailable. Try again.",
);
const INVALID: ApiError = ApiError(StatusCode::BAD_REQUEST, "Check your account details.");
const AUTH: ApiError = ApiError(
    StatusCode::UNAUTHORIZED,
    "The account details could not be verified.",
);
const CONFLICT: ApiError = ApiError(
    StatusCode::CONFLICT,
    "This account cannot be created. Try signing in or recovering it.",
);

impl Service {
    pub async fn connect(url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(5))
            .connect(url)
            .await?;
        sqlx::raw_sql(include_str!("../migrations/001_accounts.sql"))
            .execute(&pool)
            .await?;
        let dummy_hash = tokio::task::spawn_blocking(|| hash_secret(&"0".repeat(64)))
            .await
            .map_err(|_| sqlx::Error::Protocol("password worker failed".into()))?
            .map_err(|_| sqlx::Error::Protocol("password worker failed".into()))?;
        Ok(Self {
            pool,
            work: Arc::new(Semaphore::new(4)),
            rates: Default::default(),
            dummy_hash: Arc::new(dummy_hash),
        })
    }
    fn limit(&self, peer: SocketAddr) -> Result<tokio::sync::OwnedSemaphorePermit, ApiError> {
        let busy = || {
            ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many attempts. Wait a minute and try again.",
            )
        };
        let mut rates = self.rates.lock().map_err(|_| INTERNAL)?;
        rates.retain(|_, (start, _)| start.elapsed() < Duration::from_secs(60));
        if rates.len() >= 10_000 && !rates.contains_key(&peer.ip()) {
            return Err(busy());
        }
        let entry = rates.entry(peer.ip()).or_insert((Instant::now(), 0));
        if entry.1 >= 20 {
            return Err(busy());
        }
        entry.1 += 1;
        self.work.clone().try_acquire_owned().map_err(|_| busy())
    }
    async fn lookup(&self, email: &str) -> Result<Option<Stored>, ApiError> {
        sqlx::query("SELECT id,email,email_verified,auth_hash,recovery_hash,envelope,revision FROM me_account WHERE email=$1")
            .bind(email).fetch_optional(&self.pool).await.map_err(|_| INTERNAL)?
            .map(|row| Ok(Stored {
                response: AccountResponse {
                    account: Account { id: row.try_get("id").map_err(|_| INTERNAL)?, email: row.try_get("email").map_err(|_| INTERNAL)?, email_verified: row.try_get("email_verified").map_err(|_| INTERNAL)?, revision: row.try_get("revision").map_err(|_| INTERNAL)? },
                    vault: row.try_get::<sqlx::types::Json<VaultEnvelope>,_>("envelope").map_err(|_| INTERNAL)?.0,
                },
                auth_hash: row.try_get("auth_hash").map_err(|_| INTERNAL)?,
                recovery_hash: row.try_get("recovery_hash").map_err(|_| INTERNAL)?,
            })).transpose()
    }
    async fn authenticate(
        &self,
        credentials: &Credentials,
        recovery: bool,
    ) -> Result<Stored, ApiError> {
        let email = normalize_email(&credentials.email).map_err(|_| AUTH)?;
        if !valid_secret(&credentials.secret) {
            return Err(AUTH);
        }
        let record = self.lookup(&email).await?;
        let hash = record
            .as_ref()
            .map(|r| {
                if recovery {
                    &r.recovery_hash
                } else {
                    &r.auth_hash
                }
            })
            .unwrap_or(&self.dummy_hash)
            .to_owned();
        let secret = zeroize::Zeroizing::new(credentials.secret.clone());
        let valid = tokio::task::spawn_blocking(move || verify(&secret, &hash))
            .await
            .map_err(|_| INTERNAL)?;
        if !valid {
            return Err(AUTH);
        }
        record.ok_or(AUTH)
    }
}
struct Stored {
    response: AccountResponse,
    auth_hash: String,
    recovery_hash: String,
}
fn hash_secret(secret: &str) -> Result<String, ApiError> {
    Argon2::default()
        .hash_password(secret.as_bytes(), &SaltString::generate(&mut OsRng))
        .map(|hash| hash.to_string())
        .map_err(|_| INTERNAL)
}
fn verify(secret: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(secret.as_bytes(), &hash)
            .is_ok()
    })
}
pub fn router(service: Service) -> Router {
    Router::new()
        .route("/health", get(|| async { StatusCode::NO_CONTENT }))
        .route("/v1/accounts/register", post(register))
        .route("/v1/accounts/sign-in", post(sign_in))
        .route("/v1/accounts/recovery/prepare", post(prepare_recovery))
        .route("/v1/accounts/recovery/complete", post(complete_recovery))
        .layer(DefaultBodyLimit::max(8192))
        .layer(axum::middleware::map_response(
            |mut response: Response| async {
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
                response
            },
        ))
        .with_state(service)
}
async fn register(
    State(service): State<Service>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(request): Json<Registration>,
) -> Result<impl IntoResponse, ApiError> {
    let _permit = service.limit(peer)?;
    let email = normalize_email(&request.email).map_err(|_| INVALID)?;
    if !valid_secret(&request.auth_secret)
        || !valid_secret(&request.recovery_secret)
        || !request.vault.valid()
    {
        return Err(INVALID);
    }
    let auth = zeroize::Zeroizing::new(request.auth_secret.clone());
    let recovery = zeroize::Zeroizing::new(request.recovery_secret.clone());
    let (auth_hash, recovery_hash) = tokio::task::spawn_blocking(move || {
        Ok::<_, ApiError>((hash_secret(&auth)?, hash_secret(&recovery)?))
    })
    .await
    .map_err(|_| INTERNAL)??;
    let id = uuid::Uuid::new_v4().to_string();
    let result = sqlx::query("INSERT INTO me_account(id,email,auth_hash,recovery_hash,vault_id,envelope) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING")
        .bind(&id).bind(&email).bind(&auth_hash).bind(&recovery_hash).bind(&request.vault.vault_id).bind(sqlx::types::Json(&request.vault)).execute(&service.pool).await.map_err(|_| INTERNAL)?;
    let created = result.rows_affected() == 1;
    let stored = service.lookup(&email).await?.ok_or(CONFLICT)?;
    if !created {
        // A retry is successful only for the same account AND exact encrypted
        // key material; a duplicate email can never overwrite someone else's keys.
        let credentials = Credentials {
            email,
            secret: request.auth_secret.clone(),
        };
        service
            .authenticate(&credentials, false)
            .await
            .map_err(|_| CONFLICT)?;
        let credentials = Credentials {
            email: credentials.email.clone(),
            secret: request.recovery_secret.clone(),
        };
        service
            .authenticate(&credentials, true)
            .await
            .map_err(|_| CONFLICT)?;
        if stored.response.vault != request.vault {
            return Err(CONFLICT);
        }
    }
    Ok((
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(stored.response),
    ))
}
async fn sign_in(
    State(service): State<Service>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(request): Json<Credentials>,
) -> Result<Json<AccountResponse>, ApiError> {
    let _permit = service.limit(peer)?;
    Ok(Json(service.authenticate(&request, false).await?.response))
}
async fn prepare_recovery(
    State(service): State<Service>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(request): Json<Credentials>,
) -> Result<Json<AccountResponse>, ApiError> {
    let _permit = service.limit(peer)?;
    Ok(Json(service.authenticate(&request, true).await?.response))
}
async fn complete_recovery(
    State(service): State<Service>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(request): Json<Recovery>,
) -> Result<Json<AccountResponse>, ApiError> {
    let _permit = service.limit(peer)?;
    if !valid_secret(&request.auth_secret)
        || !valid_secret(&request.recovery_secret)
        || !request.vault.valid()
    {
        return Err(INVALID);
    }
    let current = service
        .authenticate(
            &Credentials {
                email: request.email.clone(),
                secret: request.old_recovery_secret.clone(),
            },
            true,
        )
        .await?;
    if current.response.account.revision != request.expected_revision
        || current.response.vault.vault_id != request.vault.vault_id
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Account changed. Start recovery again.",
        ));
    }
    let auth = zeroize::Zeroizing::new(request.auth_secret.clone());
    let recovery = zeroize::Zeroizing::new(request.recovery_secret.clone());
    let (auth_hash, recovery_hash) = tokio::task::spawn_blocking(move || {
        Ok::<_, ApiError>((hash_secret(&auth)?, hash_secret(&recovery)?))
    })
    .await
    .map_err(|_| INTERNAL)??;
    let updated = sqlx::query("UPDATE me_account SET auth_hash=$1,recovery_hash=$2,envelope=$3,revision=revision+1 WHERE id=$4 AND revision=$5 AND recovery_hash=$6")
        .bind(auth_hash).bind(recovery_hash).bind(sqlx::types::Json(&request.vault)).bind(&current.response.account.id).bind(request.expected_revision).bind(&current.recovery_hash)
        .execute(&service.pool).await.map_err(|_| INTERNAL)?;
    if updated.rows_affected() != 1 {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Account changed. Start recovery again.",
        ));
    }
    Ok(Json(
        service
            .lookup(&current.response.account.email)
            .await?
            .ok_or(INTERNAL)?
            .response,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        extract::connect_info::MockConnectInfo,
        http::Request,
    };
    use tower::ServiceExt;
    async fn call(
        router: &Router,
        path: &str,
        value: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let response = router
            .clone()
            .oneshot(
                Request::post(path)
                    .header("content-type", "application/json")
                    .body(Body::from(value.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 16_384).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }
    #[tokio::test]
    #[ignore = "Requires a disposable PostgreSQL ME_TEST_DATABASE_URL"]
    async fn postgres_registration_sign_in_recovery_and_limits() {
        let url = std::env::var("ME_TEST_DATABASE_URL").expect("Set ME_TEST_DATABASE_URL");
        let service = Service::connect(&url).await.unwrap();
        let app = router(service.clone()).layer(MockConnectInfo(
            "127.0.0.2:5000".parse::<SocketAddr>().unwrap(),
        ));
        let email = format!("{}@example.test", uuid::Uuid::new_v4());
        let vault = VaultEnvelope {
            vault_id: uuid::Uuid::new_v4().to_string(),
            salt: [1; 16],
            wrapped_keys: vec![2; 112],
            recovery_keys: vec![3; 112],
        };
        let request = serde_json::json!({"email":email.to_uppercase(),"auth_secret":"a".repeat(64),"recovery_secret":"b".repeat(64),"vault":vault});
        let (first, second) = tokio::join!(
            call(&app, "/v1/accounts/register", request.clone()),
            call(&app, "/v1/accounts/register", request.clone())
        );
        assert!([first.0, second.0].contains(&StatusCode::CREATED));
        assert!([first.0, second.0].contains(&StatusCode::OK));
        assert_eq!(first.1, second.1);
        assert_eq!(first.1["account"]["email"], email);
        assert_eq!(first.1["account"]["email_verified"], false);
        let current: AccountResponse = serde_json::from_value(first.1).unwrap();
        let mut duplicate = request.clone();
        duplicate["auth_secret"] = "c".repeat(64).into();
        assert_eq!(
            call(&app, "/v1/accounts/register", duplicate).await.0,
            StatusCode::CONFLICT
        );
        for wrong_email in [&email, "missing@example.test"] {
            assert_eq!(
                call(
                    &app,
                    "/v1/accounts/sign-in",
                    serde_json::json!({"email":wrong_email,"secret":"c".repeat(64)})
                )
                .await
                .0,
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            call(
                &app,
                "/v1/accounts/sign-in",
                serde_json::json!({"email":email,"secret":"a".repeat(64)})
            )
            .await
            .0,
            StatusCode::OK
        );
        let before_restart = service.lookup(&email).await.unwrap().unwrap();
        assert!(before_restart.auth_hash.starts_with("$argon2id$"));
        assert!(!before_restart.auth_hash.contains(&"a".repeat(64)));
        let restarted = Service::connect(&url).await.unwrap();
        assert_eq!(
            restarted
                .lookup(&email)
                .await
                .unwrap()
                .unwrap()
                .response
                .account
                .id,
            current.account.id
        );
        let recovery = serde_json::json!({"email":email,"old_recovery_secret":"b".repeat(64),"auth_secret":"d".repeat(64),"recovery_secret":"e".repeat(64),"expected_revision":1,"vault":vault});
        let (a, b) = tokio::join!(
            call(&app, "/v1/accounts/recovery/complete", recovery.clone()),
            call(&app, "/v1/accounts/recovery/complete", recovery)
        );
        assert_eq!(
            [a.0, b.0].iter().filter(|s| **s == StatusCode::OK).count(),
            1
        );
        assert_eq!(
            call(
                &app,
                "/v1/accounts/sign-in",
                serde_json::json!({"email":email,"secret":"a".repeat(64)})
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(
                &app,
                "/v1/accounts/sign-in",
                serde_json::json!({"email":email,"secret":"d".repeat(64)})
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(
            call(
                &app,
                "/v1/accounts/recovery/prepare",
                serde_json::json!({"email":email,"secret":"b".repeat(64)})
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(
                &app,
                "/v1/accounts/recovery/prepare",
                serde_json::json!({"email":email,"secret":"e".repeat(64)})
            )
            .await
            .0,
            StatusCode::OK
        );
        let mut malformed = request.clone();
        malformed["email"] = "not-an-email".into();
        assert_eq!(
            call(&app, "/v1/accounts/register", malformed).await.0,
            StatusCode::BAD_REQUEST
        );
        for _ in 0..20 {
            let _ = call(
                &app,
                "/v1/accounts/sign-in",
                serde_json::json!({"email":email,"secret":"not-a-secret"}),
            )
            .await;
        }
        assert_eq!(
            call(
                &app,
                "/v1/accounts/sign-in",
                serde_json::json!({"email":email,"secret":"d".repeat(64)})
            )
            .await
            .0,
            StatusCode::TOO_MANY_REQUESTS
        );
        let row = restarted.lookup(&email).await.unwrap().unwrap();
        assert_eq!(row.response.account.revision, 2);
        sqlx::query("DELETE FROM me_account WHERE id=$1")
            .bind(current.account.id)
            .execute(&service.pool)
            .await
            .unwrap();
    }
}
