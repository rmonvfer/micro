//! What the model is told about `codemode`, and how the other tools are presented while it is
//! declared.

use crate::declarations::mcp_structured_content_schema;
use crate::declarations::render_output_type;
use crate::declarations::render_sample;
use crate::declarations::Declaration;
use crate::declarations::MCP_TYPESCRIPT_PREAMBLE;
use crate::identifier::identifier;
use crate::CODEMODE_TOOL_NAME;
use micro_tools::CallableTool;
use micro_tools::Loadout;
use micro_tools::LoadoutChanges;
use micro_types::ToolExposure;
use micro_types::ToolNamespace;
use serde_json::json;
use serde_json::Value;

/// The token budget for tool declarations in the description when the settings do not say.
pub const DEFAULT_INLINE_BUDGET: usize = 3000;
/// Characters per token when estimating what a tool section costs.
const CHARS_PER_TOKEN: usize = 4;

/// How `codemode` presents the other tools while it is declared.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    /// Declared tools stay declared, and their descriptions say how scripts call them.
    #[default]
    On,
    /// Declared tools are hidden from the model and listed in the `codemode` description instead,
    /// so the model calls them through scripts.
    Only,
}

impl Mode {
    pub fn parse(value: &str) -> Option<Mode> {
        match value {
            "on" => Some(Mode::On),
            "only" => Some(Mode::Only),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::On => "on",
            Mode::Only => "only",
        }
    }
}

const INTRO: &str = "Run JavaScript that calls other tools. The input is raw JavaScript (not \
JSON, no code fence), run as an async function body in a QuickJS sandbox: top-level `await` and \
`return` work. No Node, file system, network, or timers.
- `await tools.<name>({ ...args })` resolves to a string, or an object if the tool's declaration \
says so, and rejects with an Error on failure. Calls still running when the script ends are \
cancelled.
- Optional first line: `// @options: {\"max_output_tokens\": 10000, \"timeout_ms\": 60000}`";

const GLOBALS: &str = "Globals:
- `text(value)`, `image(dataUrlOrImageBlock)`, `console.log(...)`, and top-level `return` add \
output; `exit()` ends the script.
- `store(key, value)` and `load(key)` keep JSON values across codemode calls.
- `ALL_TOOLS`, `searchTools(query, { limit?, namespace? })`, `describeTool(name)`, \
`describeNamespace(name)`: find unlisted tools, such as MCP tools.";

/// What a script sees of a tool. Tools without an output schema resolve to their text.
pub fn declaration_of(tool: &CallableTool) -> Declaration {
    Declaration {
        name: tool.definition.name.clone(),
        description: tool.definition.description.clone(),
        input_schema: Some(tool.definition.parameters.clone()),
        output_schema: Some(
            tool.output_schema
                .clone()
                .unwrap_or_else(|| json!({ "type": "string" })),
        ),
    }
}

/// `### \`id\` (\`raw name\`)` followed by the tool's description and declaration.
fn section(declaration: &Declaration) -> String {
    let id = identifier(&declaration.name);
    let heading = match id == declaration.name {
        true => format!("### `{id}`"),
        false => format!("### `{id}` (`{}`)", declaration.name),
    };
    format!("{heading}\n{}", render_sample(declaration).trim())
}

struct Entry {
    name: String,
    section: String,
    cost: usize,
}

struct Group {
    namespace: Option<ToolNamespace>,
    entries: Vec<Entry>,
}

