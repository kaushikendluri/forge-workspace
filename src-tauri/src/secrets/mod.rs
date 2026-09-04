//! API key storage via the OS keychain (Windows Credential Manager / macOS
//! Keychain), through the `keyring` crate. Keys never touch the SQLite
//! database — only a reference (e.g. "has a key configured") does.

use crate::error::AppResult;

const SERVICE_NAME: &str = "forge-workspace";

/// Stores `value` under `key` (e.g. "anthropic_api_key") in the OS keychain.
pub fn set_secret(_key: &str, _value: &str) -> AppResult<()> {
    todo!("M4: keyring::Entry::new(SERVICE_NAME, key).set_password(value)")
}

/// Retrieves a previously stored secret, if any.
pub fn get_secret(_key: &str) -> AppResult<Option<String>> {
    todo!("M4: keyring::Entry::new(SERVICE_NAME, key).get_password()")
}

pub fn delete_secret(_key: &str) -> AppResult<()> {
    todo!("M4: keyring::Entry::new(SERVICE_NAME, key).delete_credential()")
}

#[allow(dead_code)]
fn service_name() -> &'static str {
    SERVICE_NAME
}
