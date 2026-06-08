//! End-to-end tests against the public API: `render`, `diagram_kind` and `source_box`.

use micro_mermaid::{diagram_kind, render, source_box, Cls, DiagramKind};

fn plain(src: &str) -> Vec<String> {
    render(src)
        .unwrap_or_else(|| panic!("expected {src:?} to render"))
        .plain
}

#[test]
fn a_simple_chain_flows_left_to_right() {
    let art = render("flowchart LR\n  A[Start] --> B[Done]").unwrap();
    assert_eq!(
        art.plain,
        vec![
            "┌───────┐    ┌──────┐",
            "│ Start ├───▶│ Done │",
            "└───────┘    └──────┘",
        ]
    );
    assert_eq!(art.width, 21);
    assert!(art.warnings.is_empty());
}

#[test]
fn spans_carry_the_class_each_run_of_cells_belongs_to() {
    let art = render("flowchart LR\n  A[Start] --> B[Done]").unwrap();
    let middle = &art.styled[1];
    let classes: Vec<Cls> = middle.iter().map(|s| s.cls).collect();
    assert_eq!(
        classes,
        vec![
            Cls::Border,
            Cls::None,
            Cls::Text,
            Cls::None,
            Cls::Border,
            Cls::Edge,
            Cls::Border,
            Cls::None,
            Cls::Text,
            Cls::None,
            Cls::Border,
        ]
    );
    assert_eq!(middle[2].text, "Start");
    assert_eq!(middle[5].text, "───▶");
}

#[test]
fn a_branch_puts_its_boxes_side_by_side() {
    assert_eq!(
        plain("flowchart TD\n  A --> B\n  A --> C"),
        vec![
            "     ┌───┐",
            "     │ A │",
            "     └─┬─┘",
            "   ┌───┴───┐",
            "   ▼       ▼",
            " ┌───┐   ┌───┐",
            " │ B │   │ C │",
            " └───┘   └───┘",
        ]
    );
}

#[test]
fn subgraphs_draw_as_nested_frames() {
    let src = "flowchart TD\n  subgraph one\n    A --> B\n  end\n  subgraph two\n    C --> D\n  end\n  B --> C";
    assert_eq!(
        plain(src),
        vec![
            "┌ one ─┐",
            "│ ┌───┐│",
            "│ │ A ││",
            "│ └─┬─┘│",
            "│   │  │",
            "│   ▼  │",
            "│ ┌───┐│",
            "│ │ B ││",
            "│ └───┘│",
            "└───┬──┘",
            "    │",
            "    ▼",
            "┌ two ─┐",
            "│ ┌───┐│",
            "│ │ C ││",
            "│ └─┬─┘│",
            "│   │  │",
            "│   ▼  │",
            "│ ┌───┐│",
            "│ │ D ││",
            "│ └───┘│",
            "└──────┘",
        ]
    );
}

#[test]
fn edge_labels_sit_beside_their_arrows() {
    assert_eq!(
        plain("flowchart LR\n  A -->|yes| B\n  A -->|no| C"),
        vec![
            "      yes  ┌───┐",
            "     ┌────▶│ B │",
            "┌───┐│     └───┘",
            "│ A ├┤",
            "└───┘│no   ┌───┐",
            "     └────▶│ C │",
            "           └───┘",
        ]
    );
}

#[test]
fn each_arrowhead_gets_its_own_glyph() {
    let src = "flowchart LR\n  A --> B\n  A --o C\n  A --x D\n  A --> E";
    assert_eq!(
        plain(src),
        vec![
            "         ┌───┐",
            "     ┌──▶│ B │",
            "     │   └───┘",
            "     │",
            "     │   ┌───┐",
            "     ├──o│ C │",
            "┌───┐│   └───┘",
            "│ A ├┤",
            "└───┘│   ┌───┐",
            "     ├──×│ D │",
            "     │   └───┘",
            "     │",
            "     │   ┌───┐",
            "     └──▶│ E │",
            "         └───┘",
        ]
    );
}

#[test]
fn each_line_kind_gets_its_own_stroke() {
    let src = "flowchart LR\n  A --> B\n  A -.-> C\n  A ==> D";
    assert_eq!(
        plain(src),
        vec![
            "         ┌───┐",
            "     ┌──▶│ B │",
            "     │   └───┘",
            "     │",
            "┌───┐│   ┌───┐",
            "│ A ├┼╌╌▶│ C │",
            "└───┘┃   └───┘",
            "     ┃",
            "     ┃   ┌───┐",
            "     ┗━━▶│ D │",
            "         └───┘",
        ]
    );
}

