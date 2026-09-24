use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        // Never print database URLs, SQL errors, or request bodies.
        eprintln!("{message}");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), &'static str> {
    let database_url = zeroize::Zeroizing::new(
        std::env::var("ME_DATABASE_URL")
            .map_err(|_| "Set ME_DATABASE_URL to a PostgreSQL database.")?,
    );
    let bind: SocketAddr = std::env::var("ME_API_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8787".into())
        .parse()
        .map_err(|_| "Invalid ME_API_BIND.")?;
    let service = me_api::Service::connect(&database_url)
        .await
        .map_err(|_| "Could not initialize the account database.")?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|_| "Could not bind the account service.")?;
    eprintln!("ME account service listening on {bind}");
    axum::serve(
        listener,
        me_api::router(service).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
    .map_err(|_| "The account service stopped unexpectedly.")
}
