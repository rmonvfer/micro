//! What a session cost, and what the money went on.

use crate::session::thousands;
use crate::CommandContext;
use crate::CommandOutcome;
use crate::InspectionItem;
use micro_models::Catalog;
use micro_models::ModelCost;
use micro_models::RequestCost;
use micro_models::TokenUsage;
use micro_session::SessionStore;
use micro_types::ContentBlock;
use micro_types::EventSource;
use micro_types::LedgerEvent;
use micro_types::Message;
use micro_types::ToolDefinition;
use micro_types::Usage;
use std::collections::HashMap;


#[derive(Debug, Clone, PartialEq)]
pub struct Bill {
    pub session_id: String,
    /// Every turn that the provider reported a price for, in the order they were issued.
    pub turns: Vec<TurnBill>,
    /// What summarizing the conversation cost, each time it was summarized.
    pub compactions: Vec<CompactionBill>,
    /// Models the catalog carries no price for, so their turns are billed at nothing.
    pub unpriced: Vec<String>,
    
    pub unmetered: Vec<String>,
    /// Every turn and every compaction, added up.
    pub total: f64,
    /// Spend on turns whose recorded message path is an ancestor of the current head.
    pub current_branch_total: f64,
    /// Attempts that returned no usage and therefore have unknown cost.
    pub unknown_attempts: Vec<UnknownAttempt>,
    /// Whether this was read from the ledger.
    pub from_ledger: bool,
}

/// One turn, priced, and where its money went.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnBill {
    pub turn: u64,
    pub provider: String,
    pub model: String,
    pub usage: Usage,
    /// What the provider billed, priced against the model's rates.
    pub cost: RequestCost,
    /// Whether this turn is on the branch at the session's current head.
    pub on_current_branch: bool,
    /// What each source contributed to that, adding up to [`RequestCost::total`].
    pub lines: Vec<BillLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownAttempt {
    pub turn: u64,
    pub attempt: u32,
    pub error: String,
}

impl TurnBill {
    /// Whether this turn recorded enough to say where its money went.
    pub fn itemized(&self) -> bool {
        !self.lines.is_empty()
    }

    /// What this turn cost in total.
    pub fn total(&self) -> f64 {
        self.cost.total()
    }
}

/// One source's share of one turn.
#[derive(Debug, Clone, PartialEq)]
pub struct BillLine {
    pub source: EventSource,
    pub side: Side,
    
    pub bytes: u64,
    pub amount: f64,
}

/// Which half of a request a line was billed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// What the model read: the prompt, the tool definitions, the conversation.
    Prompt,
    /// What the model wrote.
    Answer,
}

/// What one summary cost.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionBill {
    /// The turn the conversation had reached when it was summarized.
    pub after_turn: u64,
    pub provider: String,
    pub model: String,
    pub usage: Usage,
    pub cost: RequestCost,
    pub on_current_branch: bool,
}

