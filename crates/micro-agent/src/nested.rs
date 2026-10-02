//! Calls a tool makes to other tools while it runs, such as those of a `codemode` script. Each
//! one goes through the same checks as a call the model made, is reported with the id of the call
//! that made it, and is recorded on that call's result.

use crate::Agent;
use crate::Fan;
use crate::ToolDecision;
use async_trait::async_trait;
use micro_tools::CallableTool;
use micro_tools::NestedOutcome;
use micro_tools::Tool;
use micro_tools::ToolCaller;
use micro_tools::ToolContext;
use micro_tools::ToolOutput;
use micro_types::AgentEvent;
use micro_types::ContentBlock;
use micro_types::EventSource;
use micro_types::LedgerEvent;
use micro_types::NestedCallStatus;
use micro_types::NestedToolCall;
use micro_types::NestedToolCalls;
use micro_types::ToolExecutionMode;
use micro_types::Usage;
use serde_json::Value;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

/// How many nested calls one model-issued call records; later ones run but are not recorded.
const MAX_CALLS: usize = 256;
/// The largest arguments recorded for one nested call, in bytes of JSON.
const MAX_ARGUMENT_BYTES_PER_CALL: usize = 8 * 1024;
/// The most argument bytes recorded across every nested call of one model-issued call.
const MAX_ARGUMENT_BYTES_TOTAL: usize = 32 * 1024;
/// Characters of a failed call's error kept in the record.
const MAX_ERROR_CHARS: usize = 500;
/// How long a call waits for tools still arriving before it settles for what is there.
const ARRIVAL_WAIT: Duration = Duration::from_secs(30);

/// The nested calls of one model-issued call, kept within bounds.
#[derive(Default)]
pub(crate) struct Recorder {
    calls: Vec<NestedToolCall>,
    started: Vec<Instant>,
    complete: bool,
    argument_bytes: usize,
    usage: Option<Usage>,
    /// What the nested calls said they spent, in US dollars.
    cost: Option<f64>,
}

impl Recorder {
    pub(crate) fn new() -> Self {
        Recorder {
            complete: true,
            ..Recorder::default()
        }
    }

    /// Record a call as it starts. Says where it was recorded, unless it was left out.
    fn start(&mut self, id: &str, name: &str, arguments: &Value) -> Option<usize> {
        if self.calls.len() >= MAX_CALLS {
            self.complete = false;
            return None;
        }
        let bytes = arguments.to_string().len();
        let keep = bytes <= MAX_ARGUMENT_BYTES_PER_CALL
            && self.argument_bytes + bytes <= MAX_ARGUMENT_BYTES_TOTAL;
        if keep {
            self.argument_bytes += bytes;
        } else {
            self.complete = false;
        }
        self.calls.push(NestedToolCall {
            id: id.to_string(),
            name: name.to_string(),
            status: NestedCallStatus::Unfinished,
            arguments: keep.then(|| arguments.clone()),
            arguments_bytes: (!keep).then_some(bytes),
            duration_ms: None,
            error: None,
        });
        self.started.push(Instant::now());
        Some(self.calls.len() - 1)
    }

    fn finish(&mut self, index: Option<usize>, output: &ToolOutput) {
        if let Some(usage) = output.usage {
            self.usage = Some(self.usage.map_or(usage, |sum| sum.plus(usage)));
        }
        if let Some(cost) = output.cost {
            self.cost = Some(self.cost.unwrap_or_default() + cost);
        }
        let Some(index) = index else {
            return;
        };
        let started = self.started[index];
        let call = &mut self.calls[index];
        call.duration_ms = Some(started.elapsed().as_millis() as u64);
        call.status = match output.is_error {
            true => NestedCallStatus::Error,
            false => NestedCallStatus::Ok,
        };
        if output.is_error {
            let text = output.text_content();
            if !text.is_empty() {
                call.error = Some(text.chars().take(MAX_ERROR_CHARS).collect());
            }
        }
    }

    /// What the nested calls spent in US dollars, taken once.
    pub(crate) fn take_cost(&mut self) -> Option<f64> {
        self.cost.take()
    }

