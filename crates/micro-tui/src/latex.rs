//! Maths, written the way a terminal can show it.
//!
//! A model asked about anything mathematical answers in LaTeX, and a terminal has no way
//! to typeset it. What it does have is a large part of the same alphabet: Unicode carries
//! the Greek letters, the operators, the arrows, and enough superscripts and subscripts to
//! read an exponent. Rendering here means substituting those, and arranging the few
//! structures — fractions, roots, accents — that have no single character.
//!
//! What cannot be shown is left as it was written. A reader who knows LaTeX can still read
//! `\begin{matrix}`; a reader who does not is no worse off than with a blank.

/// Symbols, as ohm maps them.
const SYMBOLS: &[(&str, &str)] = &[
    ("alpha", "α"),
    ("beta", "β"),
    ("gamma", "γ"),
    ("delta", "δ"),
    ("epsilon", "ϵ"),
    ("varepsilon", "ε"),
    ("zeta", "ζ"),
    ("eta", "η"),
    ("theta", "θ"),
    ("vartheta", "ϑ"),
    ("iota", "ι"),
    ("kappa", "κ"),
    ("varkappa", "ϰ"),
    ("lambda", "λ"),
    ("mu", "μ"),
    ("nu", "ν"),
    ("xi", "ξ"),
    ("pi", "π"),
    ("varpi", "ϖ"),
    ("rho", "ρ"),
    ("varrho", "ϱ"),
    ("sigma", "σ"),
    ("varsigma", "ς"),
    ("tau", "τ"),
    ("upsilon", "υ"),
    ("phi", "ϕ"),
    ("varphi", "φ"),
    ("chi", "χ"),
    ("psi", "ψ"),
    ("omega", "ω"),
    ("Gamma", "Γ"),
    ("Delta", "Δ"),
    ("Theta", "Θ"),
    ("Lambda", "Λ"),
    ("Xi", "Ξ"),
    ("Pi", "Π"),
    ("Sigma", "Σ"),
    ("Upsilon", "Υ"),
    ("Phi", "Φ"),
    ("Psi", "Ψ"),
    ("Omega", "Ω"),
    ("pm", "±"),
    ("mp", "∓"),
    ("times", "×"),
    ("div", "÷"),
    ("cdot", "·"),
    ("ast", "∗"),
    ("star", "⋆"),
    ("circ", "∘"),
    ("bullet", "•"),
    ("oplus", "⊕"),
    ("ominus", "⊖"),
    ("otimes", "⊗"),
    ("oslash", "⊘"),
    ("odot", "⊙"),
    ("bigcirc", "○"),
    ("dagger", "†"),
    ("ddagger", "‡"),
    ("amalg", "⨿"),
    ("uplus", "⊎"),
    ("sqcap", "⊓"),
    ("sqcup", "⊔"),
    ("triangleleft", "◁"),
    ("triangleright", "▷"),
    ("wr", "≀"),
    ("cap", "∩"),
    ("cup", "∪"),
    ("bigcap", "⋂"),
    ("bigcup", "⋃"),
    ("bigwedge", "⋀"),
    ("bigvee", "⋁"),
    ("bigsqcup", "⨆"),
    ("biguplus", "⨄"),
    ("bigoplus", "⨁"),
    ("bigotimes", "⨂"),
    ("bigodot", "⨀"),
    ("setminus", "∖"),
    ("in", "∈"),
    ("notin", "∉"),
    ("ni", "∋"),
    ("subset", "⊂"),
    ("supset", "⊃"),
    ("subseteq", "⊆"),
    ("supseteq", "⊇"),
    ("sqsubset", "⊏"),
    ("sqsupset", "⊐"),
    ("sqsubseteq", "⊑"),
    ("sqsupseteq", "⊒"),
    ("prec", "≺"),
    ("preceq", "≼"),
    ("succ", "≻"),
    ("succeq", "≽"),
    ("ll", "≪"),
    ("gg", "≫"),
    ("le", "≤"),
    ("leq", "≤"),
    ("leqslant", "≤"),
    ("ge", "≥"),
    ("geq", "≥"),
    ("geqslant", "≥"),
    ("ne", "≠"),
    ("neq", "≠"),
    ("equiv", "≡"),
    ("approx", "≈"),
    ("sim", "∼"),
    ("simeq", "≃"),
    ("cong", "≅"),
    ("asymp", "≍"),
    ("doteq", "≐"),
    ("propto", "∝"),
    ("parallel", "∥"),
    ("perp", "⊥"),
    ("mid", "∣"),
    ("vdash", "⊢"),
    ("dashv", "⊣"),
    ("models", "⊨"),
    ("Vdash", "⊩"),
    ("Vvdash", "⊪"),
    ("nvdash", "⊬"),
    ("nvDash", "⊭"),
    ("forall", "∀"),
    ("exists", "∃"),
    ("nexists", "∄"),
    ("neg", "¬"),
    ("land", "∧"),
    ("wedge", "∧"),
    ("lor", "∨"),
    ("vee", "∨"),
    ("to", "→"),
    ("rightarrow", "→"),
    ("longrightarrow", "→"),
    ("leftarrow", "←"),
    ("longleftarrow", "←"),
    ("gets", "←"),
    ("leftrightarrow", "↔"),
    ("longleftrightarrow", "↔"),
    ("hookleftarrow", "↩"),
    ("hookrightarrow", "↪"),
    ("twoheadleftarrow", "↞"),
    ("twoheadrightarrow", "↠"),
    ("leftharpoonup", "↼"),
    ("leftharpoondown", "↽"),
    ("rightharpoonup", "⇀"),
    ("rightharpoondown", "⇁"),
    ("rightleftharpoons", "⇌"),
    ("leftrightharpoons", "⇋"),
    ("nearrow", "↗"),
    ("searrow", "↘"),
    ("swarrow", "↙"),
    ("nwarrow", "↖"),
    ("rightsquigarrow", "⇝"),
    ("leadsto", "⇝"),
    ("Rightarrow", "⇒"),
    ("Longrightarrow", "⇒"),
    ("Leftarrow", "⇐"),
    ("Longleftarrow", "⇐"),
    ("Leftrightarrow", "⇔"),
    ("Longleftrightarrow", "⇔"),
    ("implies", "⇒"),
    ("iff", "⇔"),
    ("mapsto", "↦"),
    ("longmapsto", "↦"),
    ("uparrow", "↑"),
    ("downarrow", "↓"),
    ("partial", "∂"),
    ("nabla", "∇"),
    ("int", "∫"),
    ("iint", "∬"),
    ("iiint", "∭"),
    ("oint", "∮"),
    ("sum", "∑"),
    ("prod", "∏"),
    ("coprod", "∐"),
    ("infty", "∞"),
    ("emptyset", "∅"),
    ("varnothing", "∅"),
    ("angle", "∠"),
    ("therefore", "∴"),
    ("because", "∵"),
    ("aleph", "ℵ"),
    ("beth", "ℶ"),
    ("gimel", "ℷ"),
    ("daleth", "ℸ"),
    ("top", "⊤"),
    ("bot", "⊥"),
    ("triangle", "△"),
    ("square", "□"),
    ("lozenge", "◊"),
    ("checkmark", "✓"),
    ("complement", "∁"),
    ("wp", "℘"),
    ("prime", "′"),
    ("ldots", "…"),
    ("dots", "…"),
    ("cdots", "⋯"),
    ("vdots", "⋮"),
    ("ddots", "⋱"),
    ("ell", "ℓ"),
    ("hbar", "ℏ"),
    ("Im", "ℑ"),
    ("Re", "ℜ"),
    ("langle", "⟨"),
    ("rangle", "⟩"),
    ("vert", "|"),
    ("lvert", "|"),
    ("rvert", "|"),
    ("Vert", "‖"),
    ("lVert", "‖"),
    ("rVert", "‖"),
    ("lbrace", "{"),
    ("rbrace", "}"),
    ("backslash", "\\"),
    ("lfloor", "⌊"),
    ("rfloor", "⌋"),
    ("lceil", "⌈"),
    ("rceil", "⌉"),
    ("colon", ":"),
];