/// The tool sections that fit the budget: in each round every group (tools without a namespace
/// first, then namespaces by name) places its cheapest remaining tool, and a group whose next tool
/// does not fit drops out while the others continue, so every namespace is represented before any
/// is complete.
fn select(groups: &[Group], budget: Option<usize>) -> Vec<String> {
    let Some(mut remaining) = budget else {
        return groups
            .iter()
            .flat_map(|group| group.entries.iter().map(|entry| entry.name.clone()))
            .collect();
    };
    let mut queues: Vec<Vec<&Entry>> = groups
        .iter()
        .map(|group| {
            let mut entries: Vec<&Entry> = group.entries.iter().collect();
            entries.sort_by_key(|entry| entry.cost);
            entries.reverse();
            entries
        })
        .filter(|queue| !queue.is_empty())
        .collect();
    let mut shown = Vec::new();
    while !queues.is_empty() {
        queues.retain_mut(|queue| {
            let Some(next) = queue.last() else {
                return false;
            };
            if next.cost > remaining {
                return false;
            }
            remaining -= next.cost;
            shown.push(next.name.clone());
            queue.pop();
            !queue.is_empty()
        });
    }
    shown
}

/// The model-facing description: the helpers, how to find unlisted tools, the shared MCP types
/// when listed tools need them, and one section per listed tool, grouped by namespace. `Deferred`
/// tools are never listed and do not change the description at all, so it stays the same while
/// MCP servers connect or change their tools. Tool sections share `inline_budget` tokens.
/// `extra_globals` adds a line to the list of globals for each further global the host offers.
pub fn description(
    listed: &[CallableTool],
    inline_budget: Option<usize>,
    extra_globals: &[String],
) -> String {
    let listed: Vec<&CallableTool> = listed
        .iter()
        .filter(|tool| tool.definition.name != CODEMODE_TOOL_NAME)
        .filter(|tool| tool.exposure != ToolExposure::Deferred)
        .collect();

    let mut groups: Vec<Group> = vec![Group {
        namespace: None,
        entries: Vec::new(),
    }];
    let mut declarations = Vec::new();
    for tool in &listed {
        let declaration = declaration_of(tool);
        let section = section(&declaration);
        let entry = Entry {
            name: declaration.name.clone(),
            cost: section.len().div_ceil(CHARS_PER_TOKEN),
            section,
        };
        let index = match &tool.namespace {
            None => 0,
            Some(namespace) => match groups.iter().position(|group| {
                group
                    .namespace
                    .as_ref()
                    .is_some_and(|existing| existing.name == namespace.name)
            }) {
                Some(index) => index,
                None => {
                    groups.push(Group {
                        namespace: Some(namespace.clone()),
                        entries: Vec::new(),
                    });
                    groups.len() - 1
                }
            },
        };
        groups[index].entries.push(entry);
        declarations.push(declaration);
    }
    groups[1..].sort_by(|left, right| {
        let name = |group: &Group| group.namespace.as_ref().map(|namespace| namespace.name.clone());
        name(left).cmp(&name(right))
    });
    let shown = select(&groups, inline_budget);

    let mut globals = GLOBALS.to_string();
    for line in extra_globals {
        globals.push_str("\n- ");
        globals.push_str(line);
    }
    let mut sections = vec![INTRO.to_string(), globals];
    if declarations.iter().any(|declaration| {
        shown.contains(&declaration.name)
            && mcp_structured_content_schema(declaration.output_schema.as_ref()).is_some()
    }) {
        sections.push(format!(
            "Shared MCP Types:\n```ts\n{MCP_TYPESCRIPT_PREAMBLE}\n```"
        ));
    }
    if declarations.is_empty() {
        return sections.join("\n\n");
    }

    let mut tool_sections = vec!["Nested tools:".to_string()];
    for group in &groups {
        let visible: Vec<&Entry> = group
            .entries
            .iter()
            .filter(|entry| shown.contains(&entry.name))
            .collect();
        if let Some(namespace) = &group.namespace {
            // Only tools that did not fit the budget are counted as not listed here.
            let listing = match (visible.len(), group.entries.len()) {
                (visible, all) if visible == all => "",
                (0, _) => " (tools not listed)",
                _ => " (some tools not listed)",
            };
            let mut heading = format!("## {}{listing}", namespace.name);
            if let Some(description) = namespace
                .description
                .as_deref()
                .map(str::trim)
                .filter(|description| !description.is_empty())
            {
                heading.push('\n');
                heading.push_str(description);
            }
            tool_sections.push(heading);
        }
        tool_sections.extend(visible.iter().map(|entry| entry.section.clone()));
    }
    sections.push(tool_sections.join("\n\n"));
    sections.join("\n\n")
}

