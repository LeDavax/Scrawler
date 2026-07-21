//! Types and parsing logic for Scrawler application manifests.
//!
//! An application owns its manifest. Scrawler reads it, validates its
//! structure, and exposes the declared capabilities to an agent host. The
//! agent therefore stays away from implementation details like DOM selectors.

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppManifest {
    /// Stable technical identifier. Example: `scrawler.mail`.
    pub id: String,

    /// Human-readable name. Example: `Scrawler Mail`.
    pub name: String,

    /// Manifest format version, not necessarily the application version.
    pub version: String,

    /// Path, relative to the XML manifest, of the Lua file that implements the
    /// handlers declared in actions.
    pub actions: Option<String>,

    /// State values declared via `<state><value id="..." .../></state>`.
    pub state: Vec<StateValue>,

    /// Top-level tree nodes: usually screens.
    pub nodes: Vec<SemanticNode>,
}

/// State value declaration inside `<state>`.
///
/// XML example: `<value id="draft.recipient" type="string" default="" />`
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StateValue {
    pub id: String,
    pub kind: String,
    pub default: String,
}

/// UI or logic component that the agent can discover.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticNode {
    pub id: String,
    pub role: String,
    pub label: String,
    /// Binding to a state key; the renderer reads/writes `state[bind]` automatically.
    pub bind: Option<String>,
    /// Lucide icon name, e.g. `pencil`, `send`, `mail`.
    pub icon: Option<String>,
    pub placeholder: Option<String>,
    pub disabled: bool,
    pub readonly: bool,
    /// Visual variant: `primary` (default), `secondary`, `destructive`.
    pub variant: Option<String>,
    /// Semantic label for the MCP agent (overrides `label` when present).
    pub aria_label: Option<String>,
    /// Optional layout hint for container-like nodes: `column`, `row`, `wrap`, `grid`.
    pub layout: Option<String>,
    /// Spacing between children, in logical pixels.
    pub gap: Option<f32>,
    /// Inner padding for container-like nodes, in logical pixels.
    pub padding: Option<f32>,
    /// Explicit width for the node when rendered by the UI.
    pub width: Option<f32>,
    /// Minimum width for the node.
    pub min_width: Option<f32>,
    /// Minimum height for the node.
    pub min_height: Option<f32>,
    /// Maximum height for the node.
    pub max_height: Option<f32>,
    /// Number of columns used by grid layouts.
    pub columns: Option<usize>,
    /// Whether row layouts may wrap onto multiple lines.
    pub wrap: bool,
    /// Whether the container should be rendered inside a scroll area.
    pub scroll: bool,
    /// Hint that the node should expand to available space.
    pub grow: bool,
    pub actions: Vec<SemanticAction>,
    pub children: Vec<SemanticNode>,
}

/// Action declared by a component.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticAction {
    pub id: String,
    pub description: String,
    /// Name of the Lua handler function; resolved at runtime.
    pub handler: String,
    pub shortcut: Option<String>,
    pub parameters: Vec<ActionParameter>,
}

