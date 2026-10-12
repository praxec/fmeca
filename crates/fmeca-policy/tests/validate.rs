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

/// `MINIMAL` plus a `confidence` matrix: rows `standard` band rank 0..2,
/// columns C4…C0.
fn with_confidence(rows: &str) -> String {
    format!(
        "{MINIMAL}\n[matrices.confidence]\nderived_from = \"standard-bands\"\ncells = [\n{rows}]\n"
    )
}

const CONFIDENCE_ROWS: &str = "  [\"none\", \"none\", \"none\", \"none\", \"none\"],\n  [\"none\", \"none\", \"none\", \"act\", \"act\"],\n  [\"act\", \"act\", \"act\", \"act\", \"act\"],\n";

#[test]
fn confidence_matrix_wrong_size_is_rejected() {
    let rows = CONFIDENCE_ROWS.replace("  [\"act\", \"act\", \"act\", \"act\", \"act\"],\n", "");
    assert_eq!(
        invalid_field(check(&with_confidence(&rows))).as_deref(),
        Some("matrices.confidence.cells")
    );
}

#[test]
fn confidence_matrix_rising_confidence_must_not_lower_rank() {
    // Row 0 is `act` at C4 but `none` at C3: rank falls as confidence falls.
    let rows = CONFIDENCE_ROWS.replacen(
        "[\"none\", \"none\", \"none\", \"none\", \"none\"]",
        "[\"act\", \"none\", \"none\", \"none\", \"none\"]",
        1,
    );
    assert_eq!(
        invalid_field(check(&with_confidence(&rows))).as_deref(),
        Some("matrices.confidence.cells[0][1]")
    );
}

#[test]
fn confidence_matrix_wrong_derived_from_is_rejected() {
    let text = with_confidence(CONFIDENCE_ROWS).replace(
        "derived_from = \"standard-bands\"",
        "derived_from = \"qualitative3x3\"",
    );
    assert_eq!(
        invalid_field(check(&text)).as_deref(),
        Some("matrices.confidence.derived_from")
    );
}

#[test]
fn schema_2_is_rejected() {
    let text = MINIMAL.replace("schema = 1", "schema = 2");
    assert_eq!(invalid_field(check(&text)).as_deref(), Some("schema"));
}

#[test]
fn missing_sxp_matrix_is_rejected() {
    let start = MINIMAL.find("[matrices.qualitative3x3]").expect("matrix");
    let end = MINIMAL.find("[signoff]").expect("signoff");
    // Only the confidence matrix is left.
    let text = with_confidence(CONFIDENCE_ROWS);
    let text = format!("{}{}", &text[..start], &text[end..]);
    assert_eq!(invalid_field(check(&text)).as_deref(), Some("matrices"));
}

#[test]
fn unknown_band_in_action_condition_is_rejected() {
    let text = format!(
        "{MINIMAL}\n[[actions]]\nid = \"review\"\ntext = \"Review it\"\ntags = []\nprecedence = \"detect\"\napplies_when = {{ band_in = [\"act\", \"severe\"] }}\n"
    );
    assert_eq!(
        invalid_field(check(&text)).as_deref(),
        Some("actions[0].applies_when.band_in[1]")
    );
}

#[test]
fn unscored_fails_defaults_true() {
    let policy = parse_policy(MINIMAL).expect("MINIMAL parses");
    assert!(policy.signoff.unscored_fails);
}

#[test]
fn rank_at_least_above_highest_band_rank_is_rejected() {
    let text = format!(
        "{MINIMAL}\n[[actions]]\nid = \"probe\"\ntext = \"Probe\"\ntags = []\nprecedence = \"detect\"\nrequired_when = {{ rank_at_least = 2 }}\n"
    );
    assert_eq!(
        invalid_field(check(&text)).as_deref(),
        Some("actions[0].required_when.rank_at_least")
    );
}

#[test]
fn rank_at_least_at_highest_band_rank_is_accepted() {
    let text = format!(
        "{MINIMAL}\n[[actions]]\nid = \"probe\"\ntext = \"Probe\"\ntags = []\nprecedence = \"detect\"\napplies_when = {{ rank_at_least = 1 }}\n"
    );
    assert_eq!(check(&text), Ok(()));
}