/// Superscripts, as ohm maps them.
const SUPERSCRIPTS: &[(&str, &str)] = &[
    ("0", "⁰"),
    ("1", "¹"),
    ("2", "²"),
    ("3", "³"),
    ("4", "⁴"),
    ("5", "⁵"),
    ("6", "⁶"),
    ("7", "⁷"),
    ("8", "⁸"),
    ("9", "⁹"),
    ("+", "⁺"),
    ("-", "⁻"),
    ("=", "⁼"),
    ("(", "⁽"),
    (")", "⁾"),
    ("a", "ᵃ"),
    ("b", "ᵇ"),
    ("c", "ᶜ"),
    ("d", "ᵈ"),
    ("e", "ᵉ"),
    ("f", "ᶠ"),
    ("g", "ᵍ"),
    ("h", "ʰ"),
    ("i", "ⁱ"),
    ("j", "ʲ"),
    ("k", "ᵏ"),
    ("l", "ˡ"),
    ("m", "ᵐ"),
    ("n", "ⁿ"),
    ("o", "ᵒ"),
    ("p", "ᵖ"),
    ("r", "ʳ"),
    ("s", "ˢ"),
    ("t", "ᵗ"),
    ("u", "ᵘ"),
    ("v", "ᵛ"),
    ("w", "ʷ"),
    ("x", "ˣ"),
    ("y", "ʸ"),
    ("z", "ᶻ"),
];

