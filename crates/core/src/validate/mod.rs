//! Functional blueprint validation.
//!
//! Port of `src/validate.py` — foundation types and top-level `validate()` dispatcher.

pub mod belt_flow;
pub mod inserters;
mod fluids;
pub mod power;
pub mod underground;

pub use fluids::{check_fluid_port_connectivity, check_pipe_isolation};

pub mod belt_structural;

use thiserror::Error;

use crate::models::{LayoutResult, SolverResult};
use power::check_power_coverage;

use belt_flow::{
    check_belt_connectivity, check_belt_direction_continuity, check_belt_flow_path,
    check_belt_flow_reachability, check_belt_junctions, check_belt_network_topology,
    check_underground_belt_entry_sideload, check_underground_belt_pairs,
    check_underground_belt_sideloading,
};

/// Layout style: affects which validation checks run and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutStyle {
    /// Constraint-based spaghetti layout (default).
    #[default]
    Spaghetti,
    /// Deterministic row-based main-bus layout.
    Bus,
}

/// Severity level of a single validation finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// A single validation finding, mirroring Python's `ValidationIssue` dataclass.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationIssue {
    pub severity: Severity,
    /// Category tag, e.g. `"pipe-isolation"`, `"fluid-connectivity"`, `"inserter"`, `"power"`.
    pub category: String,
    pub message: String,
    /// Optional grid position associated with the issue.
    pub x: Option<i32>,
    pub y: Option<i32>,
}

impl ValidationIssue {
    /// Construct a new issue without an associated position.
    pub fn new(severity: Severity, category: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity,
            category: category.into(),
            message: message.into(),
            x: None,
            y: None,
        }
    }

    /// Construct a new issue with an associated grid position.
    pub fn with_pos(
        severity: Severity,
        category: impl Into<String>,
        message: impl Into<String>,
        x: i32,
        y: i32,
    ) -> Self {
        Self {
            severity,
            category: category.into(),
            message: message.into(),
            x: Some(x),
            y: Some(y),
        }
    }
}

/// Raised when critical validation issues block blueprint generation.
///
/// Mirrors Python's `ValidationError` exception.
#[derive(Debug, Error)]
#[error("Validation failed:\n{}", format_issues(.issues))]
pub struct ValidationError {
    pub issues: Vec<ValidationIssue>,
}

impl ValidationError {
    pub fn new(issues: Vec<ValidationIssue>) -> Self {
        Self { issues }
    }
}

