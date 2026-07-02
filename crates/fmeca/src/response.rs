//! Remediation-magnitude derivation: from a failure mode's
//! computed criticality + a breadth signal, derive HOW big the response must be.
//!
//! This is deterministic — the kernel owns the rule, not the model. The model
//! only optionally names a `scope` observation (how broad the change surface is);
//! the [`ResponseClass`] itself is computed here.
//!
//! ## The rule (documented & tested)
//!
//! Inputs: the failure mode's raw [`Criticality`], its [`Domain`], and an
//! optional [`Scope`] breadth observation.
//!
//!  - **High** criticality + `architecture` domain (or `scope = structural`)
//!    → [`ResponseClass::ReArchitecture`] — the magnitude is structural.
//!  - **High** criticality + `runtime`/`delivery`/`ux` domain
//!    → [`ResponseClass::Restructure`].
//!  - **Medium** criticality → [`ResponseClass::Restructure`].
//!  - **Low** criticality → [`ResponseClass::MinorFix`].
//!  - not yet scored (`None`) → `None` (nothing to size yet).
//!
//! The optional `scope` sharpens it without breaking determinism: a
//! `structural` scope promotes a High mode to re-architecture regardless of
//! domain, and a `localized` scope on a Medium mode is still a restructure (the
//! criticality floor dominates so nothing High/Medium is under-sized).

use serde::{Deserialize, Serialize};

use crate::model::{Criticality, Domain, Level};

/// Optional caller-supplied breadth of the change surface.
/// Sharpens [`response_class`] but never makes it less conservative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Confined to one component / a small surface.
    Localized,
    /// Spans multiple components or layers.
    CrossCutting,
    /// Touches the system's structure / boundaries.
    Structural,
}

/// The deterministically-derived magnitude of the remediation a failure mode
/// demands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseClass {
    /// Rework the structure / boundaries.
    ReArchitecture,
    /// A non-trivial restructuring within existing boundaries.
    Restructure,
    /// A small, localized fix.
    MinorFix,
}

/// Derive the [`ResponseClass`] from raw criticality + domain + optional scope
///. Deterministic; `None` until the mode is scored.
pub fn response_class(
    criticality: Option<Criticality>,
    domain: Domain,
    scope: Option<Scope>,
) -> Option<ResponseClass> {
    let criticality = criticality?;
    Some(match criticality {
        Level::High => {
            if domain == Domain::Architecture || scope == Some(Scope::Structural) {
                ResponseClass::ReArchitecture
            } else {
                ResponseClass::Restructure
            }
        }
        Level::Medium => ResponseClass::Restructure,
        Level::Low => ResponseClass::MinorFix,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Domain::{Architecture, Delivery, Runtime, Ux};
    use crate::model::Level::{High, Low, Medium};

    #[test]
    fn unscored_has_no_response_class() {
        assert_eq!(response_class(None, Runtime, None), None);
        assert_eq!(
            response_class(None, Architecture, Some(Scope::Structural)),
            None
        );
    }

    #[test]
    fn high_architecture_is_re_architecture() {
        assert_eq!(
            response_class(Some(High), Architecture, None),
            Some(ResponseClass::ReArchitecture)
        );
    }

    #[test]
    fn high_runtime_or_delivery_is_restructure() {
        assert_eq!(
            response_class(Some(High), Runtime, None),
            Some(ResponseClass::Restructure)
        );
        assert_eq!(
            response_class(Some(High), Delivery, None),
            Some(ResponseClass::Restructure)
        );
        assert_eq!(
            response_class(Some(High), Ux, None),
            Some(ResponseClass::Restructure)
        );
    }

    #[test]
    fn structural_scope_promotes_high_to_re_architecture() {
        assert_eq!(
            response_class(Some(High), Runtime, Some(Scope::Structural)),
            Some(ResponseClass::ReArchitecture)
        );
    }

    #[test]
    fn medium_is_restructure_regardless_of_domain_or_scope() {
        for d in [Ux, Runtime, Architecture, Delivery] {
            assert_eq!(
                response_class(Some(Medium), d, None),
                Some(ResponseClass::Restructure)
            );
        }
        assert_eq!(
            response_class(Some(Medium), Runtime, Some(Scope::Localized)),
            Some(ResponseClass::Restructure)
        );
    }

    #[test]
    fn low_is_minor_fix() {
        assert_eq!(
            response_class(Some(Low), Architecture, None),
            Some(ResponseClass::MinorFix)
        );
        assert_eq!(
            response_class(Some(Low), Runtime, Some(Scope::CrossCutting)),
            Some(ResponseClass::MinorFix)
        );
    }
}