/// What a session cost, read out of its ledger and priced against the catalog.
pub async fn bill(
    store: &SessionStore,
    catalog: &Catalog,
    session_id: &str,
) -> Result<Bill, String> {
    let loaded = store
        .load(session_id)
        .await
        .map_err(|error| format!("cannot read session {session_id}: {error}"))?;
    let session = &loaded.session;
    let current_path = session.tree().path_entry_ids();

    
    let entries: HashMap<&str, &Message> = session
        .tree()
        .entries()
        .iter()
        .map(|entry| (entry.id.as_str(), &entry.message))
        .collect();

    let mut requests: Vec<(u64, Requested)> = Vec::new();
    let mut usages: Vec<(u64, Usage, String, String)> = Vec::new();
    let mut compactions: Vec<CompactionBill> = Vec::new();
    let mut unknown_attempts = Vec::new();
    let mut reached = 0;
    for recorded in session.events() {
        match &recorded.event {
            LedgerEvent::TurnRequest {
                turn,
                tools_blob,
                prefix_spans,
                message_entry_ids,
                ..
            } => {
                reached = reached.max(*turn);
                
                let described = Requested {
                    tools_blob: tools_blob.clone(),
                    prefix_spans: prefix_spans
                        .iter()
                        .map(|span| (span.source.clone(), span.bytes))
                        .collect(),
                    message_entry_ids: message_entry_ids.clone(),
                };
                match requests.iter_mut().find(|(at, _)| at == turn) {
                    Some(found) => found.1 = described,
                    None => requests.push((*turn, described)),
                }
            }
            LedgerEvent::TurnUsage {
                turn,
                usage,
                provider,
                model,
                ..
            } => {
                reached = reached.max(*turn);
                usages.push((*turn, *usage, provider.clone(), model.clone()));
            }
            LedgerEvent::Compaction {
                cost,
                message_entry_ids,
                ..
            } if cost.usage.total_tokens() > 0 => {
                compactions.push(CompactionBill {
                    after_turn: reached,
                    provider: cost.provider.clone(),
                    model: cost.model.clone(),
                    usage: cost.usage,
                    cost: RequestCost::default(),
                    on_current_branch: message_entry_ids.len() <= current_path.len()
                        && current_path[..message_entry_ids.len()] == message_entry_ids[..],
                });
            }
            LedgerEvent::RequestAttemptFailed {
                turn,
                attempt,
                error,
                usage_unknown: true,
            } => unknown_attempts.push(UnknownAttempt {
                turn: *turn,
                attempt: *attempt,
                error: error.clone(),
            }),
            _ => {}
        }
    }

    let from_ledger = !requests.is_empty();
    let mut noted = Noted::default();
    let mut tools: HashMap<String, Vec<(EventSource, u64)>> = HashMap::new();
    let mut turns: Vec<TurnBill> = Vec::new();

    for (turn, usage, provider, model) in usages {
        let priced = rates(catalog, &provider, &model, &mut noted);
        let cost = priced.price(counted(usage));
        let lines = match requests.iter().find(|(at, _)| *at == turn) {
            Some((_, requested)) => {
                let defined = match tools.get(&requested.tools_blob) {
                    Some(known) => known.clone(),
                    None => {
                        let read = tool_weights(store, session_id, &requested.tools_blob).await;
                        tools.insert(requested.tools_blob.clone(), read.clone());
                        read
                    }
                };
                itemize(&cost, requested, &defined, &entries)
            }
            None => Vec::new(),
        };
        let on_current_branch = match requests.iter().find(|(at, _)| *at == turn) {
            Some((_, requested)) => {
                requested.message_entry_ids.len() <= current_path.len()
                    && current_path[..requested.message_entry_ids.len()]
                        == requested.message_entry_ids[..]
            }
            None => true,
        };
        turns.push(TurnBill {
            turn,
            provider,
            model,
            usage,
            cost,
            on_current_branch,
            lines,
        });
    }

    
    if !from_ledger && turns.is_empty() {
        for (index, entry) in session.tree().entries().iter().enumerate() {
            let Message::Assistant(assistant) = &entry.message else {
                continue;
            };
            if assistant.usage.total_tokens() == 0 {
                continue;
            }
            let priced = rates(catalog, &assistant.provider, &assistant.model, &mut noted);
            turns.push(TurnBill {
                turn: index as u64 + 1,
                provider: assistant.provider.clone(),
                model: assistant.model.clone(),
                usage: assistant.usage,
                cost: priced.price(counted(assistant.usage)),
                on_current_branch: true,
                lines: Vec::new(),
            });
        }
    }

    for summarized in &mut compactions {
        let priced = rates(catalog, &summarized.provider, &summarized.model, &mut noted);
        summarized.cost = priced.price(counted(summarized.usage));
    }

    let total = turns.iter().map(TurnBill::total).sum::<f64>()
        + compactions
            .iter()
            .filter(|summarized| summarized.on_current_branch)
            .map(|summarized| summarized.cost.total())
            .sum::<f64>();
    let current_branch_total = turns
        .iter()
        .filter(|turn| turn.on_current_branch)
        .map(TurnBill::total)
        .sum::<f64>()
        + compactions
            .iter()
            .map(|summarized| summarized.cost.total())
            .sum::<f64>();

    Ok(Bill {
        session_id: session_id.to_string(),
        unpriced: noted.unpriced,
        unmetered: noted.unmetered,
        turns,
        compactions,
        total,
        current_branch_total,
        unknown_attempts,
        from_ledger,
    })
}

/// What one turn's request was built from, as far as pricing it cares.
struct Requested {
    tools_blob: String,
    prefix_spans: Vec<(EventSource, u64)>,
    message_entry_ids: Vec<String>,
}

/// The models a bill has something to say about beyond what they charged.
#[derive(Default)]
struct Noted {
    unpriced: Vec<String>,
    unmetered: Vec<String>,
}

impl Noted {
    fn name(list: &mut Vec<String>, provider: &str, model: &str) {
        let named = format!("{provider}/{model}");
        if !list.contains(&named) {
            list.push(named);
        }
    }
}

/// The rates a turn was billed at, noting a model that charged nothing and why.
fn rates<'a>(
    catalog: &'a Catalog,
    provider: &str,
    model: &str,
    noted: &mut Noted,
) -> &'a ModelCost {
    /// Stands in for a model the catalog does not carry, which bills every turn at nothing.
    static UNKNOWN: ModelCost = ModelCost {
        input: 0.0,
        output: 0.0,
        cache_read: 0.0,
        cache_write: 0.0,
        tiers: Vec::new(),
    };

    match catalog.get(provider, model) {
        Some(found) => {
            if found.cost.is_free() {
                Noted::name(&mut noted.unmetered, provider, model);
            }
            &found.cost
        }
        None => {
            Noted::name(&mut noted.unpriced, provider, model);
            &UNKNOWN
        }
    }
}

/// Tokens as the pricing tables count them.
fn counted(usage: Usage) -> TokenUsage {
    TokenUsage::new(usage.input as u64, usage.output as u64)
        .with_cache(usage.cache_read as u64, usage.cache_write as u64)
}

/// What each tool's definition contributed to a prompt, read out of the blob the turn named them
/// by.
async fn tool_weights(
    store: &SessionStore,
    session_id: &str,
    blob: &str,
) -> Vec<(EventSource, u64)> {
    let Ok(raw) = store.blob(session_id, blob).await else {
        return Vec::new();
    };
    let Ok(defined) = serde_json::from_slice::<Vec<ToolDefinition>>(&raw) else {
        return Vec::new();
    };
    defined
        .iter()
        .map(|tool| {
            let bytes = serde_json::to_vec(tool).map(|body| body.len()).unwrap_or(0);
            (EventSource::Tool(tool.name.clone()), bytes as u64)
        })
        .collect()
}