fn format_issues(issues: &[ValidationIssue]) -> String {
    issues
        .iter()
        .map(|i| format!("  [{}] {}", i.severity.as_str(), i.message))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run all functional validation checks on a layout.
///
/// Returns a list of issues found.  Returns `Err(ValidationError)` if any
/// error-severity issues are present.
pub fn validate(
    layout_result: &LayoutResult,
    solver_result: Option<&SolverResult>,
    layout_style: LayoutStyle,
) -> Result<Vec<ValidationIssue>, ValidationError> {
    let mut issues: Vec<ValidationIssue> = Vec::new();

    issues.extend(check_power_coverage(layout_result));
    issues.extend(inserters::check_inserter_chains(layout_result, solver_result));
    issues.extend(inserters::check_inserter_direction(layout_result));
    issues.extend(check_pipe_isolation(layout_result));
    issues.extend(check_fluid_port_connectivity(layout_result, layout_style));
    issues.extend(check_belt_connectivity(layout_result, solver_result));
    issues.extend(check_belt_flow_path(layout_result, solver_result, layout_style));
    issues.extend(check_belt_direction_continuity(layout_result));
    issues.extend(belt_structural::check_belt_throughput(layout_result));
    issues.extend(belt_structural::check_output_belt_coverage(layout_result, solver_result));
    if layout_style == LayoutStyle::Spaghetti {
        issues.extend(check_belt_network_topology(layout_result, solver_result));
    }
    issues.extend(check_belt_junctions(layout_result));
    issues.extend(check_underground_belt_pairs(layout_result));
    issues.extend(check_underground_belt_sideloading(layout_result));
    issues.extend(check_underground_belt_entry_sideload(layout_result));
    issues.extend(belt_structural::check_belt_dead_ends(layout_result));
    issues.extend(belt_structural::check_belt_loops(layout_result));
    issues.extend(belt_structural::check_belt_item_isolation(layout_result));
    issues.extend(belt_structural::check_belt_inserter_conflict(layout_result));
    issues.extend(check_belt_flow_reachability(layout_result, solver_result, layout_style));
    issues.extend(belt_structural::check_lane_throughput(layout_result, solver_result));

    let errors: Vec<ValidationIssue> = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .cloned()
        .collect();
    if !errors.is_empty() {
        return Err(ValidationError::new(errors));
    }

    Ok(issues)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EntityDirection, LayoutResult, PlacedEntity};

    fn empty_layout() -> LayoutResult {
        LayoutResult {
            entities: vec![],
            width: 0,
            height: 0,
        }
    }

    fn layout_with_machine() -> LayoutResult {
        LayoutResult {
            entities: vec![PlacedEntity {
                name: "assembling-machine-1".to_string(),
                x: 0,
                y: 0,
                direction: EntityDirection::North,
                recipe: Some("iron-gear-wheel".to_string()),
                io_type: None,
                carries: None,
                mirror: false,
                segment_id: None,
            }],
            width: 10,
            height: 10,
        }
    }

    #[test]
    fn severity_as_str() {
        assert_eq!(Severity::Error.as_str(), "error");
        assert_eq!(Severity::Warning.as_str(), "warning");
    }

    #[test]
    fn issue_new_has_no_position() {
        let issue = ValidationIssue::new(Severity::Error, "pipe-isolation", "test message");
        assert_eq!(issue.severity, Severity::Error);
        assert_eq!(issue.category, "pipe-isolation");
        assert_eq!(issue.message, "test message");
        assert_eq!(issue.x, None);
        assert_eq!(issue.y, None);
    }

    #[test]
    fn issue_with_pos_stores_coordinates() {
        let issue = ValidationIssue::with_pos(Severity::Warning, "power", "no pole", 3, 7);
        assert_eq!(issue.severity, Severity::Warning);
        assert_eq!(issue.x, Some(3));
        assert_eq!(issue.y, Some(7));
    }

    #[test]
    fn validation_error_contains_issues() {
        let issues = vec![
            ValidationIssue::new(Severity::Error, "pipe-isolation", "fluids merged"),
            ValidationIssue::new(Severity::Error, "power", "no coverage"),
        ];
        let err = ValidationError::new(issues.clone());
        assert_eq!(err.issues.len(), 2);
        assert_eq!(err.issues[0].category, "pipe-isolation");
    }

    #[test]
    fn validation_error_message_format() {
        let issues = vec![ValidationIssue::new(Severity::Error, "power", "no pole nearby")];
        let err = ValidationError::new(issues);
        let msg = err.to_string();
        assert!(msg.contains("Validation failed:"));
        assert!(msg.contains("[error]"));
        assert!(msg.contains("no pole nearby"));
    }

    #[test]
    fn validate_empty_layout_returns_ok_with_no_poles_warning() {
        let lr = empty_layout();
        let result = validate(&lr, None, LayoutStyle::Spaghetti);
        assert!(result.is_ok());
        let issues = result.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, "power");
    }

    #[test]
    fn validate_with_machine_returns_errors() {
        let lr = layout_with_machine();
        let result = validate(&lr, None, LayoutStyle::Bus);
        assert!(result.is_err(), "expected errors for a machine with no belts");
    }

    #[test]
    fn validate_default_layout_style_is_spaghetti() {
        assert_eq!(LayoutStyle::default(), LayoutStyle::Spaghetti);
    }

    #[test]
    fn layout_style_equality() {
        assert_eq!(LayoutStyle::Bus, LayoutStyle::Bus);
        assert_ne!(LayoutStyle::Bus, LayoutStyle::Spaghetti);
    }
}