/// What a call resolves to, in a few words: `a string`, the field names of an object, or the
/// rendered type.
fn describe_output(schema: Option<&Value>) -> String {
    let rendered = render_output_type(schema);
    if rendered == "string" {
        return "a string".to_string();
    }
    if let Some(schema) = schema.filter(|schema| {
        schema.get("type").and_then(Value::as_str) == Some("object")
            && mcp_structured_content_schema(Some(schema)).is_none()
    }) {
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            let required: Vec<&str> = schema
                .get("required")
                .and_then(Value::as_array)
                .map(|required| required.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let fields: Vec<String> = properties
                .keys()
                .map(|name| match required.contains(&name.as_str()) {
                    true => name.clone(),
                    false => format!("{name}?"),
                })
                .collect();
            return format!("`{{ {} }}`", fields.join(", "));
        }
    }
    format!(
        "`{}`",
        rendered.split_whitespace().collect::<Vec<_>>().join(" ")
    )
}

/// A declared tool's description followed by how scripts call it and what the call resolves to.
fn describe_script_call(tool: &CallableTool) -> String {
    let declaration = declaration_of(tool);
    format!(
        "{}\n\nCodemode: `tools.{}(args)` resolves to {}.",
        tool.definition.description.trim(),
        identifier(&tool.definition.name),
        describe_output(declaration.output_schema.as_ref())
    )
}