/// Subscripts, as ohm maps them.
const SUBSCRIPTS: &[(&str, &str)] = &[
    ("0", "₀"),
    ("1", "₁"),
    ("2", "₂"),
    ("3", "₃"),
    ("4", "₄"),
    ("5", "₅"),
    ("6", "₆"),
    ("7", "₇"),
    ("8", "₈"),
    ("9", "₉"),
    ("+", "₊"),
    ("-", "₋"),
    ("=", "₌"),
    ("(", "₍"),
    (")", "₎"),
    ("a", "ₐ"),
    ("e", "ₑ"),
    ("h", "ₕ"),
    ("i", "ᵢ"),
    ("j", "ⱼ"),
    ("k", "ₖ"),
    ("l", "ₗ"),
    ("m", "ₘ"),
    ("n", "ₙ"),
    ("o", "ₒ"),
    ("p", "ₚ"),
    ("r", "ᵣ"),
    ("s", "ₛ"),
    ("t", "ₜ"),
    ("u", "ᵤ"),
    ("v", "ᵥ"),
    ("x", "ₓ"),
];

/// Blackboard, as ohm maps them.
const BLACKBOARD: &[(&str, &str)] = &[
    ("C", "ℂ"),
    ("H", "ℍ"),
    ("N", "ℕ"),
    ("P", "ℙ"),
    ("Q", "ℚ"),
    ("R", "ℝ"),
    ("Z", "ℤ"),
];

/// Accents, as ohm maps them.
const ACCENTS: &[(&str, &str)] = &[
    ("acute", "\u{0301}"),
    ("bar", "\u{0305}"),
    ("breve", "\u{0306}"),
    ("check", "\u{030c}"),
    ("ddot", "\u{0308}"),
    ("dot", "\u{0307}"),
    ("grave", "\u{0300}"),
    ("hat", "\u{0302}"),
    ("mathring", "\u{030a}"),
    ("overleftarrow", "\u{20d6}"),
    ("overleftrightarrow", "\u{20e1}"),
    ("overline", "\u{0305}"),
    ("overrightarrow", "\u{20d7}"),
    ("tilde", "\u{0303}"),
    ("underline", "\u{0332}"),
    ("vec", "\u{20d7}"),
    ("widehat", "\u{0302}"),
    ("widetilde", "\u{0303}"),
];
/// Commands that stand for a word rather than a symbol: `\sin`, `\log`, `\max`.
const NAMED_OPERATORS: &[&str] = &[
    "arccos", "arcsin", "arctan", "arg", "cos", "cosh", "cot", "coth", "csc", "deg", "det", "dim",
    "exp", "gcd", "hom", "inf", "ker", "lg", "lim", "liminf", "limsup", "ln", "log", "max", "min",
    "Pr", "sec", "sin", "sinh", "sup", "tan", "tanh",
];

/// Commands that only affect spacing, which a terminal renders as one space or none.
const SPACING_COMMANDS: &[&str] = &[
    ",",
    ":",
    ";",
    " ",
    "quad",
    "qquad",
    "thinspace",
    "medspace",
    "thickspace",
    "enspace",
];

/// Commands that change size or style and mean nothing here.
const IGNORED_COMMANDS: &[&str] = &[
    "displaystyle",
    "textstyle",
    "scriptstyle",
    "scriptscriptstyle",
    "limits",
    "nolimits",
    "left",
    "right",
    "big",
    "Big",
    "bigg",
    "Bigg",
    "bigl",
    "bigr",
    "Bigl",
    "Bigr",
    "mathrm",
    "mathit",
    "mathsf",
    "mathtt",
    "mathnormal",
    "text",
    "textrm",
    "textit",
    "textbf",
    "operatorname",
];

