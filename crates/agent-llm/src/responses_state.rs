//! Adapter-owned Responses continuity. The kernel only stores opaque data;
//! original output items are replayed only for an unchanged assistant message
//! from the same endpoint/account/model. No process-local transcript cache.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use agent_llm_core::{ChatMessage, ProviderState, Role};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) fn origin(
    base_url: &str,
    api_key: &str,
    headers: &HashMap<String, String>,
    user_agent: Option<&str>,
    auth_header: bool,
) -> String {
    let headers: BTreeMap<_, _> = headers.iter().collect();
    let bytes = serde_json::to_vec(&(
        base_url.trim_end_matches('/'),
        api_key,
        headers,
        user_agent,
        auth_header,
    ))
    .expect("strings and headers are serializable");
    format!("openai_responses:{:x}", Sha256::digest(bytes))
}

pub(crate) struct RequestPolicy {
    pub(crate) temperature: bool,
    pub(crate) reasoning: bool,
}

pub(crate) struct RequestContext {
    pub(crate) exchange: Arc<Exchange>,
    pub(crate) replay: ReplayPlan,
    sampling: serde_json::Map<String, Value>,
    policy: RequestPolicy,
}

impl std::fmt::Debug for RequestContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResponsesRequestContext")
            .field("replay_groups", &self.replay.groups.len())
            .field("sampling_keys", &self.sampling.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl RequestContext {
    pub(crate) fn new(
        origin: String,
        model: &str,
        messages: &[ChatMessage],
        effort: Option<&str>,
        update_effort: Option<&str>,
        sampling: serde_json::Map<String, Value>,
        policy: RequestPolicy,
    ) -> Self {
        Self {
            replay: ReplayPlan::new(messages, &origin, model, update_effort),
            exchange: Arc::new(Exchange::new(origin, model, effort)),
            sampling,
            policy,
        }
    }

    pub(crate) fn patch(&self, body: &mut Value) {
        self.replay.apply(body);
        // Rig's typed AdditionalParameters drops fields it does not model.
        // Apply explicit model/request overrides at the actual wire boundary.
        // Null means omit: unsupported parameters must not be sent as null.
        if let Some(body) = body.as_object_mut() {
            for (key, value) in &self.sampling {
                if value.is_null() {
                    body.remove(key);
                } else {
                    body.insert(key.clone(), value.clone());
                }
            }
            // A model's newer cache options replace the generated legacy TTL.
            if self
                .sampling
                .get("prompt_cache_options")
                .is_some_and(|v| !v.is_null())
            {
                body.remove("prompt_cache_retention");
            }
            if !self.policy.temperature {
                body.remove("temperature");
            }
            if !self.policy.reasoning {
                body.remove("reasoning");
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct ReplayData {
    version: u8,
    output: Vec<Value>,
    effort: Option<String>,
    response_id: Option<String>,
}

#[derive(Default)]
struct Captured {
    event_data: Vec<u8>,
    items: BTreeMap<u64, Value>,
    completed: Option<ReplayData>,
}

pub(crate) struct Exchange {
    origin: String,
    model: String,
    effort: Option<String>,
    captured: Mutex<Captured>,
}

impl Exchange {
    pub(crate) fn new(origin: String, model: &str, effort: Option<&str>) -> Self {
        Self {
            origin,
            model: model.to_string(),
            effort: effort.map(str::to_string),
            captured: Mutex::new(Captured::default()),
        }
    }

    /// Only completed items are authoritative. In particular, never save the
    /// incomplete encrypted_content from response.output_item.added.
    pub(crate) fn observe_line(&self, line: &[u8]) {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let mut captured = self.captured.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(data) = line.strip_prefix(b"data:") {
            if !captured.event_data.is_empty() {
                captured.event_data.push(b'\n');
            }
            captured
                .event_data
                .extend_from_slice(data.strip_prefix(b" ").unwrap_or(data));
            return;
        }
        if !line.is_empty() {
            return;
        }
        // SSE joins multiple data fields with newlines and dispatches on a
        // blank line. Parsing each line alone silently lost valid pretty JSON.
        let data = std::mem::take(&mut captured.event_data);
        let Ok(frame) = serde_json::from_slice::<Value>(&data) else {
            return;
        };
        match frame["type"].as_str() {
            Some("response.output_item.done") => {
                if let (Some(index), Some(item)) =
                    (frame["output_index"].as_u64(), frame.get("item"))
                {
                    captured.items.insert(index, item.clone());
                }
            }
            Some("response.completed") if frame["response"]["status"] == "completed" => {
                let mut output = frame["response"]["output"]
                    .as_array()
                    .filter(|items| !items.is_empty())
                    .cloned()
                    .unwrap_or_else(|| captured.items.values().cloned().collect());
                // Some gateways omit encrypted_content from the terminal
                // restatement even though the completed item carried it.
                for item in &mut output {
                    if item["type"] != "reasoning"
                        || item["encrypted_content"]
                            .as_str()
                            .is_some_and(|s| !s.is_empty())
                    {
                        continue;
                    }
                    if let Some(complete) = captured
                        .items
                        .values()
                        .find(|old| old["id"].as_str().is_some() && old["id"] == item["id"])
                    {
                        if let Some(encrypted) = complete["encrypted_content"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                        {
                            item["encrypted_content"] = json!(encrypted);
                        }
                    }
                }
                captured.completed = Some(ReplayData {
                    version: 1,
                    output,
                    effort: self.effort.clone(),
                    response_id: frame["response"]["id"].as_str().map(str::to_string),
                });
            }
            _ => {}
        }
    }

    pub(crate) fn state(&self) -> Option<ProviderState> {
        let captured = self.captured.lock().unwrap_or_else(|e| e.into_inner());
        let data = captured
            .completed
            .as_ref()
            .filter(|d| !d.output.is_empty())?;
        Some(ProviderState {
            origin: self.origin.clone(),
            model: self.model.clone(),
            data: serde_json::to_string(data).ok()?,
        })
    }
}

pub(crate) struct ReplayGroup {
    data: ReplayData,
}

impl ReplayGroup {
    pub(crate) fn message_id(&self) -> Option<&str> {
        self.data
            .output
            .iter()
            .find(|item| item["type"] == "message")?["id"]
            .as_str()
    }

    pub(crate) fn reasoning_id(&self) -> Option<&str> {
        self.data
            .output
            .iter()
            .find(|item| item["type"] == "reasoning")?["id"]
            .as_str()
    }
}

#[derive(Default)]
pub(crate) struct ReplayPlan {
    groups: BTreeMap<usize, ReplayGroup>,
    anchors: HashMap<String, usize>,
    pub(crate) update_effort: Option<String>,
}

impl ReplayPlan {
    pub(crate) fn new(
        messages: &[ChatMessage],
        origin: &str,
        model: &str,
        update_effort: Option<&str>,
    ) -> Self {
        let mut plan = Self {
            update_effort: update_effort.map(str::to_string),
            ..Self::default()
        };
        let mut conflicts = HashSet::new();
        for (index, message) in messages.iter().enumerate() {
            let Some(state) = message.provider_state.as_ref().filter(|state| {
                message.role == Role::Assistant && state.origin == origin && state.model == model
            }) else {
                continue;
            };
            let Ok(data) = serde_json::from_str::<ReplayData>(&state.data) else {
                continue;
            };
            // History sanitization, hooks, egress masking and budget trimming
            // may have edited this message. Never resurrect the old payload.
            if data.version != 1 || !matches_message(&data.output, message) {
                continue;
            }
            let keys: Vec<_> = data.output.iter().filter_map(anchor).collect();
            if keys.is_empty() {
                continue;
            }
            for key in keys {
                if let Some(old) = plan.anchors.insert(key, index) {
                    conflicts.insert(old);
                    conflicts.insert(index);
                }
            }
            plan.groups.insert(index, ReplayGroup { data });
        }
        // Duplicate provider ids cannot safely identify a turn. Fall back to
        // ordinary id-less history instead of attaching the wrong opaque data.
        plan.groups.retain(|index, _| !conflicts.contains(index));
        plan.anchors.retain(|_, index| !conflicts.contains(index));
        plan
    }

    pub(crate) fn group(&self, message_index: usize) -> Option<&ReplayGroup> {
        self.groups.get(&message_index)
    }

    /// Rig transports the ordinary history; this last wire-boundary step puts
    /// the untouched provider items back, including fields Rig does not model
    /// (message phase and future output-item metadata).
    pub(crate) fn apply(&self, body: &mut Value) {
        let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
            return;
        };
        let mut emitted = HashSet::new();
        let mut patched = Vec::new();
        for item in std::mem::take(input) {
            let group = anchor(&item).and_then(|key| self.anchors.get(&key));
            if let Some(index) = group {
                if emitted.insert(*index) {
                    let data = &self.groups[index].data;
                    if self.update_effort.is_some() {
                        if let Some(effort) = &data.effort {
                            patched.push(effort_item(effort));
                        }
                    }
                    patched.extend(data.output.iter().cloned());
                }
            } else {
                patched.push(item);
            }
        }
        if let Some(effort) = &self.update_effort {
            patched.push(effort_item(effort));
        }
        *input = patched;
    }
}

fn effort_item(effort: &str) -> Value {
    json!({"type":"configuration_update", "reasoning":{"effort":effort}})
}

fn anchor(item: &Value) -> Option<String> {
    let kind = item["type"].as_str()?;
    let id = match kind {
        "function_call" => item["call_id"].as_str()?,
        "message" | "reasoning" => item["id"].as_str()?,
        _ => return None,
    };
    (!id.is_empty()).then(|| format!("{kind}:{id}"))
}

fn matches_message(output: &[Value], message: &ChatMessage) -> bool {
    let mut text = String::new();
    let mut calls = Vec::new();
    for item in output {
        match item["type"].as_str() {
            Some("message") => {
                let Some(content) = item["content"].as_array() else {
                    return false;
                };
                for block in content {
                    match block["type"].as_str() {
                        Some("output_text") => {
                            text.push_str(block["text"].as_str().unwrap_or_default())
                        }
                        Some("refusal") => {
                            text.push_str(block["refusal"].as_str().unwrap_or_default())
                        }
                        _ => return false,
                    }
                }
            }
            Some("function_call") => calls.push(item),
            Some("custom_tool_call") => return false, // not a supported client tool wire
            _ => {}
        }
    }
    if text != message.content.as_deref().unwrap_or_default() {
        return false;
    }
    let stored = message.tool_calls.as_deref().unwrap_or_default();
    calls.len() == stored.len()
        && calls.iter().zip(stored).all(|(item, call)| {
            item["call_id"] == call.id
                && item["name"] == call.name
                && same_arguments(
                    item["arguments"].as_str().unwrap_or_default(),
                    &call.arguments,
                )
        })
}

fn same_arguments(left: &str, right: &str) -> bool {
    match (
        serde_json::from_str::<Value>(left),
        serde_json::from_str::<Value>(right),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}