#[test]
fn a_self_edge_loops_below_its_box() {
    assert_eq!(
        plain("flowchart TD\n  A --> A"),
        vec![" ┌─────┐", " │  A  │", " └───┬─┘", "     │▲", "     ╰╯"]
    );
}

#[test]
fn bottom_to_top_flips_the_diagram_vertically() {
    assert_eq!(
        plain("flowchart BT\n  A --> B --> C"),
        vec![
            " ┌───┐",
            " │ C │",
            " └───┘",
            "   ▲",
            "   │",
            " ┌─┴─┐",
            " │ B │",
            " └───┘",
            "   ▲",
            "   │",
            " ┌─┴─┐",
            " │ A │",
            " └───┘",
        ]
    );
}

#[test]
fn right_to_left_flips_the_diagram_horizontally() {
    assert_eq!(
        plain("flowchart RL\n  A --> B"),
        vec!["┌───┐    ┌───┐", "│ B │◄───┤ A │", "└───┘    └───┘"]
    );
}

#[test]
fn an_unclosed_bracket_warns_but_still_renders() {
    let art = render("flowchart LR\n  A[Unclosed --> B").unwrap();
    assert_eq!(
        art.warnings,
        vec!["node \"A\": label is missing its closing `]`"]
    );
    assert_eq!(
        art.plain,
        vec![
            "┌────────────────┐",
            "│ Unclosed --> B │",
            "└────────────────┘"
        ]
    );
}

#[test]
fn an_unreadable_link_is_dropped_with_a_warning() {
    let art = render("flowchart LR\n  A[Foo]:::highlight --> B[Bar]").unwrap();
    assert_eq!(
        art.warnings,
        vec!["dropped, expected a link: \":::highlight --> B[Bar]\""]
    );
    assert_eq!(art.plain, vec!["┌─────┐", "│ Foo │", "└─────┘"]);
}

#[test]
fn every_unreadable_statement_gets_its_own_warning() {
    let src = "flowchart LR\n  A[Foo]:::highlight --> B[Bar]\n  C[Baz]:::other --> D[Qux]";
    let art = render(src).unwrap();
    assert_eq!(
        art.warnings,
        vec![
            "dropped, expected a link: \":::highlight --> B[Bar]\"",
            "dropped, expected a link: \":::other --> D[Qux]\"",
        ]
    );
    assert_eq!(
        art.plain,
        vec![
            "┌─────┐",
            "│ Foo │",
            "└─────┘",
            "",
            "┌─────┐",
            "│ Baz │",
            "└─────┘",
        ]
    );
}

#[test]
fn too_many_nodes_refuses_to_render() {
    let mut src = "flowchart TD\n".to_string();
    for i in 0..130 {
        src.push_str(&format!("  n{i}[Node {i}]\n"));
    }
    assert!(render(&src).is_none());
    assert_eq!(diagram_kind(&src), Some(DiagramKind::Flowchart));
}

#[test]
fn too_many_edges_refuses_to_render() {
    let mut src = "flowchart TD\n".to_string();
    for i in 0..520 {
        src.push_str(&format!("  n{i} --> n{}\n", i + 1));
    }
    assert!(render(&src).is_none());
}

#[test]
fn too_many_subgraphs_refuses_to_render() {
    let mut src = "flowchart TD\n".to_string();
    for i in 0..25 {
        src.push_str(&format!("subgraph g{i}\n"));
    }
    src.push_str("A --> B\n");
    for _ in 0..25 {
        src.push_str("end\n");
    }
    assert!(render(&src).is_none());
}

#[test]
fn subgraphs_nested_past_the_depth_cap_refuse_to_render() {
    let mut src = "flowchart TD\n".to_string();
    for i in 0..7 {
        src.push_str(&format!("subgraph g{i}\n"));
    }
    src.push_str("A --> B\n");
    for _ in 0..7 {
        src.push_str("end\n");
    }
    assert!(render(&src).is_none());
}

#[test]
fn a_state_diagram_draws_start_and_end_markers() {
    let src = "stateDiagram-v2\n  [*] --> Still\n  Still --> Moving\n  Moving --> Still\n  Moving --> Crash\n  Crash --> [*]";
    assert_eq!(
        plain(src),
        vec![
            "   ╭───╮",
            "   │ ● │",
            "   ╰─┬─╯",
            "     │",
            "     ▼",
            " ╭───────╮",
            " │ Still │◄┐",
            " ╰───┬───╯ │",
            "     │     │",
            "     ▼     │",
            "╭────────╮ │",
            "│ Moving ├─┘",
            "╰────┬───╯",
            "     │",
            "     ▼",
            " ╭───────╮",
            " │ Crash │",
            " ╰───┬───╯",
            "     │",
            "     ▼",
            "   ╭───╮",
            "   │ ● │",
            "   ╰───╯",
        ]
    );
}

