//! Explicit per-turn trace evidence. Applications supply actual transcripts and
//! tool inputs/results; the transport must never invent them from audio bytes.
use crate::{protocol, A2aError, Result};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct TraceContext {
    trace_id: String,
    parent_span_id: String,
}
impl TraceContext {
    pub fn from_traceparent(value: &str) -> Result<Self> {
        let fields: Vec<_> = value.split('-').collect();
        if fields.len() != 4
            || fields[0] != "00"
            || !valid_hex(fields[1], 32)
            || !valid_hex(fields[2], 16)
            || fields[3].len() != 2
            || !fields[3]
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(A2aError::Trace);
        }
        Ok(Self {
            trace_id: fields[1].into(),
            parent_span_id: fields[2].into(),
        })
    }
    /// For deployments where Connect did not propagate a traceparent header.
    pub fn root() -> Self {
        Self {
            trace_id: uuid::Uuid::new_v4().simple().to_string(),
            parent_span_id: String::new(),
        }
    }
    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }
}
fn valid_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        && value.bytes().any(|b| b != b'0')
}

/// Every tool span requires both sides and valid timestamps at construction.
pub struct ToolTrace {
    name: String,
    input: String,
    output: String,
    start: u64,
    end: u64,
}
impl ToolTrace {
    pub fn new(
        name: impl Into<String>,
        input: impl Into<String>,
        output: impl Into<String>,
        start_unix_nano: u64,
        end_unix_nano: u64,
    ) -> Result<Self> {
        let trace = Self {
            name: name.into(),
            input: input.into(),
            output: output.into(),
            start: start_unix_nano,
            end: end_unix_nano,
        };
        if trace.name.is_empty() || trace.start == 0 || trace.end < trace.start {
            return Err(A2aError::Trace);
        }
        Ok(trace)
    }
}
/// A complete turn, including each tool invocation. No Debug implementation:
/// this structure deliberately carries private conversation content.
pub struct TurnTrace {
    agent: String,
    input: String,
    output: String,
    start: u64,
    end: u64,
    tools: Vec<ToolTrace>,
}
impl TurnTrace {
    pub fn new(
        agent: impl Into<String>,
        input: impl Into<String>,
        output: impl Into<String>,
        start_unix_nano: u64,
        end_unix_nano: u64,
    ) -> Result<Self> {
        let trace = Self {
            agent: agent.into(),
            input: input.into(),
            output: output.into(),
            start: start_unix_nano,
            end: end_unix_nano,
            tools: Vec::new(),
        };
        if trace.agent.is_empty() || trace.start == 0 || trace.end < trace.start {
            return Err(A2aError::Trace);
        }
        Ok(trace)
    }
    pub fn with_tool(mut self, tool: ToolTrace) -> Result<Self> {
        if tool.start < self.start || tool.end > self.end {
            return Err(A2aError::Trace);
        }
        self.tools.push(tool);
        Ok(self)
    }
    pub(crate) fn otlp(self, context: &TraceContext) -> Value {
        let root_id = span_id();
        let mut spans = vec![span(
            context,
            &root_id,
            &context.parent_span_id,
            format!("invoke_agent {}", self.agent),
            2,
            self.start,
            self.end,
            &self.input,
            &self.output,
            "user",
            "assistant",
        )];
        for tool in self.tools {
            spans.push(span(
                context,
                &span_id(),
                &root_id,
                format!("execute_tool {}", tool.name),
                3,
                tool.start,
                tool.end,
                &tool.input,
                &tool.output,
                "assistant",
                "tool",
            ));
        }
        json!({"resourceSpans":[{"resource":{"attributes":[attr("service.name",self.agent)]},"scopeSpans":[{"scope":{"name":"rvoip-amazon-a2a","version":env!("CARGO_PKG_VERSION")},"spans":spans}]}]})
    }
}
fn span_id() -> String {
    protocol::id().replace('-', "")[..16].into()
}
fn attr(key: &str, value: impl Into<String>) -> Value {
    json!({"key":key,"value":{"stringValue":value.into()}})
}
fn event(role: &str, participant: &str, content: &str, time: u64) -> Value {
    json!({"name":"aws.connect.span.message","timeUnixNano":time.to_string(),"attributes":[attr("aws.connect.span.message.role",role),attr("aws.connect.span.message.type","TEXT_MESSAGE"),attr("aws.connect.span.message.participant",participant),attr("aws.connect.span.message.content",content)]})
}
#[allow(clippy::too_many_arguments)]
fn span(
    context: &TraceContext,
    id: &str,
    parent: &str,
    name: String,
    kind: u8,
    start: u64,
    end: u64,
    input: &str,
    output: &str,
    input_participant: &str,
    output_participant: &str,
) -> Value {
    let mut span = json!({"traceId":context.trace_id,"spanId":id,"name":name,"kind":kind,"startTimeUnixNano":start.to_string(),"endTimeUnixNano":end.to_string(),"events":[event("input",input_participant,input,start),event("output",output_participant,output,end)],"status":{"code":1}});
    if !parent.is_empty() {
        span["parentSpanId"] = json!(parent);
    }
    span
}
