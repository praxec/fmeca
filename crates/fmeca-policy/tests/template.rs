//! The policy template shipped with the `fmeca-policy` skill
//! (`skills/fmeca-policy/policy-template.toml`) is a complete, valid policy,
//! with and without its optional confidence block.

use fmeca_policy::{parse_policy, validate};

fn template() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../skills/fmeca-policy/policy-template.toml"
    ))
    .expect("skills/fmeca-policy/policy-template.toml")
}

#[test]
fn policy_template_parses_and_validates() {
    assert_eq!(parse_policy(&template()).and_then(|p| validate(&p)), Ok(()));
}

#[test]
fn policy_template_with_confidence_block_enabled_validates() {
    let enabled = template().replace("#> ", "");
    let result = parse_policy(&enabled).and_then(|p| {
        validate(&p)?;
        Ok(p.matrices.contains_key("confidence"))
    });
    assert_eq!(result, Ok(true));
}