/// Share one turn's price out between everything that put bytes into it.
fn itemize(
    cost: &RequestCost,
    requested: &Requested,
    tools: &[(EventSource, u64)],
    entries: &HashMap<&str, &Message>,
) -> Vec<BillLine> {
    let mut weights: Vec<(EventSource, u64)> = Vec::new();
    let mut weigh = |source: EventSource, bytes: u64| match weights
        .iter_mut()
        .find(|(known, _)| *known == source)
    {
        Some(found) => found.1 += bytes,
        None => weights.push((source, bytes)),
    };

    for (source, bytes) in &requested.prefix_spans {
        weigh(source.clone(), *bytes);
    }
    for (source, bytes) in tools {
        weigh(source.clone(), *bytes);
    }
    for id in &requested.message_entry_ids {
        if let Some(message) = entries.get(id.as_str()) {
            weigh(spoken_by(message), message_bytes(message));
        }
    }

    let carrying: u64 = weights.iter().map(|(_, bytes)| bytes).sum();
    if carrying == 0 {
        return Vec::new();
    }

    let prompt = cost.input + cost.cache_read + cost.cache_write;
    let mut lines: Vec<BillLine> = weights
        .into_iter()
        .map(|(source, bytes)| BillLine {
            source,
            side: Side::Prompt,
            bytes,
            amount: prompt * (bytes as f64 / carrying as f64),
        })
        .collect();

    
    let shared: f64 = lines.iter().map(|line| line.amount).sum();
    if let Some(largest) = lines
        .iter_mut()
        .max_by(|left, right| left.amount.total_cmp(&right.amount))
    {
        largest.amount += prompt - shared;
    }

    lines.push(BillLine {
        source: EventSource::Model,
        side: Side::Answer,
        bytes: 0,
        amount: cost.output,
    });
    lines
}


fn spoken_by(message: &Message) -> EventSource {
    match message {
        Message::User { .. } => EventSource::User,
        Message::Assistant(_) => EventSource::Model,
        Message::ToolResult { tool_name, .. } => EventSource::Tool(tool_name.clone()),
    }
}

/// How many bytes of a message the model actually read.
fn message_bytes(message: &Message) -> u64 {
    let content = match message {
        Message::User { content, .. } => content,
        Message::Assistant(assistant) => &assistant.content,
        Message::ToolResult { content, .. } => content,
    };
    let named = match message {
        Message::ToolResult { tool_name, .. } => tool_name.len(),
        _ => 0,
    };
    let carried: usize = content
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text } => text.len(),
            ContentBlock::Thinking { thinking, .. } => thinking.len(),
            ContentBlock::RedactedThinking { data } => data.len(),
            ContentBlock::Image { data, .. } => data.len(),
            ContentBlock::ToolCall {
                name,
                arguments,
                signature,
                ..
            } => {
                name.len() + arguments.to_string().len() + signature.as_ref().map_or(0, String::len)
            }
        })
        .sum();
    (named + carried) as u64
}

impl Bill {
    /// The fixed part of the interactive bill.
    pub fn summary(&self) -> String {
        let mut cost = RequestCost::default();
        for part in self
            .turns
            .iter()
            .map(|turn| turn.cost)
            .chain(self.compactions.iter().map(|compaction| compaction.cost))
        {
            cost += part;
        }
        let mut out = format!(
            "Session {}\n\n{:<28}{}\n",
            self.session_id,
            "Total spent",
            money(self.total),
        );
        if (self.total - self.current_branch_total).abs() > f64::EPSILON {
            out.push_str(&format!(
                "{:<28}{}\n{:<28}{}\n",
                "Current branch",
                money(self.current_branch_total),
                "Other branches",
                money(self.total - self.current_branch_total),
            ));
        }
        out.push_str(&format!(
            "{:<28}{}\n{:<28}{}\n{:<28}{}\n{:<28}{}\n",
            "Input",
            money(cost.input),
            "Cache reads",
            money(cost.cache_read),
            "Cache writes",
            money(cost.cache_write),
            "Output",
            money(cost.output),
        ));
        out
    }

    pub fn inspection_items(&self) -> Vec<InspectionItem> {
        self.turns
            .iter()
            .map(|turn| {
                let prompt = turn.usage.input + turn.usage.cache_read + turn.usage.cache_write;
                let cache = if prompt == 0 {
                    0.0
                } else {
                    (turn.usage.cache_read as f64 / prompt as f64) * 100.0
                };
                InspectionItem {
                    label: format!(
                        "turn {:<4}  {:<28}  {}  cache {:.0}%",
                        turn.turn,
                        format!("{}/{}", turn.provider, turn.model),
                        money(turn.total()),
                        cache,
                    ),
                    detail: self.added_by(turn.turn).unwrap_or_else(|error| error),
                }
            })
            .collect()
    }

