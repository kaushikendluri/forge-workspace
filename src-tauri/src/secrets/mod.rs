//! API key storage via the OS keychain (Windows Credential Manager / macOS
//! Keychain), through the `keyring` crate. Keys never touch the SQLite
//! database — only a reference (e.g. "has a key configured") does.
//!
//! Nothing in this module ever puts a secret's *value* into a `Debug`/
//! `Display` impl, an error message, or a log line — errors only ever say
//! whether a secret was found, missing, or unreadable, keyed by its account
//! name (e.g. `"anthropic_api_key"`), never by its contents.

use keyring::Entry;

use crate::error::{AppError, AppResult};

const SERVICE_NAME: &str = "forge-workspace";

/// Keychain account name for the Anthropic API key. Adding another
/// provider's key later is just another `pub const` here (and another
/// caller passing it to `set_secret`/`get_secret`/`delete_secret`) — not a
/// redesign of this module.
pub const ANTHROPIC_API_KEY: &str = "anthropic_api_key";

fn entry(key: &str) -> AppResult<Entry> {
    Entry::new(SERVICE_NAME, key)
        .map_err(|e| AppError::Other(format!("keychain unavailable for '{key}': {e}")))
}

/// Stores `value` under `key` (e.g. [`ANTHROPIC_API_KEY`]) in the OS
/// keychain, overwriting any previously stored value.
pub fn set_secret(key: &str, value: &str) -> AppResult<()> {
    entry(key)?
        .set_password(value)
        .map_err(|e| AppError::Other(format!("failed to store secret '{key}': {e}")))
}

/// Retrieves a previously stored secret, if any. `Ok(None)` — not an error —
/// when nothing has been stored under `key` yet.
pub fn get_secret(key: &str) -> AppResult<Option<String>> {
    match entry(key)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(AppError::Other(format!("failed to read secret '{key}': {e}"))),
    }
}

/// Deletes a stored secret. Succeeds as a no-op if nothing was stored.
pub fn delete_secret(key: &str) -> AppResult<()> {
    match entry(key)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(AppError::Other(format!("failed to delete secret '{key}': {e}"))),
    }
}
