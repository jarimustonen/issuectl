//! Read-only inspection of issuectl's persistent configuration.
//!
//! The schema at [`crate::schema::SCHEMA_RELATIVE_PATH`] is issuectl's
//! configuration file. Its values are resolved by [`crate::schema::load`]: a
//! repo declaration replaces the corresponding built-in default. This module
//! exposes that existing resolution without changing it, including the source
//! of every reported value.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;

use crate::schema;

/// Where an effective schema value came from.
///
/// issuectl currently has only repo-file and built-in-default layers. Flags
/// and environment variables are invocation settings, not persistent schema
/// settings, so they are intentionally not reported as configuration sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueSource {
    File,
    Default,
}

impl ValueSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Default => "default",
        }
    }
}

/// One effective configuration value and its provenance.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedValue {
    pub value: JsonValue,
    pub source: ValueSource,
}

/// The `config show` machine-readable result.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigShow {
    /// The schema configuration path, whether or not the optional file exists.
    pub path: PathBuf,
    /// Effective values, indexed by stable dotted schema keys.
    pub values: BTreeMap<String, ResolvedValue>,
}

/// Return the inspectable location of issuectl's schema configuration.
pub fn path(root: &Path) -> PathBuf {
    schema::schema_path(root)
}

/// Resolve schema configuration and report each effective value with its
/// per-key source. This deliberately calls the normal schema loader, so the
/// reported values have exactly the same merge and validation behavior as the
/// rest of the CLI.
pub fn show(root: &Path) -> Result<ConfigShow> {
    let path = path(root);
    let effective = schema::load(root)?;
    let raw = load_raw_file(&path)?;
    let mut values = BTreeMap::new();

    insert(
        &mut values,
        "schema.version",
        &effective.version,
        source_for(&raw, "version"),
    )?;
    insert(
        &mut values,
        "schema.dod.strict",
        &effective.dod.strict,
        nested_source_for(&raw, "dod", "strict"),
    )?;

    for (name, spec) in &effective.fields {
        insert(
            &mut values,
            &format!("schema.fields.{name}"),
            spec,
            map_entry_source_for(&raw, "fields", name),
        )?;
    }
    for (name, sections) in &effective.body_sections {
        insert(
            &mut values,
            &format!("schema.body_sections.{name}"),
            sections,
            map_entry_source_for(&raw, "body_sections", name),
        )?;
    }
    for (name, class) in &effective.status_classes {
        insert(
            &mut values,
            &format!("schema.status_classes.{name}"),
            class,
            map_entry_source_for(&raw, "status_classes", name),
        )?;
    }
    for (name, target) in &effective.status_aliases {
        insert(
            &mut values,
            &format!("schema.status_aliases.{name}"),
            target,
            map_entry_source_for(&raw, "status_aliases", name),
        )?;
    }
    for (name, target) in &effective.type_aliases {
        insert(
            &mut values,
            &format!("schema.type_aliases.{name}"),
            target,
            map_entry_source_for(&raw, "type_aliases", name),
        )?;
    }

    Ok(ConfigShow { path, values })
}

fn insert<T: Serialize>(
    values: &mut BTreeMap<String, ResolvedValue>,
    key: &str,
    value: &T,
    source: ValueSource,
) -> Result<()> {
    values.insert(
        key.to_string(),
        ResolvedValue {
            value: serde_json::to_value(value).context("serializing effective configuration")?,
            source,
        },
    );
    Ok(())
}

fn load_raw_file(path: &Path) -> Result<Option<YamlValue>> {
    if !path.is_file() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let value =
        serde_yaml::from_str(&text).with_context(|| format!("cannot parse {}", path.display()))?;
    Ok(Some(value))
}

fn yaml_key(key: &str) -> YamlValue {
    YamlValue::String(key.to_string())
}

fn source_for(raw: &Option<YamlValue>, key: &str) -> ValueSource {
    raw.as_ref()
        .and_then(YamlValue::as_mapping)
        .and_then(|map| map.get(&yaml_key(key)))
        .map_or(ValueSource::Default, |_| ValueSource::File)
}

fn nested_source_for(raw: &Option<YamlValue>, parent: &str, key: &str) -> ValueSource {
    raw.as_ref()
        .and_then(YamlValue::as_mapping)
        .and_then(|map| map.get(&yaml_key(parent)))
        .and_then(YamlValue::as_mapping)
        .and_then(|map| map.get(&yaml_key(key)))
        .map_or(ValueSource::Default, |_| ValueSource::File)
}

fn map_entry_source_for(raw: &Option<YamlValue>, parent: &str, key: &str) -> ValueSource {
    nested_source_for(raw, parent, key)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn path_is_the_repo_schema_location() {
        let tmp = tempdir().unwrap();
        assert_eq!(path(tmp.path()), tmp.path().join("issues/.schema.yaml"));
    }

    #[test]
    fn missing_schema_reports_builtin_defaults() {
        let tmp = tempdir().unwrap();
        let report = show(tmp.path()).unwrap();

        assert_eq!(report.path, tmp.path().join("issues/.schema.yaml"));
        assert_eq!(
            report.values["schema.fields.status"].source,
            ValueSource::Default
        );
        assert_eq!(report.values["schema.dod.strict"].value, false);
        assert_eq!(
            report.values["schema.dod.strict"].source,
            ValueSource::Default
        );
    }

    #[test]
    fn file_overrides_are_reported_per_value() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\ndod:\n  strict: true\nfields:\n  priority:\n    required: false\n",
        )
        .unwrap();

        let report = show(tmp.path()).unwrap();
        assert_eq!(report.values["schema.version"].source, ValueSource::File);
        assert_eq!(report.values["schema.dod.strict"].source, ValueSource::File);
        assert_eq!(
            report.values["schema.fields.priority"].source,
            ValueSource::File
        );
        assert_eq!(
            report.values["schema.fields.status"].source,
            ValueSource::Default
        );
    }
}