    /// The record for the calling tool's result, and what the nested calls spent. The record is
    /// absent when no nested call was made.
    pub(crate) fn take(&mut self) -> (Option<NestedToolCalls>, Option<Usage>) {
        let usage = self.usage.take();
        if self.calls.is_empty() && self.complete {
            return (None, usage);
        }
        let calls = std::mem::take(&mut self.calls);
        let complete = self.complete
            && calls
                .iter()
                .all(|call| call.status != NestedCallStatus::Unfinished);
        (Some(NestedToolCalls { calls, complete }), usage)
    }
}

/// What a running tool is handed to call other tools, scoped to the call that is running.
pub(crate) struct NestedScope<'a> {
    agent: &'a Agent,
    events: &'a Fan<'a>,
    /// The call the nested calls are made on behalf of.
    parent_id: String,
    recorder: Arc<Mutex<Recorder>>,
    next_id: AtomicUsize,
    /// Whether this call already holds the queue sequential calls wait in, so its own nested
    /// calls do not wait behind it.
    holds_queue: bool,
}

impl<'a> NestedScope<'a> {
    pub(crate) fn new(
        agent: &'a Agent,
        events: &'a Fan<'a>,
        parent_id: &str,
        recorder: Arc<Mutex<Recorder>>,
    ) -> Self {
        NestedScope {
            agent,
            events,
            parent_id: parent_id.to_string(),
            recorder,
            next_id: AtomicUsize::new(1),
            holds_queue: false,
        }
    }

    fn recorder(&self) -> std::sync::MutexGuard<'_, Recorder> {
        self.recorder
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Run the call once the hooks have let it through.
    async fn run(
        &self,
        tool: Arc<dyn Tool>,
        id: &str,
        name: &str,
        arguments: &Value,
    ) -> ToolOutput {
        let exclusive =
            !self.holds_queue && tool.execution_mode() == Some(ToolExecutionMode::Sequential);
        let _queue = match exclusive {
            true => Some(self.agent.nested_queue.lock().await),
            false => None,
        };

        let (reporting, mut reported) = tokio::sync::mpsc::unbounded_channel::<String>();
        let forwarding = {
            let events = self.events.clone_for_updates();
            let parent_id = self.parent_id.clone();
            let id = id.to_string();
            let name = name.to_string();
            tokio::spawn(async move {
                while let Some(output) = reported.recv().await {
                    events.send(AgentEvent::NestedToolUpdate {
                        parent_id: parent_id.clone(),
                        id: id.clone(),
                        name: name.clone(),
                        output,
                    });
                }
            })
        };

        let below = NestedScope {
            agent: self.agent,
            events: self.events,
            parent_id: id.to_string(),
            recorder: Arc::clone(&self.recorder),
            next_id: AtomicUsize::new(1),
            holds_queue: self.holds_queue || exclusive,
        };
        let context =
            ToolContext::new(id, micro_tools::Progress::new(reporting)).with_tools(&below);
        let output = tool.call(arguments, &context).await;
        drop(context);
        let _ = forwarding.await;
        output
    }
}

