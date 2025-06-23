use std::{
    pin::Pin,
    task::{Context, Poll},
};

use anyhow::Result;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};

use crate::types::{AssistantContent, ToolCall, ToolFunction};

#[derive(Debug, Clone)]
pub enum RawStreamingChoice {
    /// A text chunk from a message response
    Message(String),

    /// A tool call response chunk
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },

    FinalResponse(ResponseUsage),
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ResponseUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

pub type StreamingResult = Pin<Box<dyn Stream<Item = Result<RawStreamingChoice>> + Send>>;

/// The response from a streaming completion request;
/// message and response are populated at the end of the
/// `inner` stream.
pub struct StreamingCompletionResponse {
    inner: StreamingResult,
    text: String,
    tool_calls: Vec<ToolCall>,
    /// The final aggregated message from the stream
    /// contains all text and tool calls generated
    pub choice: Vec<AssistantContent>,
    /// The final response from the stream, may be `None`
    /// if the provider didn't yield it during the stream
    pub response: Option<ResponseUsage>,
}

impl StreamingCompletionResponse {
    pub fn new(inner: StreamingResult) -> StreamingCompletionResponse {
        Self {
            inner,
            text: "".to_string(),
            tool_calls: vec![],
            choice: vec![],
            response: None,
        }
    }
}

impl Stream for StreamingCompletionResponse {
    type Item = Result<AssistantContent>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let stream = self.get_mut();

        match stream.inner.as_mut().poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => {
                // This is run at the end of the inner stream to collect all tokens into
                // a single unified `Message`.
                let mut choice = vec![];

                stream.tool_calls.iter().for_each(|tc| {
                    choice.push(AssistantContent::ToolCall(tc.clone()));
                });

                // This is required to ensure there's always at least one item in the content
                if choice.is_empty() || !stream.text.is_empty() {
                    choice.insert(0, AssistantContent::text(stream.text.clone()));
                }

                stream.choice = choice;

                Poll::Ready(None)
            }
            Poll::Ready(Some(Err(err))) => Poll::Ready(Some(Err(err))),
            Poll::Ready(Some(Ok(choice))) => match choice {
                RawStreamingChoice::Message(text) => {
                    // Forward the streaming tokens to the outer stream
                    // and concat the text together
                    stream.text = format!("{}{}", stream.text, text.clone());
                    Poll::Ready(Some(Ok(AssistantContent::text(text))))
                }
                RawStreamingChoice::ToolCall {
                    id,
                    name,
                    arguments,
                } => {
                    // Keep track of each tool call to aggregate the final message later
                    // and pass it to the outer stream
                    stream.tool_calls.push(ToolCall {
                        id: id.clone(),
                        function: ToolFunction {
                            name: name.clone(),
                            arguments: arguments.clone(),
                        },
                    });
                    Poll::Ready(Some(Ok(AssistantContent::tool_call(id, name, arguments))))
                }
                RawStreamingChoice::FinalResponse(response) => {
                    // Set the final response field and return the next item in the stream
                    stream.response = Some(response);

                    stream.poll_next_unpin(cx)
                }
            },
        }
    }
}
