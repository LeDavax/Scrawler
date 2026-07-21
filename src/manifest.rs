//! Types and parsing logic for Scrawler application manifests.
//!
//! An application owns its manifest. Scrawler reads it, validates its
//! structure, and exposes the declared capabilities to an agent host. The
//! agent therefore stays away from implementation details like DOM selectors.

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use serde::Serialize;
use std::fmt;

/// Full semantic description of an application after parsing.
///
/// `#[derive(...)]` asks Rust to auto-generate useful code:
/// - `Debug` allows displaying this structure during debugging;
/// - `Clone` allows creating a copy;
/// - `PartialEq` allows comparing two manifests in tests;
/// - `Serialize` allows converting the structure to JSON.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppManifest {
    /// Stable technical identifier. Example: `scrawler.mail`.
    pub id: String,

    /// Human-readable name. Example: `Scrawler Mail`.
    pub name: String,

    /// Manifest format version, not necessarily the application version.
    pub version: String,

    /// Path, relative to the XML manifest, of the Lua file that implements the
    /// handlers declared in actions. The manifest keeps this declarative
    /// reference; the real path is resolved by the `serve` command.
    pub actions: Option<String>,

    /// State values declared via `<state><value id="..." .../></state>`.
    /// The renderer uses them to initialise its internal dictionary.
    pub state: Vec<StateValue>,

    /// Top-level tree nodes: usually screens.
    pub nodes: Vec<SemanticNode>,
}

/// State value declaration inside `<state>`.
///
/// XML example: `<value id="draft.recipient" type="string" default="" />`
/// The renderer initialises `state["draft.recipient"] = ""` at startup,
/// without ever needing to know what that field means for Mail.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StateValue {
    /// Stable identifier key, e.g. `draft.recipient`.
    pub id: String,

    /// Declared type: `string`, `number`, `boolean`. Used for future
    /// validation; for now the renderer stores everything as a String.
    pub kind: String,

    /// Initial value injected into state at application startup.
    pub default: String,
}

/// UI or logic component that the agent can discover.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticNode {
    /// Unique identifier within the application.
    pub id: String,

    /// Component kind: `screen`, `button`, `text-input`, `text-area`,
    /// `dialog`, etc.
    pub role: String,

    /// Human- and agent-readable label.
    pub label: String,

    /// Binding to a state key, e.g. `draft.recipient`.
    /// When `bind` is present the renderer reads/writes `state[bind]`
    /// automatically, without knowing the key's semantics.
    pub bind: Option<String>,

    /// Lucide icon name, e.g. `pencil`, `send`, `mail`, `inbox`.
    /// The renderer maps it to the corresponding Unicode character.
    pub icon: Option<String>,

    /// Placeholder text shown in empty fields (`text-input`, `text-area`).
    pub placeholder: Option<String>,

    /// When `true`, the component is displayed but not interactive.
    pub disabled: bool,

    /// When `true`, the field is visible but not editable (read-only).
    pub readonly: bool,

    /// Visual variant: `primary` (default), `secondary`, `destructive`.
    /// Used by `button` to choose the background colour.
    pub variant: Option<String>,

    /// Semantic label exposed to the MCP agent instead of `label`.
    /// Allows a short UI label ("Send") with a richer description for the agent.
    pub aria_label: Option<String>,

    /// Actions available on this node.
    pub actions: Vec<SemanticAction>,

    /// Child components. This field makes the structure recursive: a node may
    /// contain other nodes, as in XML.
    pub children: Vec<SemanticNode>,
}

/// Action declared by a component.
///
/// `handler` is intentionally an implementation reference, not a function
/// called directly by the agent. A future runtime will resolve it to a
/// sandboxed Lua function after validating this manifest.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticAction {
    /// Action identifier exposed to the agent, e.g. `compose_message`.
    pub id: String,

    /// Text explaining to the agent and developer what the action does.
    pub description: String,

    /// Name of the associated Lua function. No code is executed here.
    pub handler: String,

    /// Keyboard shortcut, e.g. `cmd+return`, `ctrl+s`, `escape`.
    /// The renderer triggers the action automatically when the combo is pressed.
    pub shortcut: Option<String>,

    /// Parameters declared in the XML, e.g. `recipient`.
    pub parameters: Vec<ActionParameter>,
}

/// Typed argument accepted by an action.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionParameter {
    /// Name the agent will use in the arguments object.
    pub name: String,

    /// Simple type, currently described by a string: `string`, `number`,
    /// `boolean`, etc. The runtime will validate this more strictly later.
    pub kind: String,

    /// If `true`, the runtime will reject a call that omits this parameter.
    pub required: bool,

    /// Agent-readable explanation to avoid ambiguous arguments.
    pub description: String,
}

