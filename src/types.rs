// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use serde::{Deserialize, Serialize};

pub mod streaming;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    /// User message containing one or more content types defined by `UserContent`.
    User { content: Vec<UserContent> },

    /// Assistant message containing one or more content types defined by `AssistantContent`.
    Assistant { content: Vec<AssistantContent> },
}

impl Message {
    pub fn user(text: &str) -> Self {
        Message::User {
            content: vec![UserContent::Text(Text {
                text: text.to_string(),
            })],
        }
    }

    pub fn assistant(text: &str) -> Self {
        Message::Assistant {
            content: vec![AssistantContent::Text(Text {
                text: text.to_string(),
            })],
        }
    }

    pub fn tool_call(tool_call: ToolCall) -> Self {
        Message::Assistant {
            content: vec![AssistantContent::ToolCall(tool_call)],
        }
    }

    pub fn tool_result(id: &str, content: Vec<ToolResultContent>) -> Self {
        Message::User {
            content: vec![UserContent::ToolResult(ToolResult {
                id: id.to_string(),
                content,
            })],
        }
    }
}

/// Describes the content of a message, which can be text, a tool result, an image, audio, or
///  a document. Dependent on provider supporting the content type. Multimedia content is generally
///  base64 (defined by it's format) encoded but additionally supports urls (for some providers).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum UserContent {
    Text(Text),
    ToolResult(ToolResult),
    Image(Image),
    Audio(Audio),
    Document(Document),
}

/// Describes responses from a provider which is either text or a tool call.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum AssistantContent {
    Text(Text),
    ToolCall(ToolCall),
}

impl AssistantContent {
    pub fn text(text: String) -> Self {
        AssistantContent::Text(Text { text })
    }

    pub fn tool_call(id: String, name: String, arguments: serde_json::Value) -> Self {
        AssistantContent::ToolCall(ToolCall {
            id,
            function: ToolFunction { name, arguments },
        })
    }
}

/// Tool result content containing information about a tool call and it's resulting content.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ToolResult {
    pub id: String,
    pub content: Vec<ToolResultContent>,
}

/// Describes the content of a tool result, which can be text or an image.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub enum ToolResultContent {
    Text(Text),
    Image(Image),
}

impl ToolResultContent {
    pub fn text(text: String) -> Self {
        ToolResultContent::Text(Text { text })
    }
    pub fn image(data: String, media_type: Option<ImageMediaType>) -> Self {
        ToolResultContent::Image(Image {
            data,
            format: None,
            media_type,
            detail: None,
        })
    }
    pub fn image_url(url: String, media_type: Option<ImageMediaType>) -> Self {
        ToolResultContent::Image(Image {
            data: url,
            format: Some(ContentFormat::String),
            media_type,
            detail: None,
        })
    }
}
/// Describes a tool call with an id and function to call, generally produced by a provider.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub function: ToolFunction,
}

/// Describes a tool function to call with a name and arguments, generally produced by a provider.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ToolFunction {
    pub name: String,
    pub arguments: serde_json::Value,
}

// ================================================================
// Base content models
// ================================================================

/// Basic text content.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Text {
    pub text: String,
}

/// Image content containing image data and metadata about it.
#[derive(Default, Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Image {
    pub data: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<ContentFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<ImageMediaType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<ImageDetail>,
}

/// Audio content containing audio data and metadata about it.
#[derive(Default, Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Audio {
    pub data: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<ContentFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<AudioMediaType>,
}

/// Document content containing document data and metadata about it.
#[derive(Default, Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Document {
    pub data: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<ContentFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<DocumentMediaType>,
}

/// Describes the format of the content, which can be base64 or string.
#[derive(Default, Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ContentFormat {
    #[default]
    Base64,
    String,
}

/// Helper enum that tracks the media type of the content.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub enum MediaType {
    Image(ImageMediaType),
    Audio(AudioMediaType),
    Document(DocumentMediaType),
}

/// Describes the image media type of the content. Not every provider supports every media type.
/// Convertible to and from MIME type strings.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[allow(clippy::upper_case_acronyms)]
pub enum ImageMediaType {
    JPEG,
    PNG,
    GIF,
    WEBP,
    HEIC,
    HEIF,
    SVG,
}

impl ImageMediaType {
    pub fn to_mime_type(&self) -> &'static str {
        match self {
            ImageMediaType::JPEG => "image/jpeg",
            ImageMediaType::PNG => "image/png",
            ImageMediaType::GIF => "image/gif",
            ImageMediaType::WEBP => "image/webp",
            ImageMediaType::HEIC => "image/heic",
            ImageMediaType::HEIF => "image/heif",
            ImageMediaType::SVG => "image/svg+xml",
        }
    }

    pub fn to_file_ext(&self) -> &str {
        match self {
            ImageMediaType::JPEG => "jpg",
            ImageMediaType::PNG => "png",
            ImageMediaType::GIF => "gif",
            ImageMediaType::WEBP => "webp",
            ImageMediaType::HEIC => "heic",
            ImageMediaType::HEIF => "heif",
            ImageMediaType::SVG => "svg",
        }
    }

    pub fn from_mime_type(mime_type: &str) -> Option<Self> {
        match mime_type {
            "image/jpeg" => Some(ImageMediaType::JPEG),
            "image/png" => Some(ImageMediaType::PNG),
            "image/gif" => Some(ImageMediaType::GIF),
            "image/webp" => Some(ImageMediaType::WEBP),
            "image/heic" => Some(ImageMediaType::HEIC),
            "image/heif" => Some(ImageMediaType::HEIF),
            "image/svg+xml" => Some(ImageMediaType::SVG),
            _ => None,
        }
    }
}

/// Describes the document media type of the content. Not every provider supports every media type.
/// Includes also programming languages as document types for providers who support code running.
/// Convertible to and from MIME type strings.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[allow(clippy::upper_case_acronyms)]
pub enum DocumentMediaType {
    PDF,
    TXT,
    RTF,
    HTML,
    CSS,
    MARKDOWN,
    CSV,
    XML,
    Javascript,
    Python,
}

/// Describes the audio media type of the content. Not every provider supports every media type.
/// Convertible to and from MIME type strings.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[allow(clippy::upper_case_acronyms)]
pub enum AudioMediaType {
    WAV,
    MP3,
    AIFF,
    AAC,
    OGG,
    FLAC,
}

/// Describes the detail of the image content, which can be low, high, or auto (open-ai specific).
#[derive(Default, Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ImageDetail {
    Low,
    High,
    #[default]
    Auto,
}

impl Message {
    pub fn string_context(&self) -> String {
        match self {
            Message::User { content } => content
                .iter()
                .map(|c| match c {
                    UserContent::Text(Text { text }) => text.to_string(),
                    _ => "".to_string(),
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Message::Assistant { content } => content
                .iter()
                .map(|c| match c {
                    AssistantContent::Text(Text { text }) => text.to_string(),
                    _ => "".to_string(),
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}
