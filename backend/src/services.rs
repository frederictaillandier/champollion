//! What the backend does, apart from HTTP: each service runs its
//! repositories' queries, in a transaction when they go together.

pub mod cards;
pub mod flags;
pub mod health;
pub mod pronunciations;
pub mod reviews;

pub use cards::CardService;
pub use flags::FlagService;
pub use health::HealthService;
pub use pronunciations::PronunciationService;
pub use reviews::ReviewService;