/// Look a name up in one of the tables.
fn look_up<'a>(table: &'a [(&'a str, &'a str)], name: &str) -> Option<&'a str> {
    table
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| *value)
}

/// Map every character of `value` through a table, or nothing when one has no mapping.
///
/// All or nothing on purpose: half a superscript reads as a mistake rather than as maths,
/// so an exponent with no character for one of its digits is written the plain way.
fn map_all(value: &str, table: &[(&str, &str)]) -> Option<String> {
    value
        .chars()
        .map(|character| look_up(table, &character.to_string()).map(str::to_string))
        .collect()
}

/// Render LaTeX as text a terminal can show, or nothing when there is nothing to show.
/// Draw an expression the way display maths is set: a fraction stacked over its rule, a
/// big operator carrying its limits above and below.
///
/// Several rows, so it belongs on lines of its own rather than in a sentence.
pub fn render_display(source: &str) -> Option<String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut renderer = Renderer::styled(trimmed, true);
    let text = renderer.run();
    let drawn = lay_out(&text, &renderer.layout).lines.join("\n");
    let drawn = drawn.trim_matches('\n').to_string();
    (!drawn.trim().is_empty()).then_some(drawn)
}

/// A piece of drawn maths: its rows, how wide they are, and which row sits on the line
/// everything beside it is written on.
struct Drawn {
    lines: Vec<String>,
    width: usize,
    baseline: usize,
}

/// Set `text` out over as many rows as its stacked pieces need.
///
/// Every piece beside another is lined up on its baseline — the row a reader follows across
/// — so a fraction and the `=` next to it sit on the same line however tall the fraction is.
fn lay_out(text: &str, pieces: &[Stacked]) -> Drawn {
    let mut rows: Vec<String> = Vec::new();
    let mut first_baseline = 0;

    for line in text.split('\n') {
        let mut beside: Vec<Drawn> = Vec::new();
        let mut rest = line;

        while let Some(start) = rest.find(MARK_START) {
            if start > 0 {
                beside.push(flat(&rest[..start]));
            }
            let after = &rest[start + MARK_START.len_utf8()..];
            let Some(end) = after.find(MARK_END) else {
                break;
            };
            if let Some(piece) = after[..end]
                .parse::<usize>()
                .ok()
                .and_then(|i| pieces.get(i))
            {
                beside.push(draw_piece(piece, pieces));
            }
            rest = &after[end + MARK_END.len_utf8()..];
        }
        if !rest.is_empty() {
            beside.push(flat(rest));
        }

        let joined = alongside(&beside);
        if rows.is_empty() {
            first_baseline = joined.baseline;
        }
        rows.extend(joined.lines);
    }

    let width = rows
        .iter()
        .map(|row| crate::wrap::text_width(row))
        .max()
        .unwrap_or(0);
    Drawn {
        lines: rows,
        width,
        baseline: first_baseline,
    }
}

/// Text with nothing stacked in it: one row, and that row is the baseline.
fn flat(text: &str) -> Drawn {
    Drawn {
        width: crate::wrap::text_width(text),
        lines: vec![text.to_string()],
        baseline: 0,
    }
}

/// One stacked piece, drawn.
fn draw_piece(piece: &Stacked, pieces: &[Stacked]) -> Drawn {
    match piece {
        Stacked::Fraction { above, below } => {
            let above = lay_out(above, pieces);
            let below = lay_out(below, pieces);
            // A space each side of the rule, so a fraction does not touch what abuts it.
            let inner = above.width.max(below.width).max(1);
            let width = inner + 2;
            let mut lines: Vec<String> = above
                .lines
                .iter()
                .map(|line| centred(line, width))
                .collect();
            let baseline = lines.len();
            lines.push(format!(" {} ", "─".repeat(inner)));
            lines.extend(below.lines.iter().map(|line| centred(line, width)));
            Drawn {
                lines,
                width,
                baseline,
            }
        }
        Stacked::Operator {
            operator,
            upper,
            lower,
        } => {
            let inner = [
                crate::wrap::text_width(operator),
                upper.as_deref().map_or(0, crate::wrap::text_width),
                lower.as_deref().map_or(0, crate::wrap::text_width),
            ]
            .into_iter()
            .max()
            .unwrap_or(0);

            let mut lines = Vec::new();
            if let Some(upper) = upper {
                lines.push(format!("{} ", centred(upper, inner)));
            }
            let baseline = lines.len();
            lines.push(format!("{} ", centred(operator, inner)));
            if let Some(lower) = lower {
                lines.push(format!("{} ", centred(lower, inner)));
            }
            Drawn {
                lines,
                width: inner + 1,
                baseline,
            }
        }
    }
}

