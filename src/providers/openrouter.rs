// Copyright В© 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{collections::HashMap, fmt::Write, pin::Pin};

use anyhow::{Result, anyhow};
use async_stream::stream;
use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    providers::ProviderClient,
    types::{
        AssistantContent, ContentFormat, ImageMediaType, Message, Text, ToolCall, ToolFunction,
        ToolResultContent, UserContent,
        streaming::{RawStreamingChoice, StreamingCompletionResponse},
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
    pub usage: Option<serde_json::Value>,
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
}

/// Anthropic allows only 4 blocks marked for caching
const MAX_CACHE_BLOCKS: i8 = 4;

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
        UserContent::Image(image) => {
            if let Some(ContentFormat::String) = image.format {
                Ok(json!({
                    "type": "image_url",
                    "image_url": {
                        "url": image.data,
                    }
                }))
            } else {
                Ok(json!({
                    "type": "image_url",
                    "image_url": {
                        "url": format!("data:{};base64,{}", image.media_type.as_ref().unwrap_or(&ImageMediaType::PNG).to_mime_type(), image.data),
                    }
                }))
            }
        }
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
    pub fn new(api_key: &str, model: &str) -> Result<Self> {
        Ok(Self {
            base_url: OPENROUTER_API_BASE_URL.to_string(),
            model: model.to_string(),
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
        tools: &[serde_json::Value],
    ) -> Result<serde_json::Value> {
        let need_cache_control = self.model.starts_with("anthropic/");
        let mut full_history = vec![if need_cache_control {
            json!({
                "role": "system",
                "content": system_prompt,
                "cache_control": {
                    "type": "ephemeral"
                }
            })
        } else {
            json!({
                "role": "system",
                "content": system_prompt,
            })
        }];

        // initial cache block for system prompt and tools
        let mut cache_blocks = 2;

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
                                            if let Some(ContentFormat::String) = image.format {
                                                full_history.push(json!({
                                                    "role": "user",
                                                    "content": [{
                                                        "type": "image_url",
                                                        "image_url": {
                                                            "url": image.data,
                                                        }
                                                    }]
                                                }));
                                            } else {
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

        if need_cache_control {
            let len = full_history.len();
            full_history = full_history
                .iter_mut()
                .enumerate()
                .map(|(idx, m)| {
                    if idx > 0 && idx < len - 1 && cache_blocks < MAX_CACHE_BLOCKS {
                        let mut message = m.clone();
                        let message = message.as_object_mut().unwrap();
                        if message.contains_key("content") {
                            if message["content"].is_string()
                                && !message["content"]
                                    .as_str()
                                    .unwrap()
                                    .starts_with("<context>")
                            {
                                message.insert(
                                    "cache_control".to_string(),
                                    json!({"type": "ephemeral"}),
                                );
                                cache_blocks += 1;
                            } else if message["content"].is_array() {
                                let content =
                                    message.get_mut("content").unwrap().as_array_mut().unwrap();
                                for content in content.iter_mut() {
                                    let content = content.as_object_mut().unwrap();
                                    if cache_blocks < MAX_CACHE_BLOCKS
                                        && ((content.contains_key("text")
                                            && !content["text"]
                                                .as_str()
                                                .unwrap()
                                                .starts_with("<context>"))
                                            || content.contains_key("image_url"))
                                    {
                                        content.insert(
                                            "cache_control".to_string(),
                                            json!({"type": "ephemeral"}),
                                        );
                                        cache_blocks += 1;
                                    }
                                }
                            } else if message["content"].is_null() {
                            }
                        }
                        serde_json::to_value(message).unwrap()
                    } else {
                        m.clone()
                    }
                })
                .collect()
        }

        let mut request = json!({
            "model": self.model,
            "messages": full_history,
            "stream": true,
            "temperature": 0.0,
            "usage": {
                "include": true
            }
        });
        if !tools.is_empty() {
            let tools = if self.model.starts_with("anthropic/") {
                tools
                    .iter()
                    .enumerate()
                    .map(|(idx, t)| {
                        if idx == tools.len() - 1 {
                            let mut tool = t.clone();
                            let tool = tool.as_object_mut().unwrap();
                            tool.insert("cache_control".to_string(), json!({"type": "ephemeral"}));
                            serde_json::to_value(tool).unwrap()
                        } else {
                            t.clone()
                        }
                    })
                    .collect()
            } else {
                tools.to_vec()
            };
            request["tools"] = serde_json::Value::Array(tools);
        }
        Ok(request)
    }

    async fn sse_lines_stream<E: std::error::Error + Send + Sync + 'static>(
        mut stream: impl Stream<Item = std::result::Result<Bytes, E>> + Unpin + Send + 'static,
    ) -> Pin<Box<dyn Stream<Item = Result<String>> + Send>> {
        const CR: u8 = 0x0D;
        const LF: u8 = 0x0A;

        Box::pin(stream! {
            let mut chunks: Vec<Bytes> = Vec::new();
            let mut chunks_length = 0;
            let mut has_end_carriage = false;
            while let Some(chunk_result) = stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(anyhow!(e));
                        break;
                    }
                };
                if chunk.is_empty() {
                    continue;
                }
                let mut chunk_start = 0;
                for (idx, &b) in chunk.iter().enumerate() {
                    if has_end_carriage {
                        has_end_carriage = false;
                        if b == LF {
                            chunk_start += 1;
                            continue;
                        }
                    }
                    if b == CR || b == LF {
                        has_end_carriage = b == CR;
                        let total_line_length = chunks_length + idx - chunk_start;
                        let mut buf = Vec::with_capacity(total_line_length);
                        for c in chunks.drain(..) {
                            buf.extend_from_slice(&c);
                        }
                        buf.extend_from_slice(&chunk[chunk_start..idx]);
                        chunk_start = idx + 1;
                        let line = match String::from_utf8(buf) {
                            Ok(t) => t,
                            Err(e) => {
                                yield Err(anyhow!(e));
                                break;
                            }
                        };
                        yield Ok(line);
                    }
                }
                let chunk = chunk.slice(chunk_start..);
                if !chunk.is_empty() {
                    chunks_length += chunk.len();
                    chunks.push(chunk);
                }
            }
            if chunks_length > 0 {
                let total_line_length = chunks_length;
                let mut buf = Vec::with_capacity(total_line_length);
                for c in chunks.drain(..) {
                    buf.extend_from_slice(&c);
                }
                match String::from_utf8(buf) {
                    Ok(line) => {
                        yield Ok(line);
                    },
                    Err(e) => {
                        yield Err(anyhow!(e));
                    }
                };
            }
        })
    }

    async fn sse_events_stream(
        line_stream: impl Stream<Item = Result<String>> + Unpin + Send + 'static,
    ) -> Pin<Box<dyn Stream<Item = Result<(String, String, String)>> + Send>> {
        Box::pin(stream! {
            let mut stream = line_stream;
            let mut event_type = String::new();
            let mut event_data = String::new();
            let mut event_last_id = String::new();

            while let Some(line_result) = stream.next().await {
                let line = match line_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(e);
                        break;
                    }
                };
                if line.is_empty() {
                    if event_data.is_empty() {
                        event_type.clear();
                        continue;
                    }
                    let mut new_event_data = std::mem::take(&mut event_data);
                    let mut new_event_type = std::mem::take(&mut event_type);
                    if new_event_data.ends_with('\n') {
                        new_event_data.truncate(new_event_data.len() - 1);
                    }
                    if new_event_type.is_empty() {
                        new_event_type.push_str("message");
                    }
                    yield Ok((event_last_id.clone(), new_event_type, new_event_data));
                }

                // Skip comments
                if line.starts_with(":") {
                    continue;
                }
                let (field_name, mut field_value) = line.split_once(":").unwrap_or((line.as_str(), ""));
                if field_value.starts_with(' ') {
                    field_value = &field_value[1..];
                }
                match field_name {
                    "id" => {
                        if !field_value.contains('\0') {
                            event_last_id = field_value.to_string();
                        }
                    }
                    "event" => {
                        event_type = field_value.to_string();
                    }
                    "data" => {
                        event_data.write_fmt(format_args!("{field_value}\n")).expect("write to string does not fail");
                    }
                    _ => (),
                }
            }
        })
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
        let response_stream = response.bytes_stream();
        let line_stream = Self::sse_lines_stream(response_stream).await;
        let events_stream = Self::sse_events_stream(line_stream).await;
        // Handle OpenAI Compatible SSE chunks
        let stream = Box::pin(stream! {
            let mut stream = events_stream;
            let mut tool_calls = HashMap::new();
            let mut final_usage = None;
            while let Some(event_result) = stream.next().await {
                let (_, _, event_data) = match event_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(e);
                        break;
                    }
                };
                // OpenAI (and OpenRouter) uses [DONE] as marker of last message in stream
                if event_data == "[DONE]" {
                    break;
                }
                let data = match serde_json::from_str::<OpenRouterStreamingCompletionResponse>(&event_data) {
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
                    for tool_call in &message.tool_calls {
                        let name = tool_call.function.name.clone();
                        let id = tool_call.id.clone();
                        let arguments = if let Some(args) = &tool_call.function.arguments {
                            // Try to parse the string as JSON, fallback to string value
                            if !args.is_empty() {
                                match serde_json::from_str(args) {
                                    Ok(v) => v,
                                    Err(_) => serde_json::Value::String(args.to_string()),
                                }
                            } else {
                                serde_json::Value::Object(Default::default())
                            }
                        } else {
                            serde_json::Value::Object(Default::default())
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

                    if !message.content.is_empty() {
                        yield Ok(RawStreamingChoice::Message(message.content.clone()))
                    }
                }
            }

            for (_, tool_call) in tool_calls.into_iter() {
                let arguments = if tool_call.function.arguments.is_object() {
                    tool_call.function.arguments
                } else {
                    Value::Object(Default::default())
                };

                yield Ok(RawStreamingChoice::ToolCall{
                    name: tool_call.function.name,
                    id: tool_call.id,
                    arguments
                });
            }

            // if let Some(final_usage) = final_usage.clone() {
            //     tracing::debug!("Final usage: {}", serde_json::to_string_pretty(&final_usage).unwrap());
            // }
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
        tools: &[serde_json::Value],
    ) -> Result<StreamingCompletionResponse> {
        let request = self
            .prepare_request(system_prompt, context, messages, tools)
            .await?;
        if std::env::var_os("HULY_AI_AGENT_TRACE_REQUEST").is_some() {
            std::fs::write(
                "request.json",
                serde_json::to_string_pretty(&request).unwrap(),
            )
            .unwrap();
        }
        let builder = self.post("/chat/completions").json(&request);
        self.send_streaming_request(builder).await
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures::StreamExt;

    use super::Client;

    #[tokio::test]
    async fn test_sse_lines_stream() {
        let stream = futures::stream::iter([std::io::Result::Ok(Bytes::from_owner(
            "Hello, world!".to_string(),
        ))]);
        let lines: Vec<_> = Client::sse_lines_stream(stream)
            .await
            .map(|l| l.unwrap())
            .collect()
            .await;
        assert_eq!(lines, vec!["Hello, world!".to_string()]);

        let stream = futures::stream::iter([std::io::Result::Ok(Bytes::from_owner(
            "Hello,\nwo\r\rrld!".to_string(),
        ))]);
        let lines: Vec<_> = Client::sse_lines_stream(stream)
            .await
            .map(|l| l.unwrap())
            .collect()
            .await;
        assert_eq!(
            lines,
            vec![
                "Hello,".to_string(),
                "wo".to_string(),
                "".to_string(),
                "rld!".to_string()
            ]
        );

        let stream = futures::stream::iter([std::io::Result::Ok(Bytes::from_owner(
            "Hello,\rwo\n\nrld!".to_string(),
        ))]);
        let lines: Vec<_> = Client::sse_lines_stream(stream)
            .await
            .map(|l| l.unwrap())
            .collect()
            .await;
        assert_eq!(
            lines,
            vec![
                "Hello,".to_string(),
                "wo".to_string(),
                "".to_string(),
                "rld!".to_string()
            ]
        );

        let stream = futures::stream::iter([std::io::Result::Ok(Bytes::from_owner(
            "Hello,\r\nworld!".to_string(),
        ))]);
        let lines: Vec<_> = Client::sse_lines_stream(stream)
            .await
            .map(|l| l.unwrap())
            .collect()
            .await;
        assert_eq!(lines, vec!["Hello,".to_string(), "world!".to_string()]);

        let stream = futures::stream::iter([
            std::io::Result::Ok(Bytes::from_owner("Hello,\r".to_string())),
            std::io::Result::Ok(Bytes::from_owner("\nworld!".to_string())),
        ]);
        let lines: Vec<_> = Client::sse_lines_stream(stream)
            .await
            .map(|l| l.unwrap())
            .collect()
            .await;
        assert_eq!(lines, vec!["Hello,".to_string(), "world!".to_string()]);

        let stream = futures::stream::iter([
            std::io::Result::Ok(Bytes::from_static(&[0xF0, 0x9F])),
            std::io::Result::Ok(Bytes::from_static(&[0x8C, 0x8E])),
            std::io::Result::Ok(Bytes::from_owner("\nHello")),
        ]);
        let lines: Vec<_> = Client::sse_lines_stream(stream)
            .await
            .map(|l| l.unwrap())
            .collect()
            .await;
        assert_eq!(lines, vec!["🌎".to_string(), "Hello".to_string()]);
    }

    #[tokio::test]
    async fn test_sse_events_stream() {
        let stream = futures::stream::iter(
            r#": test stream

            data: first event
            id: 1

            data:second event
            id

            data:  third event"#
                .lines()
                .map(|s| Ok(s.trim().to_string())),
        );
        let events: Vec<_> = Client::sse_events_stream(stream)
            .await
            .map(|l| l.unwrap())
            .collect()
            .await;
        assert_eq!(
            events,
            vec![
                (
                    "1".to_string(),
                    "message".to_string(),
                    "first event".to_string()
                ),
                (
                    "".to_string(),
                    "message".to_string(),
                    "second event".to_string()
                ),
            ]
        );
        let stream = futures::stream::iter(
            r#"data: YHOO
            data: +2
            data: 10
            "#
            .lines()
            .map(|s| Ok(s.trim().to_string())),
        );
        let events: Vec<_> = Client::sse_events_stream(stream)
            .await
            .map(|l| l.unwrap())
            .collect()
            .await;
        assert_eq!(
            events,
            vec![(
                "".to_string(),
                "message".to_string(),
                "YHOO\n+2\n10".to_string()
            )]
        );
        let stream = futures::stream::iter(
            r#"data

            data
            data

            data:"#
                .lines()
                .map(|s| Ok(s.trim().to_string())),
        );
        let events: Vec<_> = Client::sse_events_stream(stream)
            .await
            .map(|l| l.unwrap())
            .collect()
            .await;
        assert_eq!(
            events,
            vec![
                ("".to_string(), "message".to_string(), "".to_string()),
                ("".to_string(), "message".to_string(), "\n".to_string())
            ]
        );
    }
}
