use super::*;

/// Deserializable create request (used by `import`). Mirrors `cmd_new`'s flag set.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct NewIssueRequest {
    #[serde(rename = "type")]
    pub issue_type: String,
    pub title: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub reporter: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default)]
    pub epic: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub related: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Custom frontmatter fields keyed by field name, mirroring CLI
    /// `--field key=value`. Required for repos whose schema declares
    /// custom required fields — without this, API creation cannot
    /// satisfy the schema and falls into the same bricking failure
    /// mode the CLI `--field` flag was added to fix.
    ///
    /// JSON shape: an object (`{"team": "payments"}`). Duplicate keys
    /// in the deserialized payload are rejected during deserialization so
    /// the JSON create input enforces the same invariant the CLI
    /// `--field foo=a --field foo=b` rejection enforces — calling
    /// agents need a deterministic error rather than silent last-write-
    /// wins behavior.
    #[serde(default, deserialize_with = "deserialize_custom_fields_no_dups")]
    pub custom_fields: Vec<(String, String)>,
}

pub(crate) fn deserialize_custom_fields_no_dups<'de, D>(
    de: D,
) -> Result<Vec<(String, String)>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{MapAccess, Visitor};
    use std::fmt;

    struct CustomFieldsVisitor;
    impl<'de> Visitor<'de> for CustomFieldsVisitor {
        type Value = Vec<(String, String)>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an object of custom field key=value pairs with no duplicate keys")
        }
        fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            // serde_json's MapAccess yields raw JSON object entries in
            // input order — duplicates are NOT pre-deduplicated by the
            // parser, so we see both and can reject. Switching to
            // BTreeMap here would silently keep the last value.
            //
            // Pull `next_key` and `next_value` separately so the
            // duplicate-key check fires BEFORE value deserialization;
            // otherwise a payload like `{"team":"a","team":1}` would
            // surface a type error from the bad second value rather
            // than the duplicate-key diagnostic the test pins.
            let mut out: Vec<(String, String)> = Vec::new();
            let mut seen = std::collections::BTreeSet::new();
            while let Some(k) = map.next_key::<String>()? {
                if !seen.insert(k.clone()) {
                    return Err(serde::de::Error::custom(format!(
                        "custom field {k:?} given more than once"
                    )));
                }
                let v = map.next_value::<String>()?;
                out.push((k, v));
            }
            Ok(out)
        }
    }

    de.deserialize_map(CustomFieldsVisitor)
}

/// Sister of `deserialize_custom_fields_no_dups` for the update path,
/// where the wire shape is `{key: Patch<String>}` instead of
/// `{key: String}`. Same duplicate-key rejection contract — without it
/// a `PATCH {"custom_fields": {"team":"a","team":null}}` would silently
/// keep whichever entry `serde_json` saw last.
pub(crate) fn deserialize_patch_map_no_dups<'de, D>(
    de: D,
) -> Result<std::collections::BTreeMap<String, Patch<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{MapAccess, Visitor};
    use std::fmt;

    struct PatchMapVisitor;
    impl<'de> Visitor<'de> for PatchMapVisitor {
        type Value = std::collections::BTreeMap<String, Patch<String>>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an object of custom field key=value pairs with no duplicate keys")
        }
        fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut out: std::collections::BTreeMap<String, Patch<String>> =
                std::collections::BTreeMap::new();
            while let Some(k) = map.next_key::<String>()? {
                if out.contains_key(&k) {
                    return Err(serde::de::Error::custom(format!(
                        "custom field {k:?} given more than once"
                    )));
                }
                let v = map.next_value::<Patch<String>>()?;
                out.insert(k, v);
            }
            Ok(out)
        }
    }

    de.deserialize_map(PatchMapVisitor)
}

pub(crate) fn default_priority() -> String {
    "normal".to_string()
}

#[derive(Debug)]
pub struct NewOutcome {
    pub issue: Issue,
    pub version: String,
    pub issue_dir: PathBuf,
}

pub fn new_issue(root: &Path, req: NewIssueRequest) -> Result<NewOutcome, MutateError> {
    if req.title.trim().is_empty() {
        return Err(MutateError::Validation("title cannot be empty".into()));
    }
    if !crate::issue_fields::ISSUE_TYPES
        .iter()
        .any(|t| t == &req.issue_type)
    {
        return Err(MutateError::Validation(format!(
            "type {:?} is not one of the known types",
            req.issue_type
        )));
    }
    if !crate::issue_fields::PRIORITIES
        .iter()
        .any(|p| p == &req.priority)
    {
        return Err(MutateError::Validation(format!(
            "priority {:?} is not one of the known priorities",
            req.priority
        )));
    }

    // C3: hold the flock through write + parse + publish so seq order
    // matches disk order. The previous implementation called `do_new`,
    // which acquired/released the lock internally — the synthetic
    // `IssueUpserted` then published OUTSIDE the lock, inverting seq
    // against concurrent writers.
    let lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    let outcome = new_issue::do_new_locked(
        &lock,
        root,
        new_issue::NewArgs {
            issue_type: req.issue_type,
            title: req.title,
            slug: req.slug,
            // Server-side `new` mirrors the CLI: a `None` slug derives from
            // the title (with random fallback). The API has no `--slug-random`
            // knob; a caller wanting a random slug omits `slug` on an
            // unsluggable title, or passes an explicit one.
            slug_random: false,
            reporter: req.reporter,
            assignee: req.assignee,
            owner: req.owner,
            priority: req.priority,
            epic: req.epic,
            labels: req.labels,
            related: req.related,
            source: req.source,
            description: req.description,
            custom_fields: req.custom_fields,
            lane: None,
            lane_seq: None,
            collision: vec![],
            status: None,
            inbox: false,
        },
    )
    .map_err(MutateError::from)?;

    // Re-read for canonical hash + Issue. Still holding the lock.
    let parsed =
        crate::parser::parse_item_md_with_warnings(&outcome.item_path, &outcome.slug, "open");
    let mut issue = parsed.issue;
    let schema =
        crate::schema::load(root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    issue.folder = folder_for_status(&schema, &issue.status).to_string();
    let version = canonical_hash(&issue);

    let result = NewOutcome {
        issue_dir: outcome.item_path.parent().unwrap().to_path_buf(),
        issue,
        version,
    };
    drop(lock);
    Ok(result)
}
