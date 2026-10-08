#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("TEAMSFAST_RELAY_KEY")?;
    let public_url = std::env::var("TEAMSFAST_RELAY_PUBLIC_URL")?;
    let bind = std::env::var("TEAMSFAST_RELAY_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let client_id = std::env::var("TEAMSFAST_RELAY_CLIENT_ID").ok();
    let tenant = std::env::var("TEAMSFAST_RELAY_TENANT").ok();
    let app = teamsfast_relay::server::router(
        &public_url,
        &key,
        client_id.as_deref().filter(|id| !id.is_empty()),
        tenant.as_deref().filter(|tenant| !tenant.is_empty()),
    )?;
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("TeamsFast relay listening on {bind}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
