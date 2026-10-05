use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Why a request failed.
#[derive(Debug)]
pub enum Error {
    /// Logged and answered with a 500.
    Database(sqlx::Error),
}

impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        Self::Database(e)
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        match self {
            Self::Database(e) => {
                tracing::error!("database: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
            }
        }
    }
}
