//! Reading a project policy from `<policies_dir>/<name>.toml`.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use crate::{
    LoadedPolicy, PolicyError, PolicySource, invalid, parse_policy, policy_hash, validate,
};

/// `true` when `name` matches `^[a-z0-9][a-z0-9-]{0,63}$`: a preset name or a
/// file stem in the policies directory. Anything else cannot name a policy.
pub fn valid_policy_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let allowed = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-';
    match bytes.split_first() {
        Some((first, rest)) => {
            *first != b'-' && allowed(first) && rest.len() <= 63 && rest.iter().all(allowed)
        }
        None => false,
    }
}

/// Read, parse and validate `<policies_dir>/<name>.toml`.
///
/// Returns [`PolicyError::NotFound`] when `name` is not a
/// [valid policy name](valid_policy_name), when the file is absent or not a
/// regular file, or when it resolves (through a symlink) to anything outside
/// `policies_dir`, or cannot be read. A file that is found but is not UTF-8,
/// is malformed, or whose `name` differs from its file stem is
/// [`PolicyError::Invalid`].
pub fn load_project_policy(policies_dir: &Path, name: &str) -> Result<LoadedPolicy, PolicyError> {
    let not_found = || PolicyError::NotFound(name.to_string());
    if !valid_policy_name(name) {
        return Err(not_found());
    }
    // Resolve every symlink, then require the file to sit directly in the
    // resolved directory: a link pointing outside it is not followed.
    let dir = fs::canonicalize(policies_dir).map_err(|_| not_found())?;
    let path = fs::canonicalize(dir.join(format!("{name}.toml"))).map_err(|_| not_found())?;
    if path.parent() != Some(dir.as_path()) || !path.is_file() {
        return Err(not_found());
    }
    let text = fs::read_to_string(&path).map_err(|e| match e.kind() {
        ErrorKind::InvalidData => invalid("toml", "not UTF-8"),
        _ => not_found(),
    })?;
    let policy = parse_policy(&text)?;
    validate(&policy)?;
    if policy.name != name {
        return Err(invalid(
            "name",
            format!("must equal the file name '{name}'"),
        ));
    }
    let hash = policy_hash(&policy);
    Ok(LoadedPolicy {
        policy,
        hash,
        source: PolicySource::Project,
    })
}
