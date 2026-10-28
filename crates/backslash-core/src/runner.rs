//! Starting a command: shared by the launcher window and the CLI.

use anyhow::{bail, Result};
use serde_json::{Map, Value};

use crate::config::Config;
use crate::host::{launch_for, Pending, Session};
use crate::manifest::CommandDef;
use crate::paths;
use crate::protocol::{LaunchType, StartParams};
use crate::registry::Extension;

/// Validates raw `name → text` argument values against the manifest.
pub fn parse_arguments(command: &CommandDef, raw: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for arg in &command.arguments {
        let text = match raw.get(&arg.name) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        };
        let value = arg.parse(&text)?;
        if !value.is_null() {
            out.insert(arg.name.clone(), value);
        }
    }
    Ok(out)
}

/// Spawns the extension process and sends `start`. `arguments` must already
/// be validated with [`parse_arguments`].
pub fn start(
    ext: &Extension,
    command: &CommandDef,
    config: &Config,
    arguments: Map<String, Value>,
    query: &str,
    launch_type: LaunchType,
) -> Result<(Session, Pending)> {
    let (preferences, missing) = config.preferences(&ext.manifest);
    if !missing.is_empty() {
        bail!(
            "{} needs preferences: {} (set them in Backslash → Settings → Extensions)",
            ext.manifest.extension.title,
            missing.join(", ")
        );
    }
    let launch = launch_for(ext, command, &paths::sdk_dir())?;
    let session = Session::spawn(&ext.manifest.extension.id, &launch, &paths::data_dir().join("storage"))?;
    let params =
        StartParams { command: command.name.clone(), arguments, preferences, query: query.to_string(), launch_type };
    let pending = session.request("start", serde_json::to_value(params)?)?;
    Ok((session, pending))
}