    /// The whole bill, as it should be read.
    pub fn report(&self) -> String {
        let mut out = format!("Bill for session {}\n", self.session_id);

        if self.turns.is_empty() && self.unknown_attempts.is_empty() {
            out.push_str("\nNothing was billed: no turn of this session reported any usage.\n");
            return out;
        }

        
        let mut summaries = self.compactions.iter().peekable();
        for turn in &self.turns {
            while let Some(summarized) =
                summaries.next_if(|summarized| summarized.after_turn < turn.turn)
            {
                out.push_str(&summary_row(summarized));
            }
            out.push('\n');
            out.push_str(&self.turn_header(turn));
            for line in &turn.lines {
                out.push_str(&item(line));
            }
        }
        for summarized in summaries {
            out.push_str(&summary_row(summarized));
        }

        out.push_str(&format!("\n{:<44}{}\n", "Total session cost", money(self.total)));
        if (self.total - self.current_branch_total).abs() > f64::EPSILON {
            out.push_str(&format!(
                "{:<44}{}\n{:<44}{}\n",
                "Current branch",
                money(self.current_branch_total),
                "Other branches",
                money(self.total - self.current_branch_total),
            ));
        }
        for attempt in &self.unknown_attempts {
            out.push_str(&format!(
                "turn {} attempt {}: usage unknown ({})\n",
                attempt.turn, attempt.attempt, attempt.error
            ));
        }
        out.push_str(&format!("\n{}\n", self.tokens()));

        
        if !self.unmetered.is_empty() {
            out.push_str(&format!(
                "\n{} uses subscription billing; this report excludes the plan charge.\n",
                self.unmetered.join(", ")
            ));
        }
        if !self.unpriced.is_empty() {
            out.push_str(&format!(
                "\nNo catalog price for {}.\n",
                self.unpriced.join(", ")
            ));
        }
        out.push_str(&format!("\n{}\n", self.caveat()));
        out
    }

    /// What one turn added to the bill, and why it cost what it did.
    pub fn added_by(&self, turn: u64) -> Result<String, String> {
        let Some(position) = self.turns.iter().position(|billed| billed.turn == turn) else {
            return Err(format!(
                "session {} has no billed turn {turn}",
                self.session_id
            ));
        };
        let billed = &self.turns[position];
        let before: f64 = self.turns[..position]
            .iter()
            .map(TurnBill::total)
            .sum::<f64>()
            + self
                .compactions
                .iter()
                .filter(|summarized| summarized.after_turn < turn)
                .map(|summarized| summarized.cost.total())
                .sum::<f64>();

        
        let mut out = format!(
            "Turn {turn} of session {}  {}/{}\n\n{:<44}{}\n",
            self.session_id,
            billed.provider,
            billed.model,
            "What it added",
            money(billed.total()),
        );
        if !billed.itemized() {
            out.push_str("  (recorded before the ledger existed: no split)\n");
        }
        for line in &billed.lines {
            out.push_str(&item(line));
        }
        out.push_str(&format!(
            "\n{:<44}{}\n{:<44}{}\n",
            "running total before",
            money(before),
            "running total after",
            money(before + billed.total()),
        ));

        out.push_str("\nWhy it cost that\n");
        for reason in self.reasons(position) {
            out.push_str(&format!("  - {reason}\n"));
        }
        Ok(out)
    }

    /// A turn's own line: which model answered it, and what it came to.
    fn turn_header(&self, turn: &TurnBill) -> String {
        let header = format!(
            "{:<32}{:>10}  {}\n",
            format!("turn {}", turn.turn),
            format!("{}/{}", turn.provider, turn.model),
            money(turn.total()),
        );
        match turn.itemized() {
            true => header,
            false => format!("{header}  (recorded before the ledger existed: no split)\n"),
        }
    }

    /// Everything that explains what one turn added, against the turn before it.
    fn reasons(&self, position: usize) -> Vec<String> {
        let billed = &self.turns[position];
        let mut reasons = Vec::new();

        let Some(previous) = position.checked_sub(1).map(|before| &self.turns[before]) else {
            reasons.push(
                "this is the session's first billed turn, so everything it sent was new"
                    .to_string(),
            );
            return reasons;
        };

        for summarized in self.compactions.iter().filter(|summarized| {
            summarized.after_turn >= previous.turn && summarized.after_turn < billed.turn
        }) {
            reasons.push(format!(
                "the conversation was summarized after turn {}, which cost {} and left the \
                 prompt to be read fresh",
                summarized.after_turn,
                money(summarized.cost.total()),
            ));
        }

        if billed.usage.cache_write > 0 {
            reasons.push(format!(
                "{} tokens were written into the provider's cache, which is charged above \
                 the plain input rate and is what makes the next turn cheap",
                thousands(billed.usage.cache_write as u64),
            ));
        }
        let read = billed.usage.cache_read as u64;
        let sent = read + billed.usage.input as u64 + billed.usage.cache_write as u64;
        if read > 0 && sent > 0 {
            reasons.push(format!(
                "{} of the {} tokens it sent came back out of cache ({:.0}%)",
                thousands(read),
                thousands(sent),
                (read as f64 / sent as f64) * 100.0,
            ));
        }

        if let Some(grown) = grown_most(billed, previous) {
            reasons.push(grown);
        }
        if reasons.is_empty() {
            reasons.push(format!(
                "it sent what turn {} sent, and was billed accordingly",
                previous.turn
            ));
        }
        reasons
    }

