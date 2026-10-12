//! Parsing, validation and hashing of policy documents (spec §3).

use fmeca_policy::{PolicyError, parse_policy, policy_hash, validate};

/// Two bands (`none` rank 0, `act` rank 1), a 3×3 matrix all `none` except
/// the bottom-right `act`, and sign-off at `none`.
const MINIMAL: &str = r#"
name = "minimal"
description = "Smallest valid policy"
class = "standard"
schema = 1

[[bands]]
id = "none"
label = "No action"
rank = 0
dispositions = ["close", "accept"]

[[bands]]
id = "act"
label = "Act"
rank = 1
dispositions = ["mitigate", "accept"]
review_interval = "90d"

[matrices.qualitative3x3]
derived_from = "qualitative3x3"
cells = [
  ["none", "none", "none"],
  ["none", "none", "none"],
  ["none", "none", "act"],
]

[signoff]
max_residual_band = "none"
"#;

/// Parse then validate.
fn check(toml_text: &str) -> Result<(), PolicyError> {
    parse_policy(toml_text).and_then(|p| validate(&p))
}

/// `Invalid` naming `field`.
fn invalid_field(result: Result<(), PolicyError>) -> Option<String> {
    match result {
        Err(PolicyError::Invalid { field, .. }) => Some(field),
        _ => None,
    }
}

#[test]
fn minimal_policy_parses() {
    assert_eq!(check(MINIMAL), Ok(()));
}

#[test]
fn unknown_key_is_rejected() {
    let text = MINIMAL.replace("rank = 1", "rank = 1\ncolour = \"red\"");
    assert_eq!(
        invalid_field(check(&text)).as_deref(),
        Some("bands[1].colour")
    );
}

#[test]
fn duplicate_band_id_is_rejected() {
    let text = MINIMAL.replace("id = \"act\"", "id = \"none\"");
    assert_eq!(invalid_field(check(&text)).as_deref(), Some("bands[1].id"));
}

#[test]
fn duplicate_rank_is_rejected() {
    let text = MINIMAL.replace("rank = 1", "rank = 0");
    assert_eq!(
        invalid_field(check(&text)).as_deref(),
        Some("bands[1].rank")
    );
}

#[test]
fn unknown_band_in_matrix_is_rejected() {
    let text = MINIMAL.replace(
        "\"none\", \"none\", \"act\"",
        "\"none\", \"none\", \"severe\"",
    );
    assert_eq!(
        invalid_field(check(&text)).as_deref(),
        Some("matrices.qualitative3x3.cells[2][2]")
    );
}

#[test]
fn wrong_matrix_size_is_rejected() {
    let text = MINIMAL.replace("  [\"none\", \"none\", \"act\"],\n", "");
    assert_eq!(
        invalid_field(check(&text)).as_deref(),
        Some("matrices.qualitative3x3.cells")
    );
}

#[test]
fn bad_derived_from_is_rejected() {
    let text = MINIMAL.replace(
        "derived_from = \"qualitative3x3\"",
        "derived_from = \"nasa8004_5x5\"",
    );
    assert_eq!(
        invalid_field(check(&text)).as_deref(),
        Some("matrices.qualitative3x3.derived_from")
    );
}

#[test]
fn non_monotone_matrix_is_rejected() {
    let text = MINIMAL.replace(
        "  [\"none\", \"none\", \"none\"],\n  [\"none\", \"none\", \"none\"],\n",
        "  [\"act\", \"none\", \"none\"],\n  [\"none\", \"none\", \"none\"],\n",
    );
    assert_eq!(
        invalid_field(check(&text)).as_deref(),
        Some("matrices.qualitative3x3.cells[0][1]")
    );
}

#[test]
fn bad_duration_is_rejected() {
    let text = MINIMAL.replace("\"90d\"", "\"3 months\"");
    assert_eq!(
        invalid_field(check(&text)).as_deref(),
        Some("bands[1].review_interval")
    );
}

#[test]
fn missing_signoff_band_is_rejected() {
    let text = MINIMAL.replace(
        "max_residual_band = \"none\"",
        "max_residual_band = \"nnoe\"",
    );
    assert_eq!(
        invalid_field(check(&text)).as_deref(),
        Some("signoff.max_residual_band")
    );
}

#[test]
fn include_kernel_gate_defaults_true() {
    let policy = parse_policy(MINIMAL).expect("MINIMAL parses");
    assert!(policy.signoff.include_kernel_gate);
}

#[test]
fn hash_is_stable_across_key_order() {
    let reordered = MINIMAL.replace(
        "name = \"minimal\"\ndescription = \"Smallest valid policy\"",
        "description = \"Smallest valid policy\"\nname = \"minimal\"",
    );
    let a = policy_hash(&parse_policy(MINIMAL).expect("MINIMAL parses"));
    let b = policy_hash(&parse_policy(&reordered).expect("reordered parses"));
    assert_eq!(a, b);
}

#[test]
fn hash_changes_with_a_cell() {
    let changed = MINIMAL.replace(
        "  [\"none\", \"none\", \"none\"],\n  [\"none\", \"none\", \"act\"],\n",
        "  [\"none\", \"none\", \"act\"],\n  [\"none\", \"none\", \"act\"],\n",
    );
    let a = policy_hash(&parse_policy(MINIMAL).expect("MINIMAL parses"));
    let b = policy_hash(&parse_policy(&changed).expect("changed parses"));
    assert_ne!(a, b);
}