#[async_trait]
impl ToolCaller for NestedScope<'_> {
    fn callable(&self) -> Vec<CallableTool> {
        self.agent
            .callable_tools()
            .iter()
            .map(|tool| CallableTool::of(tool.as_ref()))
            .collect()
    }

    async fn arrived(&self) {
        if let Some(arrivals) = &self.agent.arrivals {
            arrivals.settled(ARRIVAL_WAIT).await;
        }
    }

    async fn call(&self, name: &str, arguments: Value) -> NestedOutcome {
        let id = format!(
            "{}/{}",
            self.parent_id,
            self.next_id.fetch_add(1, Ordering::Relaxed)
        );
        let recorded = self.recorder().start(&id, name, &arguments);
        self.events.send(AgentEvent::NestedToolStart {
            parent_id: self.parent_id.clone(),
            id: id.clone(),
            name: name.to_string(),
            arguments: arguments.clone(),
        });

        let output = match self.agent.decide(&id, name, &arguments).await {
            ToolDecision::Refuse(reason) => {
                self.agent.record_event(LedgerEvent::ToolDenied {
                    tool: name.to_string(),
                    reason: reason.clone(),
                    source: EventSource::Extension(String::new()),
                });
                ToolOutput::error(reason)
            }
            decision => {
                let arguments = match decision {
                    ToolDecision::Rewrite(replacement) => replacement,
                    _ => arguments,
                };
                match self.agent.find_callable(name) {
                    Some(tool) => self.run(tool, &id, name, &arguments).await,
                    None => ToolOutput::error(format!(
                        "tool not found: {name}. Only tools listed as callable can be called \
                         from another tool."
                    )),
                }
            }
        };

        let said = output.text_content();
        let (rewritten, is_error) = self
            .agent
            .rewritten(&id, name, said.clone(), output.is_error)
            .await;
        let output = match rewritten == said {
            true => ToolOutput { is_error, ..output },
            false => ToolOutput {
                content: vec![ContentBlock::text(rewritten.clone())],
                structured: None,
                is_error,
                usage: output.usage,
                cost: output.cost,
            },
        };

        self.recorder().finish(recorded, &output);
        self.events.send(AgentEvent::NestedToolEnd {
            parent_id: self.parent_id.clone(),
            id: id.clone(),
            name: name.to_string(),
            output: rewritten,
            is_error,
        });
        NestedOutcome { id, output }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_recorder_with_nothing_in_it_records_nothing() {
        assert_eq!(Recorder::new().take(), (None, None));
    }

    #[test]
    fn calls_are_recorded_with_how_they_ended() {
        let mut recorder = Recorder::new();
        let first = recorder.start("a/1", "read", &json!({ "path": "x" }));
        let second = recorder.start("a/2", "bash", &json!({ "command": "false" }));
        recorder.finish(first, &ToolOutput::text("contents"));
        recorder.finish(
            second,
            &ToolOutput {
                usage: Some(Usage {
                    input: 3,
                    ..Usage::default()
                }),
                ..ToolOutput::error("exit code 1")
            },
        );

        let (record, usage) = recorder.take();
        let record = record.unwrap();
        assert!(record.complete);
        assert_eq!(record.calls[0].status, NestedCallStatus::Ok);
        assert_eq!(record.calls[0].arguments, Some(json!({ "path": "x" })));
        assert_eq!(record.calls[1].status, NestedCallStatus::Error);
        assert_eq!(record.calls[1].error.as_deref(), Some("exit code 1"));
        assert_eq!(usage.unwrap().input, 3);
    }

    #[test]
    fn a_call_still_running_leaves_the_record_incomplete() {
        let mut recorder = Recorder::new();
        recorder.start("a/1", "read", &json!({}));
        let (record, _) = recorder.take();
        let record = record.unwrap();
        assert!(!record.complete);
        assert_eq!(record.calls[0].status, NestedCallStatus::Unfinished);
    }

    #[test]
    fn large_arguments_are_counted_instead_of_kept() {
        let mut recorder = Recorder::new();
        let big = json!({ "text": "x".repeat(MAX_ARGUMENT_BYTES_PER_CALL) });
        let index = recorder.start("a/1", "write", &big);
        recorder.finish(index, &ToolOutput::text("ok"));
        let record = recorder.take().0.unwrap();
        assert!(!record.complete);
        assert_eq!(record.calls[0].arguments, None);
        assert_eq!(record.calls[0].arguments_bytes, Some(big.to_string().len()));
    }

    #[test]
    fn calls_past_the_limit_are_left_out() {
        let mut recorder = Recorder::new();
        for n in 0..=MAX_CALLS {
            let index = recorder.start(&format!("a/{n}"), "ls", &json!({}));
            recorder.finish(index, &ToolOutput::text("ok"));
        }
        let record = recorder.take().0.unwrap();
        assert_eq!(record.calls.len(), MAX_CALLS);
        assert!(!record.complete);
    }
}