#[test]
fn a_bad_final_line_is_dropped_and_reported() {
    let src = "stateDiagram-v2\n  [*] --> A\n  A --> B\n  this is not valid state syntax at all";
    let art = render(src).unwrap();
    assert_eq!(
        art.warnings,
        vec!["dropped, unreadable final line: \"this is not valid state syntax at all\""]
    );
    assert_eq!(
        art.plain,
        vec![
            " ╭───╮",
            " │ ● │",
            " ╰─┬─╯",
            "   │",
            "   ▼",
            " ╭───╮",
            " │ A │",
            " ╰─┬─╯",
            "   │",
            "   ▼",
            " ╭───╮",
            " │ B │",
            " ╰───╯",
        ]
    );
}

#[test]
fn a_class_diagram_divides_its_box_into_compartments() {
    let src = "classDiagram\n  class Animal {\n    +String name\n    +makeSound()\n  }\n  class Dog\n  Animal <|-- Dog";
    assert_eq!(
        plain(src),
        vec![
            "┌──────────────┐",
            "│    Animal    │",
            "├──────────────┤",
            "│ +String name │",
            "├──────────────┤",
            "│ +makeSound() │",
            "└───────△──────┘",
            "        │",
            "        │",
            "     ┌─────┐",
            "     │ Dog │",
            "     └─────┘",
        ]
    );
}

#[test]
fn inheritance_draws_a_triangle_head() {
    assert_eq!(
        plain("classDiagram\n  Animal <|-- Dog"),
        vec![
            "┌────────┐",
            "│ Animal │",
            "└────△───┘",
            "     │",
            "     │",
            "  ┌─────┐",
            "  │ Dog │",
            "  └─────┘"
        ]
    );
}

#[test]
fn composition_and_aggregation_draw_diamond_heads() {
    assert_eq!(
        plain("classDiagram\n  A *-- B\n  A o-- C"),
        vec![
            "     ┌───┐",
            "     │ A │",
            "     └─◇─┘",
            "   ┌───┴───┐",
            "   │       │",
            " ┌───┐   ┌───┐",
            " │ B │   │ C │",
            " └───┘   └───┘",
        ]
    );
}

#[test]
fn a_dependency_relation_draws_a_dotted_line() {
    assert_eq!(
        plain("classDiagram\n  A ..> B"),
        vec![
            " ┌───┐",
            " │ A │",
            " └─┬─┘",
            "   ╎",
            "   ▼",
            " ┌───┐",
            " │ B │",
            " └───┘"
        ]
    );
}

#[test]
fn an_unparseable_class_diagram_renders_nothing() {
    let src = "classDiagram\n  this is not valid at all !!!";
    assert!(render(src).is_none());

    assert_eq!(diagram_kind(src), Some(DiagramKind::Class));
}

#[test]
fn an_er_diagram_shows_crows_foot_cardinality() {
    let src = "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n  ORDER ||--|{ LINE-ITEM : contains";
    assert_eq!(
        plain(src),
        vec![
            " ┌──────────┐",
            " │ CUSTOMER │",
            " └─────┬────┘",
            "       │",
            "       │1 places 0..*",
            "   ┌───────┐",
            "   │ ORDER │",
            "   └───┬───┘",
            "       │",
            "       │1 contains 1..*",
            " ┌───────────┐",
            " │ LINE-ITEM │",
            " └───────────┘",
        ]
    );
}

#[test]
fn a_sequence_diagram_draws_lifelines_and_messages() {
    let src = "sequenceDiagram\n  Alice->>Bob: Hello Bob, how are you?\n  Bob-->>Alice: I am good thanks!";
    assert_eq!(
        plain(src),
        vec![
            "┌───────┐                 ┌─────┐",
            "│ Alice │                 │ Bob │",
            "└───┬───┘                 └──┬──┘",
            "    │                        │",
            "    │Hello Bob, how are you? │",
            "    ├───────────────────────▶│",
            "    │                        │",
            "    │   I am good thanks!    │",
            "    │◄╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤",
            "    │                        │",
            "┌───┴───┐                 ┌──┴──┐",
            "│ Alice │                 │ Bob │",
            "└───────┘                 └─────┘",
        ]
    );
}