/// Typed argument accepted by an action.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionParameter {
    pub name: String,
    pub kind: String,
    pub required: bool,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ManifestError {
    Xml(String),
    MissingAttribute { element: String, attribute: String },
    UnexpectedElement(String),
    InvalidStructure(String),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Xml(message) => write!(formatter, "XML error: {message}"),
            Self::MissingAttribute { element, attribute } => {
                write!(
                    formatter,
                    "<{element}> is missing required attribute `{attribute}`"
                )
            }
            Self::UnexpectedElement(element) => write!(formatter, "unexpected <{element}> element"),
            Self::InvalidStructure(message) => {
                write!(formatter, "invalid manifest structure: {message}")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

pub fn parse_manifest(xml: &str) -> Result<AppManifest, ManifestError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut manifest: Option<AppManifest> = None;
    let mut node_stack: Vec<SemanticNode> = Vec::new();
    let mut current_action: Option<SemanticAction> = None;
    let mut inside_state = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref start)) => {
                let tag = element_name(start)?;
                match tag.as_str() {
                    "app" => {
                        if manifest.is_some() {
                            return Err(ManifestError::InvalidStructure(
                                "only one <app> root is allowed".into(),
                            ));
                        }

                        manifest = Some(AppManifest {
                            id: required_attribute(start, "app", "id")?,
                            name: required_attribute(start, "app", "name")?,
                            version: attribute_or(start, "version", "0.1"),
                            actions: attribute(start, "actions")?,
                            state: Vec::new(),
                            nodes: Vec::new(),
                        });
                    }
                    "state" => {
                        ensure_app_started(&manifest)?;
                        if inside_state {
                            return Err(ManifestError::InvalidStructure(
                                "<state> cannot be nested".into(),
                            ));
                        }
                        inside_state = true;
                    }
                    "screen" | "group" | "view" | "component" | "dialog" => {
                        ensure_app_started(&manifest)?;
                        node_stack.push(SemanticNode {
                            id: required_attribute(start, &tag, "id")?,
                            role: required_attribute(start, &tag, "role")?,
                            label: required_attribute(start, &tag, "label")?,
                            bind: attribute(start, "bind")?,
                            icon: attribute(start, "icon")?,
                            placeholder: attribute(start, "placeholder")?,
                            disabled: attribute_or(start, "disabled", "false") == "true",
                            readonly: attribute_or(start, "readonly", "false") == "true",
                            variant: attribute(start, "variant")?,
                            aria_label: attribute(start, "aria-label")?,
                            layout: attribute(start, "layout")?,
                            gap: f32_attribute(start, "gap")?,
                            padding: f32_attribute(start, "padding")?,
                            width: f32_attribute(start, "width")?,
                            min_width: f32_attribute(start, "min-width")?,
                            min_height: f32_attribute(start, "min-height")?,
                            max_height: f32_attribute(start, "max-height")?,
                            columns: usize_attribute(start, "columns")?,
                            wrap: bool_attribute(start, "wrap")?.unwrap_or(false),
                            scroll: bool_attribute(start, "scroll")?.unwrap_or(false),
                            grow: bool_attribute(start, "grow")?.unwrap_or(false),
                            actions: Vec::new(),
                            children: Vec::new(),
                        });
                    }
                    "action" => {
                        let node = node_stack.last_mut().ok_or_else(|| {
                            ManifestError::InvalidStructure(
                                "<action> must be inside a semantic node".into(),
                            )
                        })?;

                        if current_action.is_some() {
                            return Err(ManifestError::InvalidStructure(
                                "<action> elements cannot be nested".into(),
                            ));
                        }

                        current_action = Some(SemanticAction {
                            id: required_attribute(start, "action", "id")?,
                            description: attribute_or(start, "description", ""),
                            handler: required_attribute(start, "action", "handler")?,
                            shortcut: attribute(start, "shortcut")?,
                            parameters: Vec::new(),
                        });

                        let _ = node;
                    }
                    "param" => {
                        let action = current_action.as_mut().ok_or_else(|| {
                            ManifestError::InvalidStructure(
                                "<param> must be inside an <action>".into(),
                            )
                        })?;

                        action.parameters.push(ActionParameter {
                            name: required_attribute(start, "param", "name")?,
                            kind: attribute_or(start, "type", "string"),
                            required: attribute_or(start, "required", "false") == "true",
                            description: attribute_or(start, "description", ""),
                        });
                    }
                    other => return Err(ManifestError::UnexpectedElement(other.into())),
                }
            }
            Ok(Event::Empty(ref empty)) => {
                let tag = element_name(empty)?;
                match tag.as_str() {
                    "param" => {
                        let action = current_action.as_mut().ok_or_else(|| {
                            ManifestError::InvalidStructure(
                                "<param> must be inside an <action>".into(),
                            )
                        })?;
                        action.parameters.push(ActionParameter {
                            name: required_attribute(empty, "param", "name")?,
                            kind: attribute_or(empty, "type", "string"),
                            required: attribute_or(empty, "required", "false") == "true",
                            description: attribute_or(empty, "description", ""),
                        });
                    }
                    "value" => {
                        if !inside_state {
                            return Err(ManifestError::InvalidStructure(
                                "<value> must be inside <state>".into(),
                            ));
                        }
                        let m = manifest.as_mut().expect("checked by inside_state");
                        m.state.push(StateValue {
                            id: required_attribute(empty, "value", "id")?,
                            kind: attribute_or(empty, "type", "string"),
                            default: attribute_or(empty, "default", ""),
                        });
                    }
                    "action" => {
                        let node = node_stack.last_mut().ok_or_else(|| {
                            ManifestError::InvalidStructure(
                                "<action> must be inside a semantic node".into(),
                            )
                        })?;
                        node.actions.push(SemanticAction {
                            id: required_attribute(empty, "action", "id")?,
                            description: attribute_or(empty, "description", ""),
                            handler: required_attribute(empty, "action", "handler")?,
                            shortcut: attribute(empty, "shortcut")?,
                            parameters: Vec::new(),
                        });
                    }
                    "screen" | "group" | "view" | "component" | "dialog" => {
                        ensure_app_started(&manifest)?;
                        let node = SemanticNode {
                            id: required_attribute(empty, &tag, "id")?,
                            role: required_attribute(empty, &tag, "role")?,
                            label: required_attribute(empty, &tag, "label")?,
                            bind: attribute(empty, "bind")?,
                            icon: attribute(empty, "icon")?,
                            placeholder: attribute(empty, "placeholder")?,
                            disabled: attribute_or(empty, "disabled", "false") == "true",
                            readonly: attribute_or(empty, "readonly", "false") == "true",
                            variant: attribute(empty, "variant")?,
                            aria_label: attribute(empty, "aria-label")?,
                            layout: attribute(empty, "layout")?,
                            gap: f32_attribute(empty, "gap")?,
                            padding: f32_attribute(empty, "padding")?,
                            width: f32_attribute(empty, "width")?,
                            min_width: f32_attribute(empty, "min-width")?,
                            min_height: f32_attribute(empty, "min-height")?,
                            max_height: f32_attribute(empty, "max-height")?,
                            columns: usize_attribute(empty, "columns")?,
                            wrap: bool_attribute(empty, "wrap")?.unwrap_or(false),
                            scroll: bool_attribute(empty, "scroll")?.unwrap_or(false),
                            grow: bool_attribute(empty, "grow")?.unwrap_or(false),
                            actions: Vec::new(),
                            children: Vec::new(),
                        };
                        if let Some(parent) = node_stack.last_mut() {
                            parent.children.push(node);
                        } else {
                            manifest.as_mut().expect("checked above").nodes.push(node);
                        }
                    }
                    other => return Err(ManifestError::UnexpectedElement(other.into())),
                }
            }
            Ok(Event::End(ref end)) => {
                let tag = String::from_utf8_lossy(end.name().as_ref()).into_owned();
                match tag.as_str() {
                    "state" => {
                        inside_state = false;
                    }
                    "action" => {
                        let action = current_action.take().ok_or_else(|| {
                            ManifestError::InvalidStructure(
                                "closing an action that was not open".into(),
                            )
                        })?;
                        let node = node_stack.last_mut().ok_or_else(|| {
                            ManifestError::InvalidStructure(
                                "<action> closed without a parent node".into(),
                            )
                        })?;
                        node.actions.push(action);
                    }
                    "screen" | "group" | "view" | "component" | "dialog" => {
                        let completed_node = node_stack.pop().ok_or_else(|| {
                            ManifestError::InvalidStructure(
                                "closing a node that was not open".into(),
                            )
                        })?;

                        if let Some(parent) = node_stack.last_mut() {
                            parent.children.push(completed_node);
                        } else {
                            ensure_app_started(&manifest)?;
                            manifest
                                .as_mut()
                                .expect("checked above")
                                .nodes
                                .push(completed_node);
                        }
                    }
                    "app" => {
                        if !node_stack.is_empty() || current_action.is_some() {
                            return Err(ManifestError::InvalidStructure(
                                "<app> closed before all children were closed".into(),
                            ));
                        }
                    }
                    other => return Err(ManifestError::UnexpectedElement(other.into())),
                }
            }
            Ok(Event::Eof) => break,
            Ok(Event::Text(_)) | Ok(Event::Comment(_)) | Ok(Event::Decl(_)) => {}
            Ok(_) => {}
            Err(error) => return Err(ManifestError::Xml(error.to_string())),
        }
    }

    let manifest =
        manifest.ok_or_else(|| ManifestError::InvalidStructure("missing <app> root".into()))?;
    if !node_stack.is_empty() || current_action.is_some() {
        return Err(ManifestError::InvalidStructure(
            "unclosed XML elements".into(),
        ));
    }
    Ok(manifest)
}

