//! HTTP layer. Handlers translate between HTTP and the domain and nothing else:
//! lifecycle rules belong in `domain`, SQL exclusively in `repository`. That
//! separation is what makes the domain testable without HTTP.

pub mod access_log;
pub mod assets;
pub mod client_ip;
pub mod health;
pub mod pastes;
pub mod problem;
pub mod rate_limit;
pub mod reports;
pub mod request_id;
pub mod security_headers;