/// How `codemode` presents the tools that are both declared and callable from scripts. With
/// `On`, their descriptions say how scripts call them and the `codemode` description lists only
/// the callable tools that are not declared directly; with `Only`, the `codemode` description
/// lists every callable tool and requests leave out the declarations of `direct` tools. Listing
/// by exposure rather than by what is declared keeps the description the same when a search
/// loads a tool.
pub fn prepare_loadout(
    loadout: &Loadout,
    mode: Mode,
    inline_budget: Option<usize>,
    extra_globals: &[String],
) -> LoadoutChanges {
    let callable: Vec<&CallableTool> = loadout
        .callable
        .iter()
        .filter(|tool| tool.definition.name != CODEMODE_TOOL_NAME)
        .collect();
    let is_callable =
        |name: &str| callable.iter().any(|tool| tool.definition.name == name);
    let mut changes = LoadoutChanges::default();
    if mode == Mode::On {
        for tool in &loadout.declared {
            if is_callable(&tool.definition.name) {
                changes
                    .descriptions
                    .insert(tool.definition.name.clone(), describe_script_call(tool));
            }
        }
    }
    let listed: Vec<CallableTool> = callable
        .iter()
        .filter(|tool| mode == Mode::Only || tool.exposure != ToolExposure::Direct)
        .map(|tool| (*tool).clone())
        .collect();
    changes.descriptions.insert(
        CODEMODE_TOOL_NAME.to_string(),
        description(&listed, inline_budget, extra_globals),
    );
    if mode == Mode::Only {
        changes.hidden = loadout
            .declared
            .iter()
            .filter(|tool| tool.exposure == ToolExposure::Direct && is_callable(&tool.definition.name))
            .map(|tool| tool.definition.name.clone())
            .collect();
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_types::ToolDefinition;

    fn tool(name: &str, exposure: ToolExposure) -> CallableTool {
        CallableTool {
            definition: ToolDefinition {
                name: name.into(),
                description: format!("The {name} tool"),
                parameters: json!({ "type": "object", "properties": { "path": { "type": "string" } } }),
                constrained_sampling: None,
            },
            exposure,
            namespace: None,
            annotations: None,
            output_schema: None,
        }
    }

    fn in_namespace(mut tool: CallableTool, namespace: &str) -> CallableTool {
        tool.namespace = Some(ToolNamespace {
            name: namespace.into(),
            description: Some(format!("All about {namespace}")),
            instructions: None,
        });
        tool
    }

    #[test]
    fn deferred_tools_are_never_listed() {
        let described = description(
            &[
                tool("listed", ToolExposure::Codemode),
                tool("found_later", ToolExposure::Deferred),
            ],
            None,
            &[],
        );
        assert!(described.contains("### `listed`"), "{described}");
        assert!(!described.contains("found_later"), "{described}");
    }

    #[test]
    fn namespaces_get_one_heading_and_odd_names_their_identifier() {
        let described = description(
            &[in_namespace(tool("mcp__my-server__x", ToolExposure::Codemode), "mcp__my-server")],
            None,
            &[],
        );
        assert!(described.contains("## mcp__my-server\nAll about mcp__my-server"));
        assert!(described.contains("### `mcp__my_server__x` (`mcp__my-server__x`)"));
    }

    #[test]
    fn a_tight_budget_leaves_tools_out_and_says_so() {
        let described = description(
            &[
                in_namespace(tool("a", ToolExposure::Codemode), "one"),
                in_namespace(tool("b", ToolExposure::Codemode), "one"),
            ],
            Some(60),
            &[],
        );
        assert!(described.contains("## one (some tools not listed)"), "{described}");
        let nothing = description(&[in_namespace(tool("a", ToolExposure::Codemode), "one")], Some(0), &[]);
        assert!(nothing.contains("## one (tools not listed)"), "{nothing}");
    }

    #[test]
    fn on_mode_keeps_declared_tools_and_says_how_scripts_call_them() {
        let loadout = Loadout {
            declared: vec![tool("read", ToolExposure::Direct), tool(CODEMODE_TOOL_NAME, ToolExposure::ModelOnly)],
            callable: vec![tool("read", ToolExposure::Direct), tool("helper", ToolExposure::Codemode)],
        };
        let changes = prepare_loadout(&loadout, Mode::On, None, &[]);
        assert!(changes.hidden.is_empty());
        assert!(changes.descriptions["read"].ends_with("Codemode: `tools.read(args)` resolves to a string."));
        let codemode = &changes.descriptions[CODEMODE_TOOL_NAME];
        assert!(codemode.contains("### `helper`"));
        assert!(!codemode.contains("### `read`"));
    }

    #[test]
    fn only_mode_hides_declared_tools_and_lists_them_instead() {
        let loadout = Loadout {
            declared: vec![tool("read", ToolExposure::Direct), tool(CODEMODE_TOOL_NAME, ToolExposure::ModelOnly)],
            callable: vec![tool("read", ToolExposure::Direct)],
        };
        let changes = prepare_loadout(&loadout, Mode::Only, None, &[]);
        assert_eq!(changes.hidden, vec!["read"]);
        assert!(changes.descriptions[CODEMODE_TOOL_NAME].contains("### `read`"));
        assert!(!changes.descriptions.contains_key("read"));
    }

    #[test]
    fn further_globals_are_listed_with_the_built_in_ones() {
        let described = description(&[], None, &["`models`: run models".to_string()]);
        assert!(described.contains("describeNamespace(name)`: find unlisted tools, such as MCP tools.\n- `models`: run models"));
    }

    #[test]
    fn structured_outputs_are_described_by_their_fields() {
        let schema = json!({
            "type": "object",
            "properties": { "output": { "type": "string" }, "full_output_path": { "type": "string" } },
            "required": ["output"],
        });
        let described = describe_output(Some(&schema));
        assert!(described.starts_with("`{ ") && described.ends_with(" }`"), "{described}");
        assert!(described.contains("full_output_path?"), "{described}");
        assert!(described.contains("output,") || described.ends_with("output }`"), "{described}");
        assert_eq!(describe_output(Some(&json!({ "type": "string" }))), "a string");
    }
}