pub fn find_node_mut<'a>(nodes: &'a mut Vec<SemanticNode>, wanted_id: &str) -> Option<&'a mut SemanticNode> {
    for node in nodes {
        if node.id == wanted_id {
            return Some(node);
        }
        if let Some(found) = find_node_mut(&mut node.children, wanted_id) {
            return Some(found);
        }
    }
    None
}

fn ensure_app_started(manifest: &Option<AppManifest>) -> Result<(), ManifestError> {
    if manifest.is_none() {
        Err(ManifestError::InvalidStructure(
            "the document must start with <app>".into(),
        ))
    } else {
        Ok(())
    }
}

fn element_name(element: &BytesStart<'_>) -> Result<String, ManifestError> {
    String::from_utf8(element.name().as_ref().to_vec())
        .map_err(|_| ManifestError::Xml("element name is not valid UTF-8".into()))
}

fn required_attribute(
    element: &BytesStart<'_>,
    element_name: &str,
    attribute_name: &str,
) -> Result<String, ManifestError> {
    attribute(element, attribute_name)?.ok_or_else(|| ManifestError::MissingAttribute {
        element: element_name.into(),
        attribute: attribute_name.into(),
    })
}

fn attribute_or(element: &BytesStart<'_>, attribute_name: &str, default: &str) -> String {
    attribute(element, attribute_name)
        .ok()
        .flatten()
        .unwrap_or_else(|| default.into())
}

