//! Transactional patch input loading and parsing shared by the CLI surfaces.

use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::{mutate, slug};

const ACCEPTED_FORMS: &str =
    "a patch file path, or `-` to read YAML/JSON from stdin (use `./-` for a file named `-`)";

/// Read and parse one transactional patch from a path or stdin.
pub fn read_and_parse(
    input: &Path,
    stdin: &mut dyn Read,
    json_output: bool,
) -> Result<(String, mutate::UpdateIssueRequest)> {
    let (text, source) = read(input, stdin)?;
    parse(&text, json_output).with_context(|| format!("cannot parse patch fields {source}"))
}

fn read(input: &Path, stdin: &mut dyn Read) -> Result<(String, String)> {
    if input.as_os_str() == OsStr::new("-") {
        let mut text = String::new();
        stdin
            .read_to_string(&mut text)
            .context("cannot read patch from stdin")?;
        return Ok((text, "from stdin".to_string()));
    }

    if !input.exists() && !looks_path_shaped(input) {
        bail!(
            "unsupported patch input {:?}; accepted forms: {ACCEPTED_FORMS}; inline JSON is not accepted",
            input.as_os_str()
        );
    }

    let text = fs::read_to_string(input)
        .with_context(|| format!("cannot read patch file {}", input.display()))?;
    Ok((text, format!("in {}", input.display())))
}

fn looks_path_shaped(input: &Path) -> bool {
    input.is_absolute()
        || input.extension().is_some()
        || input.components().count() > 1
        || input
            .as_os_str()
            .to_string_lossy()
            .contains(std::path::MAIN_SEPARATOR)
}

/// Parse YAML (including JSON, which is a YAML subset) into the canonical update request.
pub fn parse(yaml_text: &str, json_output: bool) -> Result<(String, mutate::UpdateIssueRequest)> {
    let mut yaml: serde_yaml::Value =
        serde_yaml::from_str(yaml_text).context("cannot parse as YAML or JSON")?;
    let map = yaml
        .as_mapping_mut()
        .ok_or_else(|| anyhow::anyhow!("patch must be a YAML/JSON mapping at the top level"))?;
    let slug = map
        .remove(serde_yaml::Value::String("slug".into()))
        .ok_or_else(|| anyhow::anyhow!("patch must declare `slug:`"))?
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("`slug:` must be a string"))?
        .to_string();
    if !slug::is_valid(&slug) {
        bail!("invalid slug shape: {slug:?}");
    }
    if map.contains_key(serde_yaml::Value::String("dry_run".into())) {
        bail!("`dry_run` is a CLI flag; use `issuectl update --patch-file <PATH|-> --dry-run`, not a patch field");
    }
    let req: mutate::UpdateIssueRequest =
        serde_yaml::from_value(yaml).context("cannot parse patch fields")?;
    if json_output {
        match req.expected_version.as_deref() {
            Some(v) if !v.trim().is_empty() && v.trim() == v => {}
            _ => bail!(
                "patch must include a non-empty `expected_version:` when invoked with --json \
                 (per design D4=B); fetch with `issuectl show <slug> --json`"
            ),
        }
    }
    Ok((slug, req))
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor, Read};

    use super::*;

    struct FailingReader;

    impl Read for FailingReader {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("broken pipe source"))
        }
    }

    #[test]
    fn dash_reads_stdin() {
        let mut stdin = Cursor::new("slug: some-issue\npriority: high\n");
        let (slug, request) = read_and_parse(Path::new("-"), &mut stdin, false).unwrap();
        assert_eq!(slug, "some-issue");
        assert!(matches!(request.priority, mutate::Patch::Set(ref value) if value == "high"));
    }

    #[test]
    fn stdin_read_failure_names_stdin() {
        let error = read_and_parse(Path::new("-"), &mut FailingReader, false).unwrap_err();
        assert!(format!("{error:#}").contains("cannot read patch from stdin"));
        assert!(format!("{error:#}").contains("broken pipe source"));
    }

    #[test]
    fn unsupported_input_names_every_accepted_form() {
        let error = read_and_parse(
            Path::new("{\"slug\":\"some-issue\"}"),
            &mut Cursor::new([]),
            false,
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("patch file path"));
        assert!(message.contains("`-`"));
        assert!(message.contains("`./-`"));
        assert!(message.contains("inline JSON is not accepted"));
    }

    #[test]
    fn missing_path_keeps_file_diagnostic() {
        let error = read_and_parse(Path::new("missing-patch.yaml"), &mut Cursor::new([]), false)
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("cannot read patch file missing-patch.yaml"));
    }

    #[test]
    fn parse_rejects_invalid_json_or_yaml() {
        let error = parse("{not valid", false).unwrap_err();
        assert!(format!("{error:#}").contains("cannot parse as YAML or JSON"));
    }
}
