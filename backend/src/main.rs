mod controllers;
mod elevenlabs;
mod error;
mod repositories;
mod schedule;
mod services;

use std::net::SocketAddr;

use clap::Parser;
use sqlx::postgres::PgPoolOptions;

/// Stores the vocabulary read by champollion-daemon and schedules its review.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Address to listen on. Only reachable devices can connect: on the
    /// server, use the WireGuard address.
    #[arg(long, default_value = "127.0.0.1:8090", env = "CHAMPOLLION_LISTEN")]
    listen: SocketAddr,

    /// PostgreSQL connection URL, e.g. `postgres:///champollion` for the
    /// local socket.
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// ElevenLabs API key, to speak the cards' words; none are spoken
    /// without it
    #[arg(long, env = "ELEVENLABS_API_KEY", hide_env_values = true)]
    elevenlabs_api_key: Option<String>,

    /// ElevenLabs voice speaking the words
    #[arg(
        long,
        default_value = "JBFqnCBsd6RMkjVDRZzb",
        env = "CHAMPOLLION_ELEVENLABS_VOICE"
    )]
    elevenlabs_voice: String,

    /// ElevenLabs model; it must take a language code, or short words may be
    /// read in the wrong language
    #[arg(
        long,
        default_value = "eleven_turbo_v2_5",
        env = "CHAMPOLLION_ELEVENLABS_MODEL"
    )]
    elevenlabs_model: String,

    /// ElevenLabs API address
    #[arg(
        long,
        default_value = "https://api.elevenlabs.io",
        env = "CHAMPOLLION_ELEVENLABS_URL"
    )]
    elevenlabs_url: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();

    let db = PgPoolOptions::new()
        .max_connections(5)
        .connect(&args.database_url)
        .await?;
    sqlx::migrate!().run(&db).await?;

    let elevenlabs = args.elevenlabs_api_key.map(|key| {
        elevenlabs::ElevenLabs::new(
            args.elevenlabs_url,
            key,
            args.elevenlabs_voice,
            args.elevenlabs_model,
        )
    });
    let pronunciations = services::PronunciationService::new(db.clone(), elevenlabs);
    pronunciations.spawn();

    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!("listening on {}", args.listen);
    axum::serve(
        listener,
        controllers::router(controllers::AppState::new(db, pronunciations)),
    )
    .with_graceful_shutdown(shutdown())
    .await?;
    Ok(())
}

/// Ctrl-C, or SIGTERM from systemd.
async fn shutdown() {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("listen for SIGTERM");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