    /// The tokens the whole session ran through, which is the other number a reader wants.
    fn tokens(&self) -> String {
        let mut total = Usage::default();
        for usage in self
            .turns
            .iter()
            .map(|turn| turn.usage)
            .chain(self.compactions.iter().map(|summarized| summarized.usage))
        {
            total.input += usage.input;
            total.output += usage.output;
            total.cache_read += usage.cache_read;
            total.cache_write += usage.cache_write;
        }
        format!(
            "{} turns  {} in  {} out  {} read from cache  {} written to it",
            self.turns.len(),
            thousands(total.input as u64),
            thousands(total.output as u64),
            thousands(total.cache_read as u64),
            thousands(total.cache_write as u64),
        )
    }

    
    fn caveat(&self) -> &'static str {
        match self.from_ledger {
            true => {
                "What each turn cost is exact: it is what the provider billed, at the\n\
                 rates the model charges. How a turn is shared out between its sources\n\
                 is an estimate, worked out from the bytes each one put into the prompt\n\
                 — but the shares always add up to the turn."
            }
            false => {
                "This session was recorded before the ledger existed, so it holds what\n\
                 each answer reported and nothing about where the prompt came from. The\n\
                 turn totals are what the provider billed; there is nothing to split\n\
                 them by."
            }
        }
    }
}

/// Which source grew the most since the turn before, when one did.
fn grown_most(billed: &TurnBill, previous: &TurnBill) -> Option<String> {
    let mut worst: Option<(&EventSource, u64)> = None;
    for line in billed.lines.iter().filter(|line| line.side == Side::Prompt) {
        let had = previous
            .lines
            .iter()
            .find(|before| before.source == line.source)
            .map_or(0, |before| before.bytes);
        let grew = line.bytes.saturating_sub(had);
        if grew > 0 && worst.is_none_or(|(_, most)| grew > most) {
            worst = Some((&line.source, grew));
        }
    }

    let (source, grew) = worst?;
    Some(format!(
        "{source} put {} more bytes into the prompt than at turn {}, the largest growth of \
         any source",
        thousands(grew),
        previous.turn,
    ))
}

/// What one summary cost, as a row of its own.
fn summary_row(summarized: &CompactionBill) -> String {
    format!(
        "\n{:<32}{:>10}  {}\n",
        format!("compaction after turn {}", summarized.after_turn),
        format!("{}/{}", summarized.provider, summarized.model),
        money(summarized.cost.total()),
    )
}

/// One share of one turn, as a row.
fn item(line: &BillLine) -> String {
    let (named, measured) = match line.side {
        Side::Prompt => (line.source.as_str(), format!("{} B", thousands(line.bytes))),
        Side::Answer => (format!("{} (output)", line.source), String::new()),
    };
    format!("  {named:<30}{measured:>10}  {}\n", money(line.amount))
}

/// An amount of money, in the unit token prices are actually quoted in.
fn money(amount: f64) -> String {
    format!("${amount:.6}")
}

