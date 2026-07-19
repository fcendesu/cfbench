use crate::error::OutputError;
use crate::results::RunResult;

/// Serializes exactly one versioned result envelope.
pub fn render_json(result: &RunResult) -> Result<String, OutputError> {
    Ok(serde_json::to_string_pretty(result)?)
}
