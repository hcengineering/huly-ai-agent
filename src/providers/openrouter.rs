// Copyright В© 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::collections::HashMap;

use anyhow::{Result, anyhow};
use async_stream::stream;
use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    providers::ProviderClient,
    types::{
        AssistantContent, ImageMediaType, Message, Text, ToolCall, ToolFunction, ToolResultContent,
        UserContent,
        streaming::{RawStreamingChoice, ResponseUsage, StreamingCompletionResponse},
    },
};

const OPENROUTER_API_BASE_URL: &str = "https://openrouter.ai/api/v1";

#[derive(Serialize, Deserialize, Debug)]
pub struct OpenRouterStreamingCompletionResponse {
    pub id: String,
    pub choices: Vec<OpenRouterStreamingChoice>,
    pub created: u64,
    pub model: String,
    pub object: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ResponseUsage>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OpenRouterStreamingChoice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<Value>,
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<MessageResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<DeltaResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorResponse>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MessageResponse {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<Value>,
    #[serde(default)]
    pub tool_calls: Vec<OpenRouterToolCall>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OpenRouterToolFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OpenRouterToolCall {
    pub index: usize,
    pub id: Option<String>,
    pub r#type: Option<String>,
    pub function: OpenRouterToolFunction,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ErrorResponse {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, Value>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DeltaResponse {
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<OpenRouterToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_finish_reason: Option<String>,
}

#[derive(Clone)]
pub struct Client {
    base_url: String,
    http_client: reqwest::Client,
    model: String,
    tools: Vec<serde_json::Value>,
}

fn user_text_to_json(content: &UserContent) -> serde_json::Value {
    match content {
        UserContent::Text(text) => json!({
            "role": "user",
            "content": text.text,
        }),
        _ => unreachable!(),
    }
}

fn user_content_to_json(content: &UserContent) -> Result<serde_json::Value> {
    match content {
        UserContent::Text(text) => Ok(json!({
            "type": "text",
            "text": text.text
        })),
        UserContent::Image(image) => Ok(json!({
            "type": "image_url",
            "image_url": {
                "url": format!("data:{};base64,{}", image.media_type.as_ref().unwrap_or(&ImageMediaType::PNG).to_mime_type(), image.data),
            }
        })),
        UserContent::Audio(_) => anyhow::bail!("Audio is not supported"),
        UserContent::Document(_) => anyhow::bail!("Document is not supported"),
        UserContent::ToolResult(_) => unreachable!(),
    }
}

fn tool_content_to_json(content: Vec<&UserContent>) -> Result<serde_json::Value> {
    let mut str_content = String::new();
    let mut tool_id = String::new();

    for content in content.into_iter() {
        match content {
            UserContent::ToolResult(tool_result) => {
                tool_id = tool_result.id.clone();
                str_content = tool_result
                    .content
                    .iter()
                    .map(|c| match c {
                        ToolResultContent::Text(text) => text.text.clone(),
                        // ignore image content
                        _ => "".to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join("");
            }
            _ => unreachable!(),
        }
    }
    Ok(json!({
        "role": "tool",
        "content": str_content,
        "tool_call_id": tool_id,
    }))
}

impl Client {
    /// Create a new OpenRouter client with the given API key and base API URL.
    pub fn new(api_key: &str, model: &str, tools: Vec<serde_json::Value>) -> Result<Self> {
        Ok(Self {
            base_url: OPENROUTER_API_BASE_URL.to_string(),
            model: model.to_string(),
            tools,
            http_client: reqwest::Client::builder()
                .default_headers({
                    let mut headers = reqwest::header::HeaderMap::new();
                    headers.insert("Authorization", format!("Bearer {api_key}").parse()?);
                    headers.insert("HTTP-Referer", "https://huly.io".parse().unwrap());
                    headers.insert("X-Title", "Huly".parse().unwrap());
                    headers
                })
                .build()?,
        })
    }

    pub(crate) fn post(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}/{}", self.base_url, path).replace("//", "/");
        self.http_client.post(url)
    }

    async fn prepare_request(
        &self,
        system_prompt: &str,
        context: &str,
        messages: &[Message],
    ) -> Result<serde_json::Value> {
        let mut full_history = vec![json!({
            "role": "system",
            "content": system_prompt,
        })];

        // Convert existing chat history
        for (idx, message) in messages.iter().enumerate() {
            match message {
                Message::User { content } => {
                    let content = if idx == 0 {
                        &content
                            .clone()
                            .into_iter()
                            .chain(std::iter::once(UserContent::Text(Text {
                                text: context.to_string(),
                            })))
                            .collect()
                    } else {
                        content
                    };
                    if content.len() == 1
                        && matches!(content.first().unwrap(), UserContent::Text(_))
                    {
                        full_history.push(user_text_to_json(content.first().unwrap()));
                    } else if content
                        .iter()
                        .any(|c| matches!(c, UserContent::ToolResult(_)))
                    {
                        let (tool_content, user_content) = content
                            .iter()
                            .partition::<Vec<_>, _>(|c| matches!(c, UserContent::ToolResult(_)));
                        full_history.push(tool_content_to_json(tool_content.clone())?);
                        for tool_content in tool_content.into_iter() {
                            match tool_content {
                                UserContent::ToolResult(result) => {
                                    for tool_result_content in result.content.clone().into_iter() {
                                        if let ToolResultContent::Image(image) = tool_result_content
                                        {
                                            full_history.push(json!({
                                                "role": "user",
                                                "content": [{
                                                    "type": "image_url",
                                                    "image_url": {
                                                        "url": format!("data:{};base64,{}", image.media_type.unwrap_or(ImageMediaType::PNG).to_mime_type(), image.data),
                                                    }
                                                }]
                                            }));
                                        }
                                    }
                                }
                                _ => unreachable!(),
                            }
                        }
                        if !user_content.is_empty() {
                            if user_content.len() == 1 {
                                full_history.push(user_text_to_json(user_content.first().unwrap()));
                            } else {
                                let user_content = user_content
                                    .into_iter()
                                    .map(user_content_to_json)
                                    .collect::<Result<Vec<_>, _>>()?;
                                full_history
                                    .push(json!({ "role": "user", "content": user_content}));
                            }
                        }
                    } else {
                        let content = content
                            .iter()
                            .map(user_content_to_json)
                            .collect::<Result<Vec<_>, _>>()?;
                        full_history.push(json!({ "role": "user", "content": content}));
                    }
                }
                Message::Assistant { content } => {
                    for content in content {
                        match content {
                            AssistantContent::Text(text) => {
                                full_history.push(json!({
                                    "role": "assistant",
                                    "content": text.text
                                }));
                            }
                            AssistantContent::ToolCall(tool_call) => {
                                full_history.push(json!({
                                    "role": "assistant",
                                    "content": null,
                                    "tool_calls": [{
                                        "id": tool_call.id,
                                        "type": "function",
                                        "function": {
                                            "name": tool_call.function.name,
                                            "arguments": tool_call.function.arguments.to_string()
                                        }
                                    }]
                                }));
                            }
                        }
                    }
                }
            };
        }

        let request = json!({
            "model": self.model,
            "messages": full_history,
            "tools": self.tools,
            "stream": true,
            "temperature": 0.0,
        });

        Ok(request)
    }

    async fn send_streaming_request(
        &self,
        request_builder: reqwest::RequestBuilder,
    ) -> Result<StreamingCompletionResponse> {
        let response = request_builder.send().await?;

        if !response.status().is_success() {
            return Err(anyhow!(format!(
                "{}: {}",
                response.status(),
                response.text().await?
            )));
        }

        // Handle OpenAI Compatible SSE chunks
        let stream = Box::pin(stream! {
            let mut stream = response.bytes_stream();
            let mut tool_calls = HashMap::new();
            let mut partial_line = String::new();
            let mut final_usage = None;

            while let Some(chunk_result) = stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(anyhow!(e));
                        break;
                    }
                };

                let text = match String::from_utf8(chunk.to_vec()) {
                    Ok(t) => t,
                    Err(e) => {
                        yield Err(anyhow!(e));
                        break;
                    }
                };

                for line in text.lines() {
                    let mut line = line.to_string();

                    // Skip empty lines and processing messages, as well as [DONE] (might be useful though)
                    if line.trim().is_empty() || line.trim() == ": OPENROUTER PROCESSING" || line.trim() == "data: [DONE]" {
                        continue;
                    }

                    // Handle data: prefix
                    line = line.strip_prefix("data: ").unwrap_or(&line).to_string();

                    // If line starts with { but doesn't end with }, it's a partial JSON
                    if line.starts_with('{') && !line.ends_with('}') {
                        partial_line = line;
                        continue;
                    }

                    // If we have a partial line and this line ends with }, complete it
                    if !partial_line.is_empty() {
                        if line.ends_with('}') {
                            partial_line.push_str(&line);
                            line = partial_line;
                            partial_line = String::new();
                        } else {
                            partial_line.push_str(&line);
                            continue;
                        }
                    }

                    let data = match serde_json::from_str::<OpenRouterStreamingCompletionResponse>(&line) {
                        Ok(data) => data,
                        Err(_) => {
                            continue;
                        }
                    };


                    let choice = data.choices.first().expect("Should have at least one choice");

                    // TODO this has to handle outputs like this:
                    // [{"index": 0, "id": "call_DdmO9pD3xa9XTPNJ32zg2hcA", "function": {"arguments": "", "name": "get_weather"}, "type": "function"}]
                    // [{"index": 0, "id": null, "function": {"arguments": "{\"", "name": null}, "type": null}]
                    // [{"index": 0, "id": null, "function": {"arguments": "location", "name": null}, "type": null}]
                    // [{"index": 0, "id": null, "function": {"arguments": "\":\"", "name": null}, "type": null}]
                    // [{"index": 0, "id": null, "function": {"arguments": "Paris", "name": null}, "type": null}]
                    // [{"index": 0, "id": null, "function": {"arguments": ",", "name": null}, "type": null}]
                    // [{"index": 0, "id": null, "function": {"arguments": " France", "name": null}, "type": null}]
                    // [{"index": 0, "id": null, "function": {"arguments": "\"}", "name": null}, "type": null}]
                    if let Some(delta) = &choice.delta {
                        if !delta.tool_calls.is_empty() {
                            for tool_call in &delta.tool_calls {
                                let index = tool_call.index;

                                // Get or create tool call entry
                                let existing_tool_call = tool_calls.entry(index).or_insert_with(|| ToolCall {
                                    id: String::new(),
                                    function: ToolFunction {
                                        name: String::new(),
                                        arguments: serde_json::Value::Null,
                                    },
                                });

                                // Update fields if present
                                if let Some(id) = &tool_call.id {
                                    if !id.is_empty() {
                                        existing_tool_call.id = id.clone();
                                    }
                                }
                                if let Some(name) = &tool_call.function.name {
                                    if !name.is_empty() {
                                        existing_tool_call.function.name = name.clone();
                                    }
                                }
                                if let Some(chunk) = &tool_call.function.arguments {
                                    // Convert current arguments to string if needed
                                    let current_args = match &existing_tool_call.function.arguments {
                                        serde_json::Value::Null => String::new(),
                                        serde_json::Value::String(s) => s.clone(),
                                        v => v.to_string(),
                                    };

                                    // Concatenate the new chunk
                                    let combined = format!("{current_args}{chunk}");

                                    // Try to parse as JSON if it looks complete
                                    if combined.trim_start().starts_with('{') && combined.trim_end().ends_with('}') {
                                        match serde_json::from_str(&combined) {
                                            Ok(parsed) => existing_tool_call.function.arguments = parsed,
                                            Err(_) => existing_tool_call.function.arguments = serde_json::Value::String(combined),
                                        }
                                    } else {
                                        existing_tool_call.function.arguments = serde_json::Value::String(combined);
                                    }
                                }
                            }
                        }

                        if let Some(content) = &delta.content {
                            if !content.is_empty() {
                                yield Ok(RawStreamingChoice::Message(content.clone()))
                            }
                        }

                        if let Some(usage) = data.usage {
                            final_usage = Some(usage);
                        }
                    }

                    // Handle message format
                    if let Some(message) = &choice.message {
                        if !message.tool_calls.is_empty() {
                            for tool_call in &message.tool_calls {
                                let name = tool_call.function.name.clone();
                                let id = tool_call.id.clone();
                                let arguments = if let Some(args) = &tool_call.function.arguments {
                                    // Try to parse the string as JSON, fallback to string value
                                    match serde_json::from_str(args) {
                                        Ok(v) => v,
                                        Err(_) => serde_json::Value::String(args.to_string()),
                                    }
                                } else {
                                    serde_json::Value::Null
                                };
                                let index = tool_call.index;

                                tool_calls.insert(index, ToolCall{
                                    id: id.unwrap_or_default(),
                                    function: ToolFunction {
                                        name: name.unwrap_or_default(),
                                        arguments,
                                    },
                                });
                            }
                        }

                        if !message.content.is_empty() {
                            yield Ok(RawStreamingChoice::Message(message.content.clone()))
                        }
                    }
                }
            }

            for (_, tool_call) in tool_calls.into_iter() {

                yield Ok(RawStreamingChoice::ToolCall{
                    name: tool_call.function.name,
                    id: tool_call.id,
                    arguments: tool_call.function.arguments
                });
            }

            yield Ok(RawStreamingChoice::FinalResponse(final_usage.unwrap_or_default()))

        });

        Ok(StreamingCompletionResponse::new(stream))
    }
}

#[async_trait]
impl ProviderClient for Client {
    async fn send_messages(
        &self,
        system_prompt: &str,
        context: &str,
        messages: &[Message],
    ) -> Result<StreamingCompletionResponse> {
        let request = self
            .prepare_request(system_prompt, context, messages)
            .await?;
        let builder = self.post("/chat/completions").json(&request);
        self.send_streaming_request(builder).await
    }
}