/// Pad `line` out to `width`, with what is left over split either side of it.
fn centred(line: &str, width: usize) -> String {
    let padding = width.saturating_sub(crate::wrap::text_width(line));
    let left = padding / 2;
    format!("{}{line}{}", " ".repeat(left), " ".repeat(padding - left))
}

/// Set pieces side by side, lined up on their baselines.
fn alongside(pieces: &[Drawn]) -> Drawn {
    if pieces.is_empty() {
        return Drawn {
            lines: vec![String::new()],
            width: 0,
            baseline: 0,
        };
    }
    let baseline = pieces.iter().map(|piece| piece.baseline).max().unwrap_or(0);
    let below = pieces
        .iter()
        .map(|piece| piece.lines.len() - piece.baseline - 1)
        .max()
        .unwrap_or(0);

    let mut lines = Vec::with_capacity(baseline + below + 1);
    for row in 0..=baseline + below {
        let mut line = String::new();
        for piece in pieces {
            // Where this row falls in a piece whose baseline may sit higher than the whole.
            let at = (row + piece.baseline).checked_sub(baseline);
            match at.and_then(|at| piece.lines.get(at)) {
                Some(text) => {
                    let padding = piece.width.saturating_sub(crate::wrap::text_width(text));
                    line.push_str(text);
                    line.push_str(&" ".repeat(padding));
                }
                None => line.push_str(&" ".repeat(piece.width)),
            }
        }
        lines.push(line.trim_end().to_string());
    }

    Drawn {
        width: pieces.iter().map(|piece| piece.width).sum(),
        lines,
        baseline,
    }
}

pub fn render(source: &str) -> Option<String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return None;
    }
    let rendered = Renderer::new(trimmed).run();
    let rendered = rendered.trim().to_string();
    (!rendered.is_empty()).then_some(rendered)
}

struct Renderer {
    characters: Vec<char>,
    at: usize,
    /// Whether a fraction is stacked over a rule and an operator's limits sit above and
    /// below it, rather than everything being written on one line.
    display: bool,
    /// The stacked pieces met so far. What is rendered carries a marker in their place,
    /// because how wide one is cannot be known until the line around it has been read.
    layout: Vec<Stacked>,
}

/// Something drawn over more than one row.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stacked {
    /// A numerator over a rule over a denominator.
    Fraction { above: String, below: String },
    /// A big operator with a limit above it, below it, or both.
    Operator {
        operator: String,
        upper: Option<String>,
        lower: Option<String>,
    },
}

/// Where a stacked piece stands in the line around it. Private-use characters, so nothing
/// a source could contain is mistaken for one.
const MARK_START: char = '\u{f0000}';
const MARK_END: char = '\u{f0001}';

/// The operators whose limits are written above and below them in display maths, rather
/// than beside them. ohm makes the same list.
const BIG_OPERATORS: [&str; 10] = ["∑", "∏", "∐", "∫", "∬", "∭", "∮", "⋃", "⋂", "⨆"];

/// Move every mark in `text` on by `base`, for splicing one renderer's pieces onto another's.
fn renumber(text: &str, base: usize) -> String {
    if base == 0 || !text.contains(MARK_START) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(MARK_START) {
        out.push_str(&rest[..start]);
        let after = &rest[start + MARK_START.len_utf8()..];
        let Some(end) = after.find(MARK_END) else {
            out.push_str(&rest[start..]);
            return out;
        };
        let index: usize = after[..end].parse().unwrap_or(0);
        out.push(MARK_START);
        out.push_str(&(index + base).to_string());
        out.push(MARK_END);
        rest = &after[end + MARK_END.len_utf8()..];
    }
    out.push_str(rest);
    out
}

impl Renderer {
    fn new(source: &str) -> Self {
        Renderer::styled(source, false)
    }

