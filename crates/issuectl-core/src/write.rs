use std::fs;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::Local;
use serde_yaml::{Mapping, Value};

pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// Generate a kebab-case slug from a title. Preserves Unicode alphanumerics
/// (Finnish ä/ö stay as-is). Truncates to `max_words` words.
pub fn slugify(input: &str, max_words: usize) -> String {
    let lowered = input.to_lowercase();
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for ch in lowered.chars() {
        if ch.is_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words.truncate(max_words);
    words.join("-")
}

pub struct ItemFile {
    pub frontmatter: Mapping,
    pub body: String,
}

/// Read item.md, split into frontmatter Mapping and body string.
pub fn read_item(path: &Path) -> Result<ItemFile> {
    let text =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let (fm_text, body) = split_text(&text);
    let fm: Mapping = match fm_text {
        Some(yaml) if !yaml.trim().is_empty() => serde_yaml::from_str(yaml)
            .with_context(|| format!("cannot parse frontmatter in {}", path.display()))?,
        _ => Mapping::new(),
    };
    let raw_body = body.unwrap_or("");
    // Preserve at most one leading blank line; collapse extras.
    let body_str = if raw_body.starts_with('\n') {
        let rest = raw_body.trim_start_matches('\n');
        format!("\n{rest}")
    } else {
        raw_body.to_string()
    };
    Ok(ItemFile {
        frontmatter: fm,
        body: body_str,
    })
}

pub fn write_item(path: &Path, item: &ItemFile) -> Result<()> {
    let out = serialize_item(item)?;
    fs::write(path, out).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

/// Serialize an `ItemFile` into the on-disk byte sequence (frontmatter +
/// body), without writing it. Used by `mutate::write_item_atomic` so the
/// serialized bytes can be staged in a `.issuectl-tmp-*` file and
/// rename-persisted under the repo `flock`.
pub fn serialize_item(item: &ItemFile) -> Result<String> {
    let yaml = serialize_frontmatter(&item.frontmatter)?;
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&yaml);
    if !yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(&item.body);
    if !item.body.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

fn split_text(text: &str) -> (Option<&str>, Option<&str>) {
    let trimmed = text.trim_start();
    if !trimmed.starts_with("---") {
        return (None, Some(text));
    }
    let rest = &trimmed[3..];
    if let Some(end) = rest.find("\n---") {
        let yaml = &rest[..end];
        // Skip past the closing "\n---" plus one trailing newline (the line
        // terminator of the closing `---`). A blank line between fm and body
        // shows up as a *second* leading newline in the resulting `after`.
        let mut after_idx = end + 4;
        if rest.as_bytes().get(after_idx) == Some(&b'\n') {
            after_idx += 1;
        } else if rest.as_bytes().get(after_idx) == Some(&b'\r')
            && rest.as_bytes().get(after_idx + 1) == Some(&b'\n')
        {
            after_idx += 2;
        }
        let after = &rest[after_idx..];
        (Some(yaml), Some(after))
    } else {
        (None, Some(text))
    }
}

/// Serialize a mapping back to YAML, then convert simple string arrays to
/// flow style (`key: ["a", "b"]`) for readability.
fn serialize_frontmatter(map: &Mapping) -> Result<String> {
    let yaml = serde_yaml::to_string(map).context("cannot serialize frontmatter back to YAML")?;
    Ok(flowify_string_arrays(&yaml))
}

fn flowify_string_arrays(yaml: &str) -> String {
    let lines: Vec<&str> = yaml.lines().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some((indent, key)) = parse_empty_key(line) {
            let prefix_same = format!("{}- ", indent);
            let prefix_extra = format!("{}  - ", indent);
            let mut j = i + 1;
            let mut items: Vec<String> = Vec::new();
            let mut all_simple = true;
            while j < lines.len() {
                let next = lines[j];
                let value = if let Some(rest) = next.strip_prefix(&prefix_extra) {
                    rest
                } else if let Some(rest) = next.strip_prefix(&prefix_same) {
                    rest
                } else {
                    break;
                };
                if value.trim().is_empty() || is_complex_yaml_scalar(value) {
                    all_simple = false;
                    break;
                }
                items.push(value.to_string());
                j += 1;
            }
            if all_simple && !items.is_empty() {
                out.push_str(&format!("{}{}: [{}]\n", indent, key, items.join(", ")));
                i = j;
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
        i += 1;
    }
    out
}

fn parse_empty_key(line: &str) -> Option<(String, String)> {
    let indent: String = line
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let body = &line[indent.len()..];
    let stripped = body.strip_suffix(':')?;
    if stripped.is_empty() {
        return None;
    }
    if !stripped
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    Some((indent, stripped.to_string()))
}

fn is_complex_yaml_scalar(value: &str) -> bool {
    let mut in_quotes = None::<char>;
    let mut prev = '\0';
    for ch in value.chars() {
        match in_quotes {
            Some(q) if ch == q && prev != '\\' => in_quotes = None,
            None if ch == '"' || ch == '\'' => in_quotes = Some(ch),
            None if ch == ':' || ch == '{' || ch == '[' => return true,
            _ => {}
        }
        prev = ch;
    }
    false
}

// ── Frontmatter mutation helpers ────────────────────────────────────────────

pub fn set_string(map: &mut Mapping, key: &str, value: &str) {
    map.insert(
        Value::String(key.to_string()),
        Value::String(value.to_string()),
    );
}

pub fn remove_key(map: &mut Mapping, key: &str) {
    map.remove(Value::String(key.to_string()));
}

pub fn add_to_string_list(map: &mut Mapping, key: &str, value: &str) -> Result<()> {
    let key_val = Value::String(key.to_string());
    let entry = map
        .entry(key_val)
        .or_insert_with(|| Value::Sequence(Vec::new()));
    let seq = match entry {
        Value::Sequence(s) => s,
        _ => bail!("frontmatter field {key} is not a list"),
    };
    let new_item = Value::String(value.to_string());
    if !seq.iter().any(|v| v == &new_item) {
        seq.push(new_item);
    }
    Ok(())
}

pub fn remove_from_string_list(map: &mut Mapping, key: &str, value: &str) -> Result<()> {
    let key_val = Value::String(key.to_string());
    if let Some(Value::Sequence(seq)) = map.get_mut(&key_val) {
        seq.retain(|v| v.as_str().is_none_or(|s| s != value));
        if seq.is_empty() {
            map.remove(&key_val);
        }
    }
    Ok(())
}

pub fn add_commit(map: &mut Mapping, hash: &str, summary: &str) -> Result<()> {
    let key = Value::String("commits".to_string());
    let entry = map
        .entry(key)
        .or_insert_with(|| Value::Sequence(Vec::new()));
    let seq = match entry {
        Value::Sequence(s) => s,
        _ => bail!("frontmatter field commits is not a list"),
    };
    let mut commit = Mapping::new();
    commit.insert(
        Value::String("hash".into()),
        Value::String(hash.to_string()),
    );
    commit.insert(
        Value::String("summary".into()),
        Value::String(summary.to_string()),
    );
    seq.push(Value::Mapping(commit));
    Ok(())
}

// ── New-issue rendering ─────────────────────────────────────────────────────

pub struct NewIssueArgs<'a> {
    pub title: &'a str,
    pub issue_type: &'a str,
    pub priority: &'a str,
    pub reporter: Option<&'a str>,
    pub assignee: Option<&'a str>,
    pub owner: Option<&'a str>,
    pub epic: Option<&'a str>,
    pub labels: &'a [String],
    pub related: &'a [String],
    pub source: Option<&'a str>,
    pub description: Option<&'a str>,
    /// Custom frontmatter fields supplied by `issuectl new --field key=value`.
    /// Built-in fields (`type`, `priority`, ...) are reserved at the parser
    /// level (`parse_custom_field`), so these can only be names beyond the
    /// built-in set.
    pub custom_fields: &'a [(String, String)],
}

/// Build the frontmatter mapping for a new item. Split out from
/// `render_new_item` so callers (e.g. `do_new_locked`) can validate the
/// `Mapping` against the schema before serialization, avoiding a
/// round-trip through string parsing.
pub fn build_new_frontmatter(args: &NewIssueArgs<'_>) -> Mapping {
    let mut map = Mapping::new();
    let today = today();
    set_string(&mut map, "created", &today);
    set_string(&mut map, "updated", &today);
    set_string(&mut map, "type", args.issue_type);

    if args.issue_type == "epic" {
        if let Some(o) = args.owner {
            set_string(&mut map, "owner", o);
        }
    } else {
        if let Some(r) = args.reporter {
            set_string(&mut map, "reporter", r);
        }
        if let Some(a) = args.assignee {
            set_string(&mut map, "assignee", a);
        }
    }

    set_string(&mut map, "status", "open");
    set_string(&mut map, "priority", args.priority);

    if let Some(e) = args.epic {
        set_string(&mut map, "epic", e);
    }
    if !args.related.is_empty() {
        let seq: Vec<Value> = args
            .related
            .iter()
            .map(|s| Value::String(s.clone()))
            .collect();
        map.insert(Value::String("related".into()), Value::Sequence(seq));
    }
    if !args.labels.is_empty() {
        let seq: Vec<Value> = args
            .labels
            .iter()
            .map(|s| Value::String(s.clone()))
            .collect();
        map.insert(Value::String("labels".into()), Value::Sequence(seq));
    }

    for (key, value) in args.custom_fields {
        set_string(&mut map, key, value);
    }
    map
}

#[cfg(test)]
pub fn render_new_item(args: &NewIssueArgs<'_>) -> String {
    render_new_item_from_fm(args, &build_new_frontmatter(args))
}

pub fn render_new_item_from_fm(args: &NewIssueArgs<'_>, map: &Mapping) -> String {
    let yaml = serialize_frontmatter(map).expect("known-shape frontmatter must serialize");

    let mut body = String::new();
    body.push_str(&format!("# {}\n", args.title));
    body.push('\n');
    if let Some(s) = args.source {
        body.push_str(&format!("_Source: {}_\n\n", s));
    }
    body.push_str("## Description\n\n");
    if let Some(d) = args.description {
        body.push_str(d.trim_end());
        body.push('\n');
    }

    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&yaml);
    if !yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("---\n\n");
    out.push_str(&body);
    out
}

/// Path to a slug's flat-layout issue directory. The `folder` parameter
/// is unused and retained only to keep test call sites stable; it is
/// allowed to be any kanban-bucket label.
#[cfg(test)]
pub fn issue_dir(repo_root: &Path, _folder: &str, slug: &str) -> PathBuf {
    repo_root.join("issues").join(slug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_tmp(content: &str) -> (TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("item.md");
        fs::write(&path, content).unwrap();
        (tmp, path)
    }

    // ── slugify ────────────────────────────────────────────────────────────

    #[test]
    fn slugify_truncates_and_lowercases() {
        assert_eq!(slugify("Fix Login Redirect Bug", 3), "fix-login-redirect");
        assert_eq!(slugify("  hello, World!  ", 5), "hello-world");
    }

    #[test]
    fn slugify_preserves_finnish_chars() {
        assert_eq!(slugify("Käyttäjän virhe ÄÖ", 6), "käyttäjän-virhe-äö");
    }

    #[test]
    fn slugify_returns_empty_for_only_punctuation() {
        assert_eq!(slugify("", 6), "");
        assert_eq!(slugify("   ", 6), "");
        assert_eq!(slugify("!@#$%", 6), "");
    }

    #[test]
    fn slugify_with_zero_max_words_returns_empty() {
        assert_eq!(slugify("hello world", 0), "");
    }

    #[test]
    fn slugify_collapses_consecutive_separators() {
        assert_eq!(slugify("a  ---  b", 5), "a-b");
    }

    // ── flowify_string_arrays ──────────────────────────────────────────────

    #[test]
    fn flowify_collapses_simple_string_arrays() {
        let yaml = "labels:\n- infra\n- backend\nstatus: open\n";
        assert_eq!(
            flowify_string_arrays(yaml),
            "labels: [infra, backend]\nstatus: open\n"
        );
    }

    #[test]
    fn flowify_skips_mapping_arrays() {
        let yaml = "commits:\n- hash: abc\n  summary: fix\n";
        assert_eq!(flowify_string_arrays(yaml), yaml);
    }

    #[test]
    fn flowify_handles_extra_indented_items() {
        let yaml = "labels:\n  - infra\n  - backend\n";
        assert_eq!(flowify_string_arrays(yaml), "labels: [infra, backend]\n");
    }

    #[test]
    fn flowify_preserves_quoted_hash_strings() {
        // serde_yaml emits '#3' for "#3" since `#` would start a comment
        let yaml = "related:\n- '#3'\n- '#7'\n";
        assert_eq!(flowify_string_arrays(yaml), "related: ['#3', '#7']\n");
    }

    #[test]
    fn flowify_handles_single_item_arrays() {
        let yaml = "labels:\n- only\n";
        assert_eq!(flowify_string_arrays(yaml), "labels: [only]\n");
    }

    #[test]
    fn flowify_does_not_swallow_following_keys() {
        let yaml = "labels:\n- a\nstatus: open\nupdated: 2026-05-02\n";
        let result = flowify_string_arrays(yaml);
        assert!(result.contains("labels: [a]"));
        assert!(result.contains("status: open"));
        assert!(result.contains("updated: 2026-05-02"));
    }

    #[test]
    fn flowify_does_nothing_when_no_lists() {
        let yaml = "status: open\npriority: normal\n";
        assert_eq!(flowify_string_arrays(yaml), yaml);
    }

    // ── read_item / write_item ──────────────────────────────────────────────

    #[test]
    fn round_trip_preserves_unknown_fields() {
        let (_tmp, path) = write_tmp(
            "---\ncreated: 2026-01-01\ncustom: keep-me\nstatus: open\n---\n\n# Title\n\nBody\n",
        );
        let item = read_item(&path).unwrap();
        write_item(&path, &item).unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("custom: keep-me"));
        assert!(after.contains("status: open"));
        assert!(after.contains("# Title"));
    }

    #[test]
    fn round_trip_preserves_blank_line_before_body() {
        let (_tmp, path) = write_tmp("---\nstatus: open\n---\n\n# Title\n\nBody text\n");
        let item = read_item(&path).unwrap();
        write_item(&path, &item).unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("---\n\n# Title\n"));
    }

    #[test]
    fn round_trip_preserves_no_blank_line_before_body() {
        let (_tmp, path) = write_tmp("---\nstatus: open\n---\n# Title\n");
        let item = read_item(&path).unwrap();
        write_item(&path, &item).unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("---\n# Title\n"));
        assert!(!after.contains("---\n\n# Title\n"));
    }

    #[test]
    fn round_trip_preserves_commits_block() {
        let (_tmp, path) = write_tmp(
            "---\nstatus: open\ncommits:\n- hash: abc\n  summary: fix\n- hash: def\n  summary: more\n---\n\n# T\n",
        );
        let item = read_item(&path).unwrap();
        write_item(&path, &item).unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("hash: abc"));
        assert!(after.contains("summary: fix"));
        assert!(after.contains("hash: def"));
        assert!(after.contains("summary: more"));
    }

    #[test]
    fn read_item_handles_no_frontmatter() {
        let (_tmp, path) = write_tmp("# Just a body\nMore content\n");
        let item = read_item(&path).unwrap();
        assert!(item.frontmatter.is_empty());
        assert!(item.body.contains("# Just a body"));
    }

    #[test]
    fn read_item_handles_empty_body() {
        let (_tmp, path) = write_tmp("---\nstatus: open\n---\n");
        let item = read_item(&path).unwrap();
        assert_eq!(item.body, "");
    }

    #[test]
    fn read_item_errors_on_malformed_yaml() {
        let (_tmp, path) = write_tmp("---\nstatus: : :\n---\n");
        let result = read_item(&path);
        assert!(result.is_err());
    }

    #[test]
    fn write_item_appends_trailing_newline() {
        let (_tmp, path) = write_tmp("---\nstatus: open\n---\n# T");
        let item = read_item(&path).unwrap();
        write_item(&path, &item).unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.ends_with('\n'));
    }

    // ── mutation helpers ───────────────────────────────────────────────────

    fn empty_map() -> Mapping {
        Mapping::new()
    }

    #[test]
    fn set_string_inserts_or_overwrites() {
        let mut m = empty_map();
        set_string(&mut m, "status", "open");
        assert_eq!(
            m.get(Value::String("status".into())).unwrap().as_str(),
            Some("open")
        );
        set_string(&mut m, "status", "in-progress");
        assert_eq!(
            m.get(Value::String("status".into())).unwrap().as_str(),
            Some("in-progress")
        );
    }

    #[test]
    fn remove_key_deletes() {
        let mut m = empty_map();
        set_string(&mut m, "epic", "x");
        remove_key(&mut m, "epic");
        assert!(m.get(Value::String("epic".into())).is_none());
    }

    #[test]
    fn add_to_string_list_creates_and_dedupes() {
        let mut m = empty_map();
        add_to_string_list(&mut m, "labels", "infra").unwrap();
        add_to_string_list(&mut m, "labels", "infra").unwrap();
        add_to_string_list(&mut m, "labels", "backend").unwrap();
        let seq = m
            .get(Value::String("labels".into()))
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(seq.len(), 2);
        assert_eq!(seq[0].as_str(), Some("infra"));
        assert_eq!(seq[1].as_str(), Some("backend"));
    }

    #[test]
    fn add_to_string_list_errors_on_non_list() {
        let mut m = empty_map();
        m.insert(Value::String("labels".into()), Value::String("oops".into()));
        assert!(add_to_string_list(&mut m, "labels", "infra").is_err());
    }

    #[test]
    fn remove_from_string_list_removes_only_matching() {
        let mut m = empty_map();
        add_to_string_list(&mut m, "labels", "a").unwrap();
        add_to_string_list(&mut m, "labels", "b").unwrap();
        remove_from_string_list(&mut m, "labels", "a").unwrap();
        let seq = m
            .get(Value::String("labels".into()))
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(seq.len(), 1);
        assert_eq!(seq[0].as_str(), Some("b"));
    }

    #[test]
    fn remove_from_string_list_drops_field_when_empty() {
        let mut m = empty_map();
        add_to_string_list(&mut m, "labels", "only").unwrap();
        remove_from_string_list(&mut m, "labels", "only").unwrap();
        assert!(m.get(Value::String("labels".into())).is_none());
    }

    #[test]
    fn remove_from_string_list_noop_when_missing_key() {
        let mut m = empty_map();
        // Should not error
        remove_from_string_list(&mut m, "labels", "nope").unwrap();
        assert!(m.get(Value::String("labels".into())).is_none());
    }

    #[test]
    fn add_commit_appends_mapping_entries() {
        let mut m = empty_map();
        add_commit(&mut m, "abc123", "fix login").unwrap();
        add_commit(&mut m, "def456", "follow-up").unwrap();
        let seq = m
            .get(Value::String("commits".into()))
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(seq.len(), 2);
        let first = seq[0].as_mapping().unwrap();
        assert_eq!(
            first.get(Value::String("hash".into())).unwrap().as_str(),
            Some("abc123")
        );
        assert_eq!(
            first.get(Value::String("summary".into())).unwrap().as_str(),
            Some("fix login")
        );
    }

    // ── render_new_item ────────────────────────────────────────────────────

    fn issue_args(t: &str, title: &str) -> NewIssueArgs<'static> {
        // SAFETY: we only use this in tests and never store args.
        // The static refs come from string literals.
        NewIssueArgs {
            title: Box::leak(title.to_string().into_boxed_str()),
            issue_type: Box::leak(t.to_string().into_boxed_str()),
            priority: "normal",
            reporter: None,
            assignee: None,
            owner: None,
            epic: None,
            labels: &[],
            related: &[],
            source: None,
            description: None,
            custom_fields: &[],
        }
    }

    #[test]
    fn render_new_item_for_bug_includes_reporter_and_assignee() {
        let mut a = issue_args("bug", "Login bug");
        a.reporter = Some("alice");
        a.assignee = Some("bob");
        let out = render_new_item(&a);
        assert!(out.contains("type: bug"));
        assert!(out.contains("reporter: alice"));
        assert!(out.contains("assignee: bob"));
        assert!(!out.contains("owner:"));
        assert!(out.contains("# Login bug"));
        assert!(out.contains("## Description"));
    }

    #[test]
    fn render_new_item_for_epic_uses_owner() {
        let mut a = issue_args("epic", "API v2");
        a.owner = Some("cara");
        let out = render_new_item(&a);
        assert!(out.contains("type: epic"));
        assert!(out.contains("owner: cara"));
        assert!(!out.contains("reporter:"));
        assert!(!out.contains("assignee:"));
    }

    #[test]
    fn render_new_item_includes_optional_fields() {
        let labels = vec!["frontend".to_string(), "auth".to_string()];
        let related = vec!["@extremely-quiet-otter".to_string()];
        let a = NewIssueArgs {
            title: "X",
            issue_type: "task",
            priority: "high",
            reporter: Some("alice"),
            assignee: None,
            owner: None,
            epic: Some("api-v2-epic"),
            labels: &labels,
            related: &related,
            source: Some("frontend/login"),
            description: Some("Stuck in loop."),
            custom_fields: &[],
        };
        let out = render_new_item(&a);
        assert!(out.contains("priority: high"));
        assert!(out.contains("epic: api-v2-epic"));
        assert!(out.contains("labels: [frontend, auth]"));
        assert!(out.contains("'@extremely-quiet-otter'") || out.contains("@extremely-quiet-otter"));
        assert!(out.contains("_Source: frontend/login_"));
        assert!(out.contains("Stuck in loop."));
    }

    #[test]
    fn render_new_item_omits_empty_optional_fields() {
        let a = issue_args("task", "X");
        let out = render_new_item(&a);
        assert!(!out.contains("epic:"));
        assert!(!out.contains("labels:"));
        assert!(!out.contains("related:"));
        assert!(!out.contains("_Source:"));
    }

    // ── issue_dir ──────────────────────────────────────────────────────────

    #[test]
    fn issue_dir_constructs_flat_path() {
        let p = issue_dir(Path::new("/tmp/repo"), "open", "extremely-quiet-otter");
        assert_eq!(p, Path::new("/tmp/repo/issues/extremely-quiet-otter"));
    }
}
