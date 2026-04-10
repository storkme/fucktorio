//! Layout snapshot format — self-describing debug artifact.
//!
//! Format: `"fls1" + base64(gzip(JSON))`. Captures everything needed to
//! reproduce a view of a layout without re-running the pipeline.

use std::io::Read;
use std::io::Write;
use std::path::Path;

use base64::Engine;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};

use crate::models::{LayoutResult, SolverResult};
use crate::trace::TraceEvent;
use crate::validate::ValidationIssue;

/// Magic prefix for v1 snapshots.
const MAGIC: &str = "fls1";

// ---------------------------------------------------------------------------
// Snapshot types
// ---------------------------------------------------------------------------

/// Origin of a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotSource {
    Test,
    Manual,
    Ci,
}

/// Parameters that produced the snapshot — enough to re-run the pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotParams {
    pub item: String,
    pub rate: f64,
    pub machine: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub belt_tier: Option<String>,
    pub inputs: Vec<String>,
}

/// Human-readable provenance metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnapshotContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_sha: Option<String>,
}

/// Validation results captured in the snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotValidation {
    pub issues: Vec<ValidationIssue>,
    /// True if the validator panicked or timed out.
    #[serde(default)]
    pub truncated: bool,
}

/// Trace data captured in the snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotTrace {
    pub events: Vec<TraceEvent>,
    /// False if the snapshot was captured mid-pipeline (e.g. at timeout).
    #[serde(default = "default_true")]
    pub complete: bool,
}

fn default_true() -> bool {
    true
}

/// Complete layout snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutSnapshot {
    pub version: u32,
    pub created_at: String,
    pub source: SnapshotSource,
    pub params: SnapshotParams,
    #[serde(default)]
    pub context: SnapshotContext,
    pub layout: LayoutResult,
    pub validation: SnapshotValidation,
    pub trace: SnapshotTrace,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solver: Option<SolverResult>,
}

impl LayoutSnapshot {
    /// Create a snapshot from a completed pipeline run.
    pub fn from_run(
        source: SnapshotSource,
        params: SnapshotParams,
        context: SnapshotContext,
        layout: LayoutResult,
        issues: Vec<ValidationIssue>,
        truncated: bool,
        trace_events: Vec<TraceEvent>,
        trace_complete: bool,
        solver: Option<SolverResult>,
    ) -> Self {
        Self {
            version: 1,
            created_at: chrono_now(),
            source,
            params,
            context,
            layout,
            validation: SnapshotValidation {
                issues,
                truncated,
            },
            trace: SnapshotTrace {
                events: trace_events,
                complete: trace_complete,
            },
            solver,
        }
    }

    /// Encode to the wire format: `"fls1" + base64(gzip(json))`.
    pub fn encode(&self) -> Result<String, SnapshotError> {
        let json = serde_json::to_vec(self).map_err(SnapshotError::Json)?;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&json)
            .map_err(SnapshotError::Io)?;
        let compressed = encoder.finish().map_err(SnapshotError::Io)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&compressed);
        Ok(format!("{MAGIC}{b64}"))
    }

    /// Decode from the wire format.
    pub fn decode(input: &str) -> Result<Self, SnapshotError> {
        if !input.starts_with(MAGIC) {
            return Err(SnapshotError::BadMagic {
                expected: MAGIC.to_string(),
                found: input.chars().take(4).collect(),
            });
        }
        let b64 = &input[MAGIC.len()..];
        let compressed = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(SnapshotError::Base64)?;
        let mut json_bytes = Vec::new();
        GzDecoder::new(&compressed[..])
            .read_to_end(&mut json_bytes)
            .map_err(SnapshotError::Io)?;
        let snapshot: Self =
            serde_json::from_slice(&json_bytes).map_err(SnapshotError::Json)?;
        Ok(snapshot)
    }

    /// Write the encoded snapshot to a file.
    pub fn write_to_file(&self, path: &Path) -> std::io::Result<()> {
        let encoded = self
            .encode()
            .map_err(std::io::Error::other)?;
        std::fs::write(path, encoded)
    }

    /// Read and decode a snapshot from a file.
    pub fn read_from_file(path: &Path) -> Result<Self, SnapshotError> {
        let contents = std::fs::read_to_string(path).map_err(SnapshotError::Io)?;
        Self::decode(&contents)
    }
}

/// Errors that can occur during snapshot encode/decode.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("bad magic prefix: expected {expected:?}, found {found:?}")]
    BadMagic { expected: String, found: String },
    #[error("base64 decode: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// ISO 8601 timestamp without external dependencies.
fn chrono_now() -> String {
    // Use std::time and format manually — good enough for timestamps.
    // Format: YYYY-MM-DDTHH:MM:SSZ (UTC, no subseconds)
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();

    // Calculate date components from unix timestamp
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    // Calculate year/month/day from days since epoch
    let (year, month, day) = days_to_date(days);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_date(days_since_epoch: u64) -> (u64, u64, u64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days_since_epoch + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PlacedEntity;

    fn test_snapshot() -> LayoutSnapshot {
        LayoutSnapshot::from_run(
            SnapshotSource::Test,
            SnapshotParams {
                item: "iron-gear-wheel".into(),
                rate: 10.0,
                machine: "assembling-machine-1".into(),
                belt_tier: None,
                inputs: vec!["iron-plate".into()],
            },
            SnapshotContext {
                test_name: Some("tier1_iron_gear_wheel".into()),
                label: None,
                git_sha: None,
            },
            LayoutResult {
                entities: vec![PlacedEntity {
                    name: "assembling-machine-1".into(),
                    x: 0,
                    y: 0,
                    ..Default::default()
                }],
                width: 1,
                height: 1,
                ..Default::default()
            },
            vec![],
            false,
            vec![],
            true,
            None,
        )
    }

    #[test]
    fn round_trip() {
        let snap = test_snapshot();
        let encoded = snap.encode().unwrap();
        assert!(encoded.starts_with("fls1"), "should have magic prefix");
        let decoded = LayoutSnapshot::decode(&encoded).unwrap();
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.params.item, "iron-gear-wheel");
        assert_eq!(decoded.layout.entities.len(), 1);
        assert!(decoded.trace.complete);
        assert!(decoded.solver.is_none());
    }

    #[test]
    fn bad_magic_rejected() {
        let err = LayoutSnapshot::decode("abcdpayload").unwrap_err();
        assert!(
            matches!(err, SnapshotError::BadMagic { .. }),
            "expected BadMagic, got {err:?}"
        );
    }

    #[test]
    fn truncated_flag_preserved() {
        let mut snap = test_snapshot();
        snap.validation.truncated = true;
        snap.trace.complete = false;
        let encoded = snap.encode().unwrap();
        let decoded = LayoutSnapshot::decode(&encoded).unwrap();
        assert!(decoded.validation.truncated);
        assert!(!decoded.trace.complete);
    }
}