    fn styled(source: &str, display: bool) -> Self {
        Renderer {
            characters: source.chars().collect(),
            at: 0,
            display,
            layout: Vec::new(),
        }
    }

    /// Record a stacked piece and hand back the mark that stands in for it.
    fn stack(&mut self, piece: Stacked) -> String {
        self.layout.push(piece);
        format!("{MARK_START}{}{MARK_END}", self.layout.len() - 1)
    }

    fn run(&mut self) -> String {
        let mut out = String::new();
        while self.at < self.characters.len() {
            let piece = self.step();
            // A big operator's limits are written beside it on one line and above and below
            // it where there are three, which is what `\sum_{i=1}^{n}` is asking for.
            let piece = match self.display && BIG_OPERATORS.contains(&piece.trim()) {
                true => {
                    let operator = piece.trim().to_string();
                    self.limits(&operator)
                }
                false => piece,
            };
            out.push_str(&piece);
        }
        // A named operator sits against what follows it; a space keeps `\sin x` readable.
        out.replace("  ", " ")
    }

    /// The limits written on a big operator, taken in whichever order they were given.
    fn limits(&mut self, operator: &str) -> String {
        let (mut upper, mut lower) = (None, None);
        loop {
            match self.characters.get(self.at) {
                Some('^') => {
                    self.at += 1;
                    upper = Some(self.argument());
                }
                Some('_') => {
                    self.at += 1;
                    lower = Some(self.argument());
                }
                _ => break,
            }
        }
        match upper.is_none() && lower.is_none() {
            true => operator.to_string(),
            false => self.stack(Stacked::Operator {
                operator: operator.to_string(),
                upper,
                lower,
            }),
        }
    }

    /// Render a piece of source with this renderer's style, keeping whatever it stacks.
    ///
    /// A sub-renderer numbers its own stacked pieces from zero, so they are renumbered onto
    /// the end of this one's list and the marks in its text moved to match. Without that a
    /// fraction inside a fraction would point at the wrong piece.
    fn render_inner(&mut self, inner: &str) -> String {
        let mut sub = Renderer::styled(inner, self.display);
        let text = sub.run();
        let base = self.layout.len();
        self.layout.append(&mut sub.layout);
        renumber(&text, base)
    }

    /// One thing: a command, a group, a script, or a plain character.
    fn step(&mut self) -> String {
        let character = self.characters[self.at];
        match character {
            '\\' => self.command(),
            '{' => {
                self.at += 1;
                let inner = self.group_body();
                inner
            }
            '^' | '_' => {
                let kind = character;
                self.at += 1;
                let value = self.argument();
                self.script(kind, &value)
            }
            '$' => {
                // A delimiter that reached here is part of the text, not a boundary.
                self.at += 1;
                String::new()
            }
            _ => {
                self.at += 1;
                character.to_string()
            }
        }
    }

