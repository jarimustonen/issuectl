//! Transactional patch input loading and parsing shared by the CLI surfaces.

use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::{mutate, slug};

const ACCEPTED_FORMS: &str =
    "a patch file path, or `-` to read YAML/JSON from stdin (use `./-` for a file named `-`)";

/// Whether a caller requires optimistic concurrency for this patch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpectedVersionPolicy {
    Optional,
    Required,
}

/// Read and parse one transactional patch from a path or stdin.
pub fn read_and_parse(
    input: &Path,
    stdin: &mut dyn Read,
    expected_version: ExpectedVersionPolicy,
) -> Result<(String, mutate::UpdateIssueRequest)> {
    let (text, source) = read(input, stdin)?;
    if text.trim().is_empty() {
        bail!("patch input {source} is empty; expected a YAML/JSON mapping");
    }
    parse(&text, expected_version).with_context(|| format!("cannot process patch {source}"))
}

fn read(input: &Path, stdin: &mut dyn Read) -> Result<(String, String)> {
    if input.as_os_str() == OsStr::new("-") {
        let mut text = String::new();
        stdin
            .read_to_string(&mut text)
            .context("cannot read patch from stdin")?;
        return Ok((text, "from stdin".to_string()));
    }

    if input
        .to_str()
        .map(str::trim_start)
        .is_some_and(|value| value.starts_with('{') || value.starts_with('['))
    {
        bail!("inline patch input is not accepted; accepted forms: {ACCEPTED_FORMS}");
    }

    let text = fs::read_to_string(input)
        .with_context(|| format!("cannot read patch file {}", input.display()))?;
    Ok((text, format!("in {}", input.display())))
}

/// Parse YAML (including JSON, which is a YAML subset) into the canonical update request.
pub fn parse(
    yaml_text: &str,
    expected_version: ExpectedVersionPolicy,
) -> Result<(String, mutate::UpdateIssueRequest)> {
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
        bail!("`dry_run` is a command-line flag, not a patch field; pass `--dry-run` on the command line");
    }
    let req: mutate::UpdateIssueRequest =
        serde_yaml::from_value(yaml).context("cannot parse patch fields")?;
    if expected_version == ExpectedVersionPolicy::Required {
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
    fn dash_reads_json_from_stdin() {
        let mut stdin = Cursor::new(r#"{"slug":"some-issue","priority":"high"}"#);
        let (slug, request) =
            read_and_parse(Path::new("-"), &mut stdin, ExpectedVersionPolicy::Optional).unwrap();
        assert_eq!(slug, "some-issue");
        assert!(matches!(request.priority, mutate::Patch::Set(ref value) if value == "high"));
    }

    #[test]
    fn stdin_read_failure_names_stdin() {
        let error = read_and_parse(
            Path::new("-"),
            &mut FailingReader,
            ExpectedVersionPolicy::Optional,
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("cannot read patch from stdin"));
        assert!(format!("{error:#}").contains("broken pipe source"));
    }

    #[test]
    fn empty_stdin_has_a_source_specific_error() {
        let error = read_and_parse(
            Path::new("-"),
            &mut Cursor::new([]),
            ExpectedVersionPolicy::Optional,
        )
        .unwrap_err();
        assert!(error.to_string().contains("from stdin is empty"));
    }

    #[test]
    fn inline_input_with_path_characters_names_every_accepted_form() {
        for input in [
            r#"{"slug":"some-issue","url":"https://example.test/a"}"#,
            r#"[{"slug":"some-issue"}]"#,
        ] {
            let error = read_and_parse(
                Path::new(input),
                &mut Cursor::new([]),
                ExpectedVersionPolicy::Optional,
            )
            .unwrap_err();
            let message = error.to_string();
            assert!(message.contains("patch file path"));
            assert!(message.contains("`-`"));
            assert!(message.contains("`./-`"));
            assert!(message.contains("inline patch input is not accepted"));
        }
    }

    #[test]
    fn missing_extensionless_path_keeps_file_diagnostic() {
        let error = read_and_parse(
            Path::new("missing-patch"),
            &mut Cursor::new([]),
            ExpectedVersionPolicy::Optional,
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("cannot read patch file missing-patch"));
    }

    #[test]
    fn parse_rejects_invalid_json_or_yaml() {
        let error = parse("{not valid", ExpectedVersionPolicy::Optional).unwrap_err();
        assert!(format!("{error:#}").contains("cannot parse as YAML or JSON"));
    }
}
