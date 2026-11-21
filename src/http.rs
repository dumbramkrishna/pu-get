//! Shared HTTP client construction for catalogs, resolvers and updates.

use std::time::Duration;

use reqwest::redirect::Policy;
use reqwest::Client;

/// User-agent sent to official portals.
pub const USER_AGENT: &str = concat!(
    "pu-get/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/dumbramkrishna/pu-get)"
);

/// Build a client with redirects, rustls and a total request timeout.
pub fn client(timeout: Duration) -> Result<Client, reqwest::Error> {
    Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .timeout(timeout)
        .redirect(Policy::limited(10))
        .build()
}

/// Render an error together with its source chain (TLS causes are otherwise
/// hidden behind reqwest's short `Display` output).
pub fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut message = error.to_string();
    let mut source = error.source();

    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }

    message
}