/// Errors are explicit: a malformed manifest must fail before an agent
/// receives a misleading or incomplete capability tree.
#[derive(Debug, Clone, PartialEq)]
pub enum ManifestError {
    /// Une erreur remontée par la bibliothèque XML.
    Xml(String),

    /// Une balise obligatoire existe, mais une information importante manque.
    MissingAttribute { element: String, attribute: String },

    /// Le XML utilise une balise qui ne fait pas encore partie du langage.
    UnexpectedElement(String),

    /// Les balises sont connues, mais leur imbrication est incohérente.
    InvalidStructure(String),
}

// `Display` définit la manière dont une erreur est montrée à l'utilisateur,
// par exemple avec `eprintln!("{error}")` dans le programme en ligne de commande.
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

// Cette ligne indique à Rust que ManifestError peut se comporter comme une
// erreur standard. Elle facilite l'utilisation future avec l'opérateur `?`.
impl std::error::Error for ManifestError {}

/// Parse le petit dialecte XML volontairement restreint de Scrawler.
///
/// Les balises structurelles supportées sont `screen`, `group`, `view`,
/// `component` et `dialog`. Elles produisent toutes des nœuds sémantiques.
/// `<state>` et `<value>` déclarent l'état initial de l'application.
pub fn parse_manifest(xml: &str) -> Result<AppManifest, ManifestError> {
    // `&str` est une référence vers du texte : la fonction lit le XML sans en
    // prendre possession. `Result<T, E>` signifie : soit une valeur `T`, soit
    // une erreur `E`. Ici : un AppManifest ou une ManifestError.

    // Reader lit le XML événement par événement. Cela évite de devoir charger
    // tout le document dans une structure XML intermédiaire complexe.
    let mut reader = Reader::from_str(xml);

    // Les retours à la ligne et espaces entre les balises ne sont pas utiles
    // dans notre dialecte ; on les ignore pour simplifier le parsing.
    reader.config_mut().trim_text(true);

    // `Option<T>` représente une valeur qui peut être absente. Avant d'avoir
    // rencontré <app>, il n'existe pas encore de manifeste à compléter.
    let mut manifest: Option<AppManifest> = None;

    // La pile sert à reconstruire la hiérarchie XML. Lorsqu'un nœud s'ouvre,
    // on le pousse dans ce Vec. Quand il se ferme, on le retire et l'ajoute à
    // son parent. `Vec` est utilisé ici comme une pile avec push/pop.
    let mut node_stack: Vec<SemanticNode> = Vec::new();

    // Une action est temporairement gardée séparément tant que le parser lit
    // ses éventuels <param>. À la fermeture de </action>, elle est ajoutée au
    // nœud actuellement ouvert.
    let mut current_action: Option<SemanticAction> = None;

    // Indique si le parser est actuellement à l'intérieur d'un bloc <state>.
    // Ce drapeau évite d'accepter des <value> hors de leur contexte.
    let mut inside_state = false;

    // `loop` crée une boucle sans fin que l'on quittera explicitement quand le
    // Reader signalera la fin du fichier avec Event::Eof.
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
                            // Le `?` retourne immédiatement l'erreur si un
                            // attribut obligatoire est manquant.
                            id: required_attribute(start, "app", "id")?,
                            name: required_attribute(start, "app", "name")?,
                            version: attribute_or(start, "version", "0.1"),
                            actions: attribute(start, "actions")?,
                            state: Vec::new(),
                            nodes: Vec::new(),
                        });
                    }
                    "state" => {
                        // <state> est un conteneur déclaratif pour les valeurs
                        // initiales. Il n'est pas un nœud sémantique.
                        ensure_app_started(&manifest)?;
                        if inside_state {
                            return Err(ManifestError::InvalidStructure(
                                "<state> cannot be nested".into(),
                            ));
                        }
                        inside_state = true;
                    }
                    "screen" | "group" | "view" | "component" | "dialog" => {
                        // Cette syntaxe avec `|` permet de partager le même
                        // traitement entre plusieurs noms de balises.
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
                            actions: Vec::new(),
                            children: Vec::new(),
                        });
                    }
                    "action" => {
                        // last_mut() renvoie une référence modifiable vers le
                        // dernier élément de la pile, ou None si elle est vide.
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

                        // On termine volontairement cet emprunt mutable ici.
                        // En Rust, on ne peut pas garder plusieurs accès
                        // mutables incompatibles aux mêmes données.
                        let _ = node;
                    }
                    "param" => {
                        // `as_mut()` donne une référence modifiable au contenu
                        // d'une Option seulement s'il est présent.
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
                // Une balise XML vide agit comme une balise ouvrante suivie
                // immédiatement d'une balise fermante, courant pour <param />
                // et <value />.
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
                        // <value /> déclare une entrée d'état initiale.
                        // Elle doit impérativement être à l'intérieur de <state>.
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
                    // Une action sans paramètres peut être écrite en forme
                    // auto-fermante : `<action id="send" handler="send" />`.
                    // On l'ajoute directement au nœud courant sans passer par
                    // la pile temporaire `current_action`.
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
                    // Un nœud auto-fermant comme `<component ... />` ou
                    // `<screen ... />` est un nœud feuille sans enfants.
                    // On le traite comme l'ouverture + fermeture immédiate.
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
                // Une balise fermante arrive après que ses enfants ont déjà
                // été lus. C'est donc le bon moment pour construire l'arbre.
                let tag = String::from_utf8_lossy(end.name().as_ref()).into_owned();
                match tag.as_str() {
                    "state" => {
                        inside_state = false;
                    }
                    "action" => {
                        // take() retire l'action de l'Option et la laisse à
                        // None. Cela évite de la copier inutilement.
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
                        // pop() récupère le dernier nœud ouvert : c'est celui
                        // que la balise fermante vient de terminer.
                        let completed_node = node_stack.pop().ok_or_else(|| {
                            ManifestError::InvalidStructure(
                                "closing a node that was not open".into(),
                            )
                        })?;

                        if let Some(parent) = node_stack.last_mut() {
                            // S'il reste un nœud sur la pile, le nœud terminé
                            // est son enfant direct.
                            parent.children.push(completed_node);
                        } else {
                            // Sinon, il s'agit d'un nœud de premier niveau de
                            // l'application, par exemple un écran.
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
            // Notre langage ne donne pas de sens au texte libre XML. On ignore
            // donc les espaces, commentaires et déclaration `<?xml ...?>`.
            Ok(Event::Text(_)) | Ok(Event::Comment(_)) | Ok(Event::Decl(_)) => {}
            Ok(_) => {}
            Err(error) => return Err(ManifestError::Xml(error.to_string())),
        }
    }

    // À la fin, on vérifie que le document possède bien une app et que toutes
    // les balises que nous avons ouvertes ont été correctement refermées.
    let manifest =
        manifest.ok_or_else(|| ManifestError::InvalidStructure("missing <app> root".into()))?;
    if !node_stack.is_empty() || current_action.is_some() {
        return Err(ManifestError::InvalidStructure(
            "unclosed XML elements".into(),
        ));
    }
    Ok(manifest)
}

/// Trouve un nœud par son `id` dans l'arbre et retourne une référence mutable.
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

// Petite fonction de validation réutilisée dès qu'une balise a besoin d'être
// placée à l'intérieur d'un <app> déjà rencontré.
fn ensure_app_started(manifest: &Option<AppManifest>) -> Result<(), ManifestError> {
    if manifest.is_none() {
        Err(ManifestError::InvalidStructure(
            "the document must start with <app>".into(),
        ))
    } else {
        Ok(())
    }
}

// BytesStart est le type de quick-xml qui représente une balise ouvrante. On
// convertit son nom d'octets en String Rust pour pouvoir le comparer facilement.
fn element_name(element: &BytesStart<'_>) -> Result<String, ManifestError> {
    String::from_utf8(element.name().as_ref().to_vec())
        .map_err(|_| ManifestError::Xml("element name is not valid UTF-8".into()))
}

// Lit un attribut obligatoire. La différence entre cette fonction et
// attribute_or est que celle-ci produit une erreur claire si la valeur manque.
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

// Lit un attribut facultatif en utilisant une valeur par défaut quand il n'est
// pas présent. C'est le cas de `version` ou de `description`.
fn attribute_or(element: &BytesStart<'_>, attribute_name: &str, default: &str) -> String {
    attribute(element, attribute_name)
        .ok()
        .flatten()
        .unwrap_or_else(|| default.into())
}

// Parcourt tous les attributs d'une balise et retourne `Some(valeur)` lorsque
// le nom recherché est trouvé, sinon `None`.
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

#[cfg(test)]
mod tests {
    use super::*;

    // Un test est une fonction normale marquée avec #[test]. `cargo test`
    // l'exécute automatiquement et signale si une assertion échoue.
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
}