fn attribute(element: &BytesStart<'_>, wanted_name: &str) -> Result<Option<String>, ManifestError> {
    for item in element.attributes() {
        let attribute = item.map_err(|error| ManifestError::Xml(error.to_string()))?;
        if attribute.key.as_ref() == wanted_name.as_bytes() {
            let value = attribute
                .unescape_value()
                .map_err(|error| ManifestError::Xml(error.to_string()))?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

fn bool_attribute(element: &BytesStart<'_>, wanted_name: &str) -> Result<Option<bool>, ManifestError> {
    Ok(match attribute(element, wanted_name)? {
        Some(value) => Some(matches!(value.as_str(), "true" | "1" | "yes" | "on")),
        None => None,
    })
}

fn f32_attribute(element: &BytesStart<'_>, wanted_name: &str) -> Result<Option<f32>, ManifestError> {
    match attribute(element, wanted_name)? {
        Some(value) => value
            .parse::<f32>()
            .map(Some)
            .map_err(|error| ManifestError::Xml(format!("invalid `{wanted_name}` value: {error}"))),
        None => Ok(None),
    }
}

fn usize_attribute(element: &BytesStart<'_>, wanted_name: &str) -> Result<Option<usize>, ManifestError> {
    match attribute(element, wanted_name)? {
        Some(value) => value
            .parse::<usize>()
            .map(Some)
            .map_err(|error| ManifestError::Xml(format!("invalid `{wanted_name}` value: {error}"))),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_component_with_a_typed_action() {
        let xml = r#"
            <app id="mail" name="Mail" version="0.1">
              <screen id="inbox" role="screen" label="Inbox">
                <component id="compose" role="button" label="Compose">
                  <action id="compose_message" handler="compose_message" description="Open a new draft">
                    <param name="recipient" type="string" required="false" description="Initial recipient" />
                  </action>
                </component>
              </screen>
            </app>
        "#;

        let manifest = parse_manifest(xml).expect("manifest should parse");
        assert_eq!(manifest.id, "mail");
        assert_eq!(manifest.actions, None);
        assert_eq!(
            manifest.nodes[0].children[0].actions[0].id,
            "compose_message"
        );
        assert!(!manifest.nodes[0].children[0].actions[0].parameters[0].required);
    }

    #[test]
    fn parses_state_declarations() {
        let xml = r#"
            <app id="mail" name="Mail">
              <state>
                <value id="draft.recipient" type="string" default="" />
                <value id="draft.body" type="string" default="Hello" />
              </state>
              <screen id="inbox" role="screen" label="Inbox" />
            </app>
        "#;

        let manifest = parse_manifest(xml).expect("manifest with state should parse");
        assert_eq!(manifest.state.len(), 2);
        assert_eq!(manifest.state[0].id, "draft.recipient");
        assert_eq!(manifest.state[1].default, "Hello");
    }

    #[test]
    fn parses_bind_attribute_on_component() {
        let xml = r#"
            <app id="mail" name="Mail">
              <screen id="inbox" role="screen" label="Inbox">
                <view id="composer" role="dialog" label="New message">
                  <component id="recipient" role="text-input" label="Recipient" bind="draft.recipient" />
                </view>
              </screen>
            </app>
        "#;

        let manifest = parse_manifest(xml).expect("manifest with bind should parse");
        let view = &manifest.nodes[0].children[0];
        assert_eq!(view.role, "dialog");
        let input = &view.children[0];
        assert_eq!(input.bind, Some("draft.recipient".into()));
    }

    #[test]
    fn parses_layout_hints_on_containers() {
        let xml = r#"
            <app id="notes" name="Notes">
              <screen id="workspace" role="screen" label="Workspace" layout="column" gap="18">
                <group id="toolbar" role="group" label="" layout="row" gap="10" padding="16" wrap="true" />
                <component id="note-grid" role="list" label="Notes" layout="grid" columns="3" scroll="true" max-height="320" />
              </screen>
            </app>
        "#;

        let manifest = parse_manifest(xml).expect("manifest with layout hints should parse");
        let screen = &manifest.nodes[0];
        assert_eq!(screen.layout.as_deref(), Some("column"));
        assert_eq!(screen.gap, Some(18.0));
        let toolbar = &screen.children[0];
        assert_eq!(toolbar.layout.as_deref(), Some("row"));
        assert_eq!(toolbar.padding, Some(16.0));
        assert!(toolbar.wrap);
        let grid = &screen.children[1];
        assert_eq!(grid.columns, Some(3));
        assert!(grid.scroll);
        assert_eq!(grid.max_height, Some(320.0));
    }
}