/// `/bill [turn]`.
pub(crate) async fn command(
    argument: Option<&str>,
    context: &CommandContext<'_>,
) -> CommandOutcome {
    let Some(session_id) = context.session_id else {
        return CommandOutcome::error("nothing to bill: this conversation is not being recorded");
    };

    let turn = match argument.map(str::trim).filter(|turn| !turn.is_empty()) {
        Some(argument) => match argument.parse::<u64>() {
            Ok(turn) => Some(turn),
            Err(_) => return CommandOutcome::error(format!("{argument} is not a turn number")),
        },
        None => None,
    };

    let billed = match bill(context.sessions, context.catalog, session_id).await {
        Ok(billed) => billed,
        Err(reason) => return CommandOutcome::error(reason),
    };
    match turn {
        Some(turn) => match billed.added_by(turn) {
            Ok(report) => CommandOutcome::inspect("Session bill", report),
            Err(reason) => CommandOutcome::error(reason),
        },
        None => CommandOutcome::inspect_items(
            "Session bill",
            billed.summary(),
            billed.inspection_items(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Harness;
    use micro_models::ModelDef;
    use micro_session::Session;
    use micro_types::AssistantMessage;
    use micro_types::CompactionCost;
    use micro_types::PrefixSpan;
    use micro_types::StopReason;

    /// The rates the test model charges, in dollars per million tokens.
    const INPUT: f64 = 3.0;
    const OUTPUT: f64 = 15.0;
    const CACHE_READ: f64 = 0.3;

    /// A catalog holding one model, priced.
    fn catalog() -> Catalog {
        let model: ModelDef = serde_json::from_value(serde_json::json!({
            "id": "test-model",
            "name": "Test Model",
            "provider": "openai",
            "api": "openai-completions",
            "base_url": "http://127.0.0.1/v1",
            "context_window": 200_000,
            "max_output_tokens": 4_096,
            "cost": { "input": INPUT, "output": OUTPUT, "cache_read": CACHE_READ },
        }))
        .expect("a model the catalog can price");
        Catalog::from_models(vec![model])
    }

    fn spent(usage: Usage) -> f64 {
        (usage.input as f64 * INPUT
            + usage.output as f64 * OUTPUT
            + usage.cache_read as f64 * CACHE_READ)
            / 1e6
    }

    fn usage(input: u32, output: u32, cache_read: u32) -> Usage {
        Usage {
            input,
            output,
            cache_read,
            cache_write: 0,
        }
    }

    /// Record one turn: what the request was built from, what came back, and what it cost.
    async fn record_turn(session: &mut Session, turn: u64, reported: Usage) {
        let defined = vec![ToolDefinition {
            name: "read".into(),
            description: "read a file".into(),
            parameters: serde_json::json!({}),
            constrained_sampling: None,
        }];
        let tools_blob = session
            .store_blob(&serde_json::to_vec(&defined).unwrap())
            .await
            .unwrap();

        session.append(&Message::user("hello")).await.unwrap();
        session
            .append_event(LedgerEvent::TurnRequest {
                turn,
                provider: "openai".into(),
                model: "test-model".into(),
                prefix_hash: "aa".into(),
                request_hash: "bb".into(),
                request_body_blob: None,
                system_prompt_blob: None,
                tools_blob,
                model_blob: "cc".into(),
                prefix_spans: vec![
                    PrefixSpan {
                        source: EventSource::SystemPrompt,
                        bytes: 400,
                        hash: "dd".into(),
                    },
                    PrefixSpan {
                        source: EventSource::ProjectInstructions,
                        bytes: 100,
                        hash: "ee".into(),
                    },
                ],
                
                message_entry_ids: Vec::new(),
                attempt: 1,
            })
            .await
            .unwrap();
        session
            .append_event(LedgerEvent::TurnUsage {
                turn,
                usage: reported,
                stop_reason: StopReason::Stop,
                provider: "openai".into(),
                model: "test-model".into(),
            })
            .await
            .unwrap();
        session.append(&answered("done", reported)).await.unwrap();
    }

    fn answered(text: &str, reported: Usage) -> Message {
        Message::Assistant(AssistantMessage {
            content: vec![ContentBlock::text(text)],
            provider: "openai".into(),
            model: "test-model".into(),
            usage: reported,
            stop_reason: StopReason::Stop,
            error: None,
            timestamp: 0,
        })
    }

    
    #[tokio::test]
    async fn a_bill_of_a_recorded_session_adds_up_to_what_was_reported() {
        let harness = Harness::new("bill-adds-up");
        let mut session = harness
            .sessions
            .create(&harness.workspace, "openai/test-model")
            .await
            .unwrap();
        let first = usage(1_000, 200, 0);
        let second = usage(500, 100, 1_000);
        record_turn(&mut session, 1, first).await;
        record_turn(&mut session, 2, second).await;
        let id = session.id().to_string();
        drop(session);

        let billed = bill(&harness.sessions, &catalog(), &id).await.unwrap();

        assert!(billed.from_ledger);
        assert_eq!(billed.turns.len(), 2);
        assert!(billed.unpriced.is_empty(), "{:?}", billed.unpriced);
        assert!((billed.total - (spent(first) + spent(second))).abs() < 1e-12);

        for turn in &billed.turns {
            let shared: f64 = turn.lines.iter().map(|line| line.amount).sum();
            assert!(
                (shared - turn.total()).abs() < 1e-12,
                "turn {} shares add up to {shared}, not {}",
                turn.turn,
                turn.total()
            );
        }
    }

    #[tokio::test]
    async fn abandoned_branch_spend_stays_in_the_total() {
        let harness = Harness::new("bill-branches");
        let mut session = harness
            .sessions
            .create(&harness.workspace, "openai/test-model")
            .await
            .unwrap();
        let common = usage(1_000, 100, 0);
        let abandoned = usage(2_000, 100, 0);
        let current = usage(500, 100, 0);
        record_turn(&mut session, 1, common).await;
        let fork = session.tree().entries().last().unwrap().id.clone();
        record_turn(&mut session, 2, abandoned).await;
        session.branch_from(&fork).await.unwrap();
        record_turn(&mut session, 3, current).await;
        let id = session.id().to_string();
        drop(session);

        let billed = bill(&harness.sessions, &catalog(), &id).await.unwrap();
        assert!((billed.total - (spent(common) + spent(abandoned) + spent(current))).abs() < 1e-12);
        assert!((billed.current_branch_total - (spent(common) + spent(current))).abs() < 1e-12);
        assert!(!billed.turns[1].on_current_branch);
    }

    /// Every source that put bytes into a turn earns a line of its own, including the tool the turn
    /// was merely offered.
    #[tokio::test]
    async fn a_bill_names_every_source_that_contributed() {
        let harness = Harness::new("bill-sources");
        let mut session = harness
            .sessions
            .create(&harness.workspace, "openai/test-model")
            .await
            .unwrap();
        record_turn(&mut session, 1, usage(1_000, 200, 0)).await;
        let id = session.id().to_string();
        drop(session);

        let billed = bill(&harness.sessions, &catalog(), &id).await.unwrap();
        let named = |wanted: EventSource| {
            billed.turns[0]
                .lines
                .iter()
                .any(|line| line.source == wanted)
        };

        assert!(named(EventSource::SystemPrompt));
        assert!(named(EventSource::ProjectInstructions));
        assert!(named(EventSource::Tool("read".into())));
        assert!(named(EventSource::User));
        assert!(billed.turns[0]
            .lines
            .iter()
            .any(|line| line.side == Side::Answer));

        let report = billed.report();
        assert!(report.contains("project_instructions"), "{report}");
        assert!(report.contains("tool:read"), "{report}");
        assert!(report.contains("always add up to the turn"), "{report}");
    }

    
    #[tokio::test]
    async fn a_summary_is_billed_on_a_line_of_its_own() {
        let harness = Harness::new("bill-compaction");
        let mut session = harness
            .sessions
            .create(&harness.workspace, "openai/test-model")
            .await
            .unwrap();
        let turn = usage(1_000, 200, 0);
        record_turn(&mut session, 1, turn).await;
        let summary = usage(900, 120, 0);
        session
            .compacted(
                "what was said so far",
                1,
                CompactionCost {
                    usage: summary,
                    provider: "openai".into(),
                    model: "test-model".into(),
                },
            )
            .await
            .unwrap();
        let id = session.id().to_string();
        drop(session);

        let billed = bill(&harness.sessions, &catalog(), &id).await.unwrap();

        assert_eq!(billed.compactions.len(), 1);
        assert_eq!(billed.compactions[0].after_turn, 1);
        assert!((billed.compactions[0].cost.total() - spent(summary)).abs() < 1e-12);
        assert!((billed.total - (spent(turn) + spent(summary))).abs() < 1e-12);
        assert!(
            billed.report().contains("compaction after turn 1"),
            "{}",
            billed.report()
        );
    }

    /// A session written before the ledger existed still has a bill, read out of the answers
    /// themselves.
    #[tokio::test]
    async fn a_session_recorded_before_the_ledger_is_billed_turn_by_turn() {
        let harness = Harness::new("bill-legacy");
        let mut session = harness
            .sessions
            .create(&harness.workspace, "openai/test-model")
            .await
            .unwrap();
        let first = usage(1_000, 200, 0);
        let second = usage(400, 50, 0);
        session.append(&Message::user("hello")).await.unwrap();
        session.append(&answered("one", first)).await.unwrap();
        session.append(&Message::user("again")).await.unwrap();
        session.append(&answered("two", second)).await.unwrap();
        let id = session.id().to_string();
        drop(session);

        let billed = bill(&harness.sessions, &catalog(), &id).await.unwrap();

        assert!(!billed.from_ledger);
        assert_eq!(billed.turns.len(), 2);
        assert!(billed.turns.iter().all(|turn| !turn.itemized()));
        assert!((billed.total - (spent(first) + spent(second))).abs() < 1e-12);

        let report = billed.report();
        assert!(
            report.contains("recorded before the ledger existed"),
            "{report}"
        );
        assert!(report.contains("there is nothing to split"), "{report}");
    }

    
    #[tokio::test]
    async fn the_diff_of_a_turn_reports_what_it_added() {
        let harness = Harness::new("bill-diff");
        let mut session = harness
            .sessions
            .create(&harness.workspace, "openai/test-model")
            .await
            .unwrap();
        let first = usage(1_000, 200, 0);
        let second = usage(500, 100, 1_000);
        record_turn(&mut session, 1, first).await;
        record_turn(&mut session, 2, second).await;
        let id = session.id().to_string();
        drop(session);

        let billed = bill(&harness.sessions, &catalog(), &id).await.unwrap();
        let report = billed.added_by(2).unwrap();

        assert!(
            report.starts_with(&format!("Turn 2 of session {id}")),
            "{report}"
        );
        
        assert!(report.contains("What it added"), "{report}");
        assert_eq!(
            report.matches("turn 2").count(),
            0,
            "the turn is named once, in the heading: {report}"
        );
        assert!(
            report.contains(&format!("${:.6}", spent(second))),
            "what this turn came to: {report}"
        );
        assert!(
            report.contains(&format!("${:.6}", spent(first))),
            "the running total before it: {report}"
        );
        assert!(
            report.contains(&format!("${:.6}", spent(first) + spent(second))),
            "and after it: {report}"
        );
        assert!(report.contains("came back out of cache"), "{report}");

        assert!(billed.added_by(9).is_err(), "a turn nobody billed");
    }

    /// A session on a plan costs nothing per request, and the bill says so.
    #[tokio::test]
    async fn a_session_billed_against_a_plan_says_so() {
        let harness = Harness::new("bill-plan");
        let mut session = harness
            .sessions
            .create(&harness.workspace, "openai/test-model")
            .await
            .unwrap();
        record_turn(&mut session, 1, usage(1_000, 200, 0)).await;
        let id = session.id().to_string();
        drop(session);

        
        let free: ModelDef = serde_json::from_value(serde_json::json!({
            "id": "test-model",
            "name": "Test Model",
            "provider": "openai",
            "api": "openai-completions",
            "base_url": "http://127.0.0.1/v1",
            "context_window": 200_000,
            "max_output_tokens": 4_096,
        }))
        .expect("a model with no price of its own");

        let billed = bill(&harness.sessions, &Catalog::from_models(vec![free]), &id)
            .await
            .unwrap();

        assert_eq!(billed.total, 0.0);
        assert_eq!(billed.unmetered, vec!["openai/test-model".to_string()]);
        assert!(billed.unpriced.is_empty(), "the model was found");

        let report = billed.report();
        assert!(report.contains("uses subscription billing"), "{report}");
        assert!(
            !report.contains("No catalog price for"),
            "subscription-backed models should not be reported as unpriced: {report}"
        );
    }

    
    #[tokio::test]
    async fn a_model_the_catalog_cannot_price_is_named() {
        let harness = Harness::new("bill-unpriced");
        let mut session = harness
            .sessions
            .create(&harness.workspace, "openai/test-model")
            .await
            .unwrap();
        record_turn(&mut session, 1, usage(1_000, 200, 0)).await;
        let id = session.id().to_string();
        drop(session);

        let billed = bill(&harness.sessions, &Catalog::from_models(Vec::new()), &id)
            .await
            .unwrap();

        assert_eq!(billed.total, 0.0);
        assert_eq!(billed.unpriced, vec!["openai/test-model".to_string()]);
        assert!(
            billed.report().contains("No catalog price for"),
            "{}",
            billed.report()
        );
    }

    fn spans(pairs: &[(EventSource, u64)]) -> Requested {
        Requested {
            tools_blob: String::new(),
            prefix_spans: pairs.to_vec(),
            message_entry_ids: Vec::new(),
        }
    }

    /// The one rule the estimate is held to: however the shares fall out, they add up to what the
    /// provider billed.
    #[test]
    fn the_shares_of_a_turn_add_up_to_what_it_cost() {
        let cost = RequestCost {
            input: 0.031,
            output: 0.017,
            cache_read: 0.0031,
            cache_write: 0.0007,
        };
        let requested = spans(&[
            (EventSource::SystemPrompt, 1_111),
            (EventSource::ProjectInstructions, 907),
            (EventSource::Skill(String::new()), 3),
        ]);

        let lines = itemize(&cost, &requested, &[], &HashMap::new());

        let summed: f64 = lines.iter().map(|line| line.amount).sum();
        assert!(
            (summed - cost.total()).abs() < 1e-12,
            "{summed} is not {}",
            cost.total()
        );
    }

    
    #[test]
    fn a_remainder_that_will_not_divide_lands_on_the_largest_line() {
        let cost = RequestCost {
            input: 1.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        };
        let requested = spans(&[
            (EventSource::SystemPrompt, 1),
            (EventSource::ProjectInstructions, 1),
            (EventSource::Skill(String::new()), 1),
        ]);

        let lines = itemize(&cost, &requested, &[], &HashMap::new());

        let summed: f64 = lines.iter().map(|line| line.amount).sum();
        assert!((summed - 1.0).abs() < 1e-12, "{summed} is not 1.0");
    }

    
    #[test]
    fn the_answer_is_billed_to_the_model_alone() {
        let cost = RequestCost {
            input: 0.5,
            output: 0.25,
            cache_read: 0.0,
            cache_write: 0.0,
        };
        let requested = spans(&[(EventSource::SystemPrompt, 100)]);

        let lines = itemize(&cost, &requested, &[], &HashMap::new());

        let answer: Vec<&BillLine> = lines
            .iter()
            .filter(|line| line.side == Side::Answer)
            .collect();
        assert_eq!(answer.len(), 1);
        assert_eq!(answer[0].source, EventSource::Model);
        assert_eq!(answer[0].amount, 0.25);
    }

    /// A source that supplied twice the bytes carries twice the share, which is the whole of what
    /// the estimate claims.
    #[test]
    fn a_source_is_charged_in_proportion_to_what_it_supplied() {
        let cost = RequestCost {
            input: 0.3,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        };
        let requested = spans(&[
            (EventSource::SystemPrompt, 200),
            (EventSource::ProjectInstructions, 100),
        ]);

        let lines = itemize(&cost, &requested, &[], &HashMap::new());

        let share = |wanted: EventSource| {
            lines
                .iter()
                .find(|line| line.source == wanted)
                .map(|line| line.amount)
                .unwrap_or_default()
        };
        assert!((share(EventSource::SystemPrompt) - 0.2).abs() < 1e-12);
        assert!((share(EventSource::ProjectInstructions) - 0.1).abs() < 1e-12);
    }

    
    #[test]
    fn a_turn_with_nothing_recorded_about_it_has_no_lines() {
        let cost = RequestCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.0,
            cache_write: 0.0,
        };
        assert!(itemize(&cost, &spans(&[]), &[], &HashMap::new()).is_empty());
    }

    
    #[test]
    fn a_message_is_measured_by_what_the_model_reads_of_it() {
        let said = Message::user("hello");
        assert_eq!(message_bytes(&said), 5);

        let answered = Message::Assistant(AssistantMessage {
            content: vec![ContentBlock::text("hi")],
            provider: "openai".into(),
            model: "test-model".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error: None,
            timestamp: 0,
        });
        assert_eq!(message_bytes(&answered), 2);

        let result = Message::tool_result("call_1", "read", "abc", false);
        assert_eq!(message_bytes(&result), "read".len() as u64 + 3);
    }

    /// The two halves of the conversation are told apart by who produced them.
    #[test]
    fn a_tool_result_is_billed_to_the_tool_that_produced_it() {
        let result = Message::tool_result("call_1", "bash", "output", false);
        assert_eq!(spoken_by(&result), EventSource::Tool("bash".into()));
        assert_eq!(spoken_by(&Message::user("hi")), EventSource::User);
    }
}