    /// The characters up to the matching close brace, rendered.
    fn group_body(&mut self) -> String {
        let mut depth = 1;
        let start = self.at;
        while self.at < self.characters.len() {
            match self.characters[self.at] {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            self.at += 1;
        }
        let inner: String = self.characters[start..self.at].iter().collect();
        if self.at < self.characters.len() {
            // Step over the closing brace.
            self.at += 1;
        }
        self.render_inner(&inner)
    }

    /// What a command or a script applies to: a group, another command, or one character.
    fn argument(&mut self) -> String {
        if self.at >= self.characters.len() {
            return String::new();
        }
        match self.characters[self.at] {
            '{' => {
                self.at += 1;
                self.group_body()
            }
            '\\' => self.command(),
            character => {
                self.at += 1;
                character.to_string()
            }
        }
    }

    /// The name after a backslash.
    fn command_name(&mut self) -> String {
        // Step over the backslash.
        self.at += 1;
        if self.at >= self.characters.len() {
            return String::new();
        }
        // A command whose name is punctuation is one character long.
        if !self.characters[self.at].is_ascii_alphabetic() {
            let single = self.characters[self.at];
            self.at += 1;
            return single.to_string();
        }
        let start = self.at;
        while self.at < self.characters.len() && self.characters[self.at].is_ascii_alphabetic() {
            self.at += 1;
        }
        self.characters[start..self.at].iter().collect()
    }

    fn command(&mut self) -> String {
        let name = self.command_name();

        match name.as_str() {
            "frac" | "dfrac" | "tfrac" => {
                let numerator = self.argument();
                let denominator = self.argument();
                // Written out on one line where there is only one line to write on, and
                // stacked over a rule where there is room for three.
                return match self.display {
                    true => self.stack(Stacked::Fraction {
                        above: numerator,
                        below: denominator,
                    }),
                    false => format!("({numerator})/({denominator})"),
                };
            }
            "sqrt" => {
                // An index in brackets makes it a root of that degree.
                let degree = self.bracketed();
                let value = self.argument();
                return match degree {
                    Some(degree) => format!("{degree}√({value})"),
                    None => format!("√({value})"),
                };
            }
            "mathbb" => {
                let value = self.argument();
                return map_all(&value, BLACKBOARD).unwrap_or(value);
            }
            // A style command shows its argument and nothing of itself.
            name if IGNORED_COMMANDS.contains(&name) => {
                return match self.characters.get(self.at) {
                    Some('{') => {
                        self.at += 1;
                        self.group_body()
                    }
                    _ => String::new(),
                };
            }
            name if SPACING_COMMANDS.contains(&name) => return " ".to_string(),
            name if NAMED_OPERATORS.contains(&name) => return format!(" {name}"),
            _ => {}
        }

        if let Some(accent) = look_up(ACCENTS, &name) {
            let value = self.argument();
            return format!("{value}{accent}");
        }
        if let Some(symbol) = look_up(SYMBOLS, &name) {
            return symbol.to_string();
        }

        // Nothing known by that name, so it is written as it was.
        match name.is_empty() {
            true => "\\".to_string(),
            false => format!("\\{name}"),
        }
    }

    /// A `[...]` argument, which is how a root's degree is given.
    fn bracketed(&mut self) -> Option<String> {
        if self.characters.get(self.at) != Some(&'[') {
            return None;
        }
        self.at += 1;
        let start = self.at;
        while self.at < self.characters.len() && self.characters[self.at] != ']' {
            self.at += 1;
        }
        let inner: String = self.characters[start..self.at].iter().collect();
        if self.at < self.characters.len() {
            self.at += 1;
        }
        let rendered = Renderer::new(&inner).run();
        map_all(&rendered, SUPERSCRIPTS).or(Some(rendered))
    }

    /// A superscript or subscript, raised or lowered where every character can be.
    fn script(&mut self, kind: char, value: &str) -> String {
        let table = match kind {
            '^' => SUPERSCRIPTS,
            _ => SUBSCRIPTS,
        };
        match map_all(value, table) {
            Some(mapped) => mapped,
            // Nothing to raise it with, so it is written the way it was typed.
            None => format!("{kind}({value})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Display maths is set over rows: a fraction stacked over its rule, and whatever is
    /// beside it lined up on that rule rather than on the top of it.
    #[test]
    fn a_fraction_is_stacked_over_its_rule() {
        assert_eq!(
            render_display(r"\frac{a}{b}").as_deref(),
            Some(" a\n ─\n b")
        );
        assert_eq!(
            render_display(r"E = \frac{x^2 + 1}{2y}").as_deref(),
            Some("     x² + 1\nE =  ──────\n       2y")
        );
    }

    /// A big operator carries its limits above and below rather than beside it.
    #[test]
    fn an_operators_limits_sit_above_and_below_it() {
        assert_eq!(
            render_display(r"\sum_{i=1}^{n} x_i").as_deref(),
            Some(" n\n ∑   xᵢ\ni=1")
        );
    }

    /// A fraction inside a fraction points at itself, not at whatever else was stacked.
    #[test]
    fn a_nested_fraction_keeps_its_place() {
        let drawn = render_display(r"\frac{\frac{a}{b}}{c}").expect("it draws");
        assert_eq!(drawn.lines().count(), 5, "{drawn}");
        assert!(drawn.contains('─'));
    }

    /// On one line a fraction is written the way it would be typed, as before.
    #[test]
    fn one_line_maths_is_unchanged_by_any_of_this() {
        assert_eq!(render(r"\frac{a}{b}").as_deref(), Some("(a)/(b)"));
    }

    #[test]
    fn a_greek_letter_is_the_letter() {
        assert_eq!(render(r"\alpha").as_deref(), Some("α"));
        assert_eq!(render(r"\Omega").as_deref(), Some("Ω"));
        assert_eq!(render(r"\pi r^2").as_deref(), Some("π r²"));
    }

    #[test]
    fn operators_and_relations_are_symbols() {
        assert_eq!(render(r"a \times b").as_deref(), Some("a × b"));
        assert_eq!(render(r"x \leq y").as_deref(), Some("x ≤ y"));
        assert_eq!(render(r"A \subseteq B").as_deref(), Some("A ⊆ B"));
        assert_eq!(render(r"\infty").as_deref(), Some("∞"));
    }

    /// An exponent is raised where every character can be, and written plainly where one
    /// cannot: half a raised number reads as a mistake rather than as maths.
    #[test]
    fn an_exponent_is_raised_when_it_can_be() {
        assert_eq!(render(r"x^2").as_deref(), Some("x²"));
        assert_eq!(render(r"x^{10}").as_deref(), Some("x¹⁰"));
        assert_eq!(render(r"e^{-x}").as_deref(), Some("e⁻ˣ"));
        // No raised form for this, so it stays legible instead.
        assert_eq!(render(r"x^{\alpha}").as_deref(), Some("x^(α)"));
    }

    #[test]
    fn a_subscript_is_lowered_when_it_can_be() {
        assert_eq!(render(r"x_1").as_deref(), Some("x₁"));
        assert_eq!(render(r"a_{ij}").as_deref(), Some("aᵢⱼ"));
    }

    /// A fraction has no single character, so it is written the way it would be typed.
    #[test]
    fn a_fraction_is_written_out() {
        assert_eq!(render(r"\frac{a}{b}").as_deref(), Some("(a)/(b)"));
        assert_eq!(render(r"\frac{x^2}{2}").as_deref(), Some("(x²)/(2)"));
    }

    #[test]
    fn a_root_carries_its_degree() {
        assert_eq!(render(r"\sqrt{2}").as_deref(), Some("√(2)"));
        assert_eq!(render(r"\sqrt[3]{x}").as_deref(), Some("³√(x)"));
    }

    /// A named operator is a word, and keeps a space around it so it does not run into
    /// what it applies to.
    #[test]
    fn a_named_operator_stays_a_word() {
        assert_eq!(render(r"\sin x").as_deref(), Some("sin x"));
        assert_eq!(render(r"\log_2 n").as_deref(), Some("log₂ n"));
    }

    #[test]
    fn blackboard_letters_are_the_letters() {
        assert_eq!(render(r"\mathbb{R}").as_deref(), Some("ℝ"));
        assert_eq!(render(r"x \in \mathbb{N}").as_deref(), Some("x ∈ ℕ"));
    }

    #[test]
    fn an_accent_sits_on_what_it_marks() {
        let rendered = render(r"\hat{x}").expect("it renders");
        assert!(rendered.starts_with('x'), "{rendered:?}");
        assert!(rendered.chars().count() > 1, "the accent is there too");
    }

    /// Spacing commands are spacing, not text.
    #[test]
    fn spacing_commands_are_spaces() {
        assert_eq!(render(r"a \quad b").as_deref(), Some("a  b"));
        assert_eq!(render(r"a \, b").as_deref(), Some("a  b"));
    }

    /// A style command shows what it wraps and nothing of itself.
    #[test]
    fn a_style_command_shows_only_its_contents() {
        assert_eq!(render(r"\mathrm{d}x").as_deref(), Some("dx"));
        assert_eq!(render(r"\text{if } x > 0").as_deref(), Some("if x > 0"));
    }

    /// What cannot be shown is left as it was written, so a reader who knows LaTeX can
    /// still read it.
    #[test]
    fn what_cannot_be_shown_is_left_alone() {
        let rendered = render(r"\begin{matrix} a \end{matrix}").expect("it renders something");
        assert!(rendered.contains("matrix"), "{rendered:?}");
    }

    #[test]
    fn nothing_renders_as_nothing() {
        assert_eq!(render(""), None);
        assert_eq!(render("   "), None);
    }

    /// A whole expression of the kind a model actually writes.
    #[test]
    fn a_real_expression_reads() {
        assert_eq!(render(r"E = mc^2").as_deref(), Some("E = mc²"));
        assert_eq!(render(r"\sum_{i=1}^{n} x_i").as_deref(), Some("∑ᵢ₌₁ⁿ xᵢ"));
        assert_eq!(
            render(r"\forall x \in \mathbb{R}, x^2 \geq 0").as_deref(),
            Some("∀ x ∈ ℝ, x² ≥ 0")
        );
    }
}