#[test]
fn a_sequence_note_spans_its_participants() {
    let src =
        "sequenceDiagram\n  participant A\n  participant B\n  Note over A,B: a note\n  A->>B: hi";
    assert_eq!(
        plain(src),
        vec![
            "┌───┐  ┌───┐",
            "│ A │  │ B │",
            "└─┬─┘  └─┬─┘",
            "  │      │",
            "┌──────────┐",
            "│  a note  │",
            "└──────────┘",
            "  │      │",
            "  │  hi  │",
            "  ├─────▶│",
            "  │      │",
            "┌─┴─┐  ┌─┴─┐",
            "│ A │  │ B │",
            "└───┘  └───┘",
        ]
    );
}

#[test]
fn a_sequence_self_message_loops_back() {
    assert_eq!(
        plain("sequenceDiagram\n  A->>A: thinking"),
        vec![
            "┌───┐",
            "│ A │",
            "└─┬─┘",
            "  │",
            "  ├──╮",
            "  │  │ thinking",
            "  │◄─╯",
            "  │",
            "┌─┴─┐",
            "│ A │",
            "└───┘",
        ]
    );
}

#[test]
fn a_sequence_loop_block_draws_dividers() {
    let src = "sequenceDiagram\n  A->>B: hi\n  loop every day\n    A->>B: hi again\n  end";
    assert_eq!(
        plain(src),
        vec![
            "┌───┐     ┌───┐",
            "│ A │     │ B │",
            "└─┬─┘     └─┬─┘",
            "  │         │",
            "  │   hi    │",
            "  ├────────▶│",
            "  │         │",
            "── loop every day",
            "  │         │",
            "  │hi again │",
            "  ├────────▶│",
            "  │         │",
            "── end ───────────",
            "  │         │",
            "┌─┴─┐     ┌─┴─┐",
            "│ A │     │ B │",
            "└───┘     └───┘",
        ]
    );
}

#[test]
fn diagram_kind_is_none_for_a_grammar_this_renderer_does_not_draw() {
    assert_eq!(diagram_kind("zenuml\n  A->B: hi"), None);
    assert!(render("zenuml\n  A->B: hi").is_none());
    assert_eq!(diagram_kind("C4Context\n  title System"), None);
}

/// A pie is not one of the five the rest of this crate draws as graphs, but it is drawn.
#[test]
fn a_pie_chart_is_drawn_as_bars() {
    let art = render("pie title Pets\n  \"Dogs\" : 75\n  \"Cats\" : 25").expect("drawn");
    assert_eq!(art.plain[0], "Pets");
    assert!(art.plain[1].contains("75 (75.0%)"), "{:?}", art.plain);
    assert!(art.plain[1].contains('█'));
}

#[test]
fn blank_source_renders_nothing() {
    assert!(render("   \n\n  ").is_none());
}

#[test]
fn diagram_kind_recognises_every_supported_grammar() {
    assert_eq!(
        diagram_kind("flowchart LR\n  A --> B"),
        Some(DiagramKind::Flowchart)
    );
    assert_eq!(
        diagram_kind("graph TD\n  A --> B"),
        Some(DiagramKind::Flowchart)
    );
    assert_eq!(
        diagram_kind("stateDiagram-v2\n  [*] --> A"),
        Some(DiagramKind::State)
    );
    assert_eq!(
        diagram_kind("classDiagram\n  class A"),
        Some(DiagramKind::Class)
    );
    assert_eq!(
        diagram_kind("erDiagram\n  A ||--o{ B : has"),
        Some(DiagramKind::Er)
    );
    assert_eq!(
        diagram_kind("sequenceDiagram\n  A->>B: hi"),
        Some(DiagramKind::Sequence)
    );
}

#[test]
fn source_box_frames_an_unsupported_diagram() {
    let art = source_box("pie\n  title Pets\n  \"Dogs\" : 4", 20);
    assert_eq!(
        art.plain,
        vec![
            "╭ mermaid: pie ──╮",
            "│ pie            │",
            "│   title Pets   │",
            "│   \"Dogs\" : 4   │",
            "╰────────────────╯",
        ]
    );
    assert_eq!(art.width, 18);
    assert!(art.warnings.is_empty());
}

#[test]
fn source_box_wraps_long_lines_to_the_column_limit() {
    let art = source_box(
        "this is a very long single line that should get wrapped at the column limit for sure",
        20,
    );
    assert_eq!(
        art.plain,
        vec![
            "╭ mermaid: this ───╮",
            "│ this is a very l │",
            "│ ong single line  │",
            "│ that should get  │",
            "│ wrapped at the c │",
            "│ olumn limit for  │",
            "│ sure             │",
            "╰──────────────────╯",
        ]
    );
    assert_eq!(art.width, 20);
}
