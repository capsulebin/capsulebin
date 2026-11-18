//! The `CAPSULEBIN__` environment overlay folded onto a parsed TOML table.

use std::collections::BTreeMap;

use super::ConfigError;

/// Prefix for environment overrides. `CAPSULEBIN__SERVER__BIND` sets
/// `server.bind`; the double underscore separates path segments so that a single
/// underscore stays available inside a key name.
pub(super) const ENV_PREFIX: &str = "CAPSULEBIN__";

/// Folds `CAPSULEBIN__A__B=value` into the TOML table as `a.b = value`.
///
/// Values are parsed as TOML fragments so that numbers and booleans keep their
/// type: `CAPSULEBIN__PURGE__BATCH_SIZE=8` must produce an integer, not the
/// string `"8"`. A value that is not valid TOML on its own is treated as a
/// string, which is what makes `CAPSULEBIN__SERVER__BIND=0.0.0.0:8080` work.
pub(super) fn apply_env<I>(table: &mut toml::Table, vars: I) -> Result<(), ConfigError>
where
    I: IntoIterator<Item = (String, String)>,
{
    // Sorted so that the outcome does not depend on environment iteration order.
    let mut relevant: BTreeMap<String, String> = BTreeMap::new();
    for (key, value) in vars {
        if let Some(rest) = key.strip_prefix(ENV_PREFIX) {
            relevant.insert(rest.to_owned(), value);
        }
    }

    for (key, value) in relevant {
        let path: Vec<String> = key.split("__").map(|s| s.to_ascii_lowercase()).collect();
        if path.iter().any(String::is_empty) {
            return Err(ConfigError::Env(format!("{ENV_PREFIX}{key} has an empty path segment")));
        }

        let parsed = parse_scalar(&value);

        let mut cursor = &mut *table;
        for segment in &path[..path.len() - 1] {
            let entry = cursor
                .entry(segment.clone())
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            cursor = entry
                .as_table_mut()
                .ok_or_else(|| ConfigError::Env(format!("{ENV_PREFIX}{key} cannot be applied: {segment} is not a table")))?;
        }

        // Unwrap-free: the loop above leaves at least one segment.
        if let Some(last) = path.last() {
            cursor.insert(last.clone(), parsed);
        }
    }

    Ok(())
}

/// Interprets an environment value as a TOML scalar, falling back to a string.
pub(super) fn parse_scalar(value: &str) -> toml::Value {
    if let Ok(boolean) = value.parse::<bool>() {
        return toml::Value::Boolean(boolean);
    }
    if let Ok(integer) = value.parse::<i64>() {
        return toml::Value::Integer(integer);
    }
    // Comma-separated lists for array-valued settings such as token_hashes.
    if value.starts_with('[')
        && value.ends_with(']')
        && let Ok(parsed) = toml::from_str::<toml::Value>(&format!("v = {value}"))
        && let Some(inner) = parsed.get("v")
    {
        return inner.clone();
    }
    toml::Value::String(value.to_owned())
}
