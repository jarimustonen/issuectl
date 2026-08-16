//! Serializable model for machine-readable CLI help.

use serde::Serialize;

/// A structured representation of one CLI command's help.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct HelpDocument {
    pub path: Vec<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub subcommands: Vec<HelpSubcommand>,
    pub flags: Vec<HelpArgument>,
    pub args: Vec<HelpArgument>,
    pub examples: Vec<HelpExample>,
}

impl HelpDocument {
    /// Construct an empty document for one command.
    pub fn new(path: Vec<String>, name: String) -> Self {
        Self {
            path,
            name,
            description: None,
            subcommands: Vec::new(),
            flags: Vec::new(),
            args: Vec::new(),
            examples: Vec::new(),
        }
    }
}

/// A direct child command.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct HelpSubcommand {
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A flag or positional argument.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct HelpArgument {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long: Option<String>,
    pub required: bool,
    pub global: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub value_names: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub default: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub possible_values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
}

/// A copy-pasteable command invocation.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct HelpExample {
    pub description: String,
    pub argv: Vec<String>,
}

/// Render a help document as one pretty-printed JSON value.
pub fn render_json(document: &HelpDocument) -> serde_json::Result<String> {
    serde_json::to_string_pretty(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_the_stable_help_shape() {
        let mut document = HelpDocument::new(
            vec!["issuectl".to_string(), "new".to_string()],
            "new".to_string(),
        );
        document.flags.push(HelpArgument {
            name: "json".to_string(),
            short: None,
            long: Some("--json".to_string()),
            required: false,
            global: true,
            description: Some("Output as JSON".to_string()),
            value_names: Vec::new(),
            default: Vec::new(),
            possible_values: Vec::new(),
            env: None,
        });
        document.examples.push(HelpExample {
            description: "Create a bug".to_string(),
            argv: vec![
                "issuectl".to_string(),
                "new".to_string(),
                "--type".to_string(),
                "bug".to_string(),
                "--title".to_string(),
                "Login loop".to_string(),
            ],
        });

        let value: serde_json::Value =
            serde_json::from_str(&render_json(&document).unwrap()).unwrap();
        assert_eq!(value["path"], serde_json::json!(["issuectl", "new"]));
        assert_eq!(value["flags"][0]["global"], true);
        assert_eq!(value["examples"][0]["argv"][0], "issuectl");
    }
}
