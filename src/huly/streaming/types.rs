// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::fmt::Display;

use hulyrs::services::core::AccountUuid;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize, PartialEq)]
pub struct StreamingMessage {
    #[serde(flatten)]
    pub params: CommonParams,
    #[serde(flatten)]
    pub kind: StreamingMessageKind,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommonParams {
    #[serde(rename = "_id")]
    pub id: String,
    pub space: String,
    pub object_space: String,
    pub modified_by: String,
    pub modified_on: i64,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "_class")]
pub enum StreamingMessageKind {
    #[serde(rename = "core:class:TxWorkspaceEvent")]
    Workspace(WorkspaceEvent),
    #[serde(rename = "core:class:TxDomainEvent")]
    Domain(DomainEventKind),
    #[serde(untagged)]
    Unknown(Value),
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct WorkspaceEvent {}

#[derive(Debug, Deserialize, PartialEq)]
pub struct DomainEvent {}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "domain", content = "event", rename_all = "lowercase")]
pub enum DomainEventKind {
    #[serde(rename = "communication")]
    Communication(CommunicationDomainEventKind),
    #[serde(untagged)]
    UnknownEvent(Value),
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CommunicationDomainEventKind {
    CreateMessage(CreateMessage),
    AttachmentPatch(AttachmentPatch),
    ReactionPatch(ReactionPatch),
    ThreadPatch(ThreadPatch),
    UpdateNotificationContext(UpdateNotificationContext),
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MessageType {
    Message,
    Activity,
    #[serde(untagged)]
    Unknown(String),
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateMessage {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub message_id: String,
    pub message_type: MessageType,
    pub card_id: String,
    pub card_type: String,
    pub content: String,
    pub social_id: String,
    pub options: Option<CreateMessageOptions>,
    pub date: String,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentPatch {
    pub card_id: String,
    pub message_id: String,
    pub operations: Vec<AttachmentPatchOperation>,
    pub social_id: String,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentPatchOperation {
    pub opcode: String,
    pub attachments: Vec<AttachmentPatchOperationAttachment>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentPatchOperationAttachment {
    pub id: String,
    #[serde(rename = "type")]
    pub mime_type: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReactionPatch {
    pub card_id: String,
    pub message_id: String,
    pub operation: ReactionPatchOperation,
    pub social_id: String,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReactionPatchOperation {
    pub opcode: String,
    pub reaction: String,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadPatch {
    pub card_id: String,
    pub message_id: String,
    pub social_id: String,
    pub operation: ThreadPatchOperation,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "opcode")]
pub enum ThreadPatchOperation {
    Attach(AttachThreadOperation),
    Update(UpdateThreadOperation),
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AttachThreadOperation {
    pub thread_id: String,
    pub thread_type: String,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateThreadOperation {
    pub thread_id: String,
    pub updates: UpdateThreadOperationUpdates,
}

#[derive(Debug, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateThreadOperationUpdates {
    pub thread_type: Option<String>,
    pub replies_count_op: Option<ThreadRepliesCountOp>,
    pub last_reply: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ThreadRepliesCountOp {
    Increment,
    Decrement,
}

#[derive(Debug)]
pub enum CommunicationEvent {
    Message(ReceivedMessage),
    Reaction(ReceivedReaction),
    Attachment(ReceivedAttachment),
}

#[derive(Debug)]
pub struct ReceivedMessage {
    pub card_id: String,
    pub card_title: Option<String>,
    pub content: String,
    pub social_id: String,
    pub person_info: PersonInfo,
    pub message_id: String,
    pub date: String,
    pub is_mention: bool,
}

#[derive(Debug)]
pub struct ReceivedAttachment {
    pub channel_id: String,
    pub message_id: String,
    pub file_name: String,
    pub url: String,
}

#[derive(Debug)]
pub struct ReceivedReaction {
    pub channel_id: String,
    pub message_id: String,
    pub person: String,
    pub reaction: String,
}

#[derive(Debug, Clone, Default)]
pub struct PersonInfo {
    pub person_id: String,
    pub person_name: String,
}

impl Display for PersonInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}]({})", self.person_id, self.person_name)
    }
}

impl From<CreateMessage> for ReceivedMessage {
    fn from(value: CreateMessage) -> Self {
        ReceivedMessage {
            card_id: value.card_id,
            card_title: None,
            content: value.content,
            social_id: value.social_id,
            person_info: Default::default(),
            message_id: value.message_id,
            date: value.date,
            is_mention: false,
        }
    }
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateMessageOptions {
    pub skip_link_previews: bool,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateNotificationContext {
    #[serde(rename = "_id")]
    pub id: String,
    pub context_id: String,
    pub account: AccountUuid,
    pub updates: Option<UpdateNotificationContextUpdates>,
    pub date: String,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateNotificationContextUpdates {
    pub last_view: String,
}

mod test {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn test_deserialize_workspace_event() {
        let event = serde_json::from_str::<StreamingMessage>(
            r#"{
                "_class": "core:class:TxWorkspaceEvent",
                "_id": "685cce53ac5a06bc3c0e1c50",
                "event": 5,
                "modifiedBy": "core:account:System",
                "modifiedOn": 1750912595092,
                "objectSpace": "core:space:DerivedTx",
                "space": "core:space:DerivedTx",
                "params": {
                    "lastTx": "685cce537a178870b1f19a30"
                }
            }"#,
        )
        .unwrap();
        assert_eq!(
            event,
            StreamingMessage {
                params: CommonParams {
                    id: "685cce53ac5a06bc3c0e1c50".to_string(),
                    space: "core:space:DerivedTx".to_string(),
                    object_space: "core:space:DerivedTx".to_string(),
                    modified_by: "core:account:System".to_string(),
                    modified_on: 1750912595092,
                },
                kind: StreamingMessageKind::Workspace(WorkspaceEvent {}),
            }
        );
    }

    #[test]
    fn test_deserialize_domain_event_notification() {
        let event = serde_json::from_str::<StreamingMessage>(
            r#"{
                "_id": "685cce4cac5a06bc3c0e1c46",
                "space": "core:space:Tx",
                "objectSpace": "core:space:Domain",
                "_class": "core:class:TxDomainEvent",
                "domain": "communication",
                "event": {
                    "type": "updateNotificationContext",
                    "contextId": "1083586688821067777",
                    "account": "be650ba5-4d82-40b2-a123-7d1e66f9d55c",
                    "updates": {
                        "lastView": "2025-06-26T04:36:27.693Z"
                    },
                    "_id": "685cce4c7a178870b1f19a2b",
                    "date": "2025-06-26T04:36:28.176Z"
                },
                "modifiedBy": "1083545787011006465",
                "modifiedOn": 1750912588185
            }"#,
        )
        .unwrap();
        assert_eq!(
            event,
            StreamingMessage {
                params: CommonParams {
                    id: "685cce4cac5a06bc3c0e1c46".to_string(),
                    space: "core:space:Tx".to_string(),
                    object_space: "core:space:Domain".to_string(),
                    modified_by: "1083545787011006465".to_string(),
                    modified_on: 1750912588185,
                },
                kind: StreamingMessageKind::Domain(DomainEventKind::Communication(
                    CommunicationDomainEventKind::UpdateNotificationContext(
                        UpdateNotificationContext {
                            id: "685cce4c7a178870b1f19a2b".to_string(),
                            context_id: "1083586688821067777".to_string(),
                            account: "be650ba5-4d82-40b2-a123-7d1e66f9d55c".parse().unwrap(),
                            updates: Some(UpdateNotificationContextUpdates {
                                last_view: "2025-06-26T04:36:27.693Z".to_string(),
                            }),
                            date: "2025-06-26T04:36:28.176Z".to_string(),
                        }
                    )
                ))
            }
        );
    }

    #[test]
    fn test_deserialize_domain_event_create_message() {
        let event = serde_json::from_str::<StreamingMessage>(
            r#"{
                "_id": "685cce53ac5a06bc3c0e1c4d",
                "space": "core:space:Tx",
                "objectSpace": "core:space:Domain",
                "_class": "core:class:TxDomainEvent",
                "domain": "communication",
                "event": {
                    "type": "createMessage",
                    "messageType": "message",
                    "cardId": "685a65fefc4c285b40977554",
                    "cardType": "chat:masterTag:Channel",
                    "content": "asdasda",
                    "socialId": "1083545787011006465",
                    "options": {
                        "skipLinkPreviews": true
                    },
                    "_id": "685cce537a178870b1f19a2f",
                    "date": "2025-06-26T04:36:35.056Z",
                    "messageId": "10985817141951"
                },
                "modifiedBy": "1083545787011006465",
                "modifiedOn": 1750912595073
            }"#,
        )
        .unwrap();
        assert_eq!(
            event,
            StreamingMessage {
                params: CommonParams {
                    id: "685cce53ac5a06bc3c0e1c4d".to_string(),
                    space: "core:space:Tx".to_string(),
                    object_space: "core:space:Domain".to_string(),
                    modified_by: "1083545787011006465".to_string(),
                    modified_on: 1750912595073,
                },
                kind: StreamingMessageKind::Domain(DomainEventKind::Communication(
                    CommunicationDomainEventKind::CreateMessage(CreateMessage {
                        id: Some("685cce537a178870b1f19a2f".to_string()),
                        message_id: "10985817141951".to_string(),
                        message_type: MessageType::Message,
                        card_id: "685a65fefc4c285b40977554".to_string(),
                        card_type: "chat:masterTag:Channel".to_string(),
                        content: "asdasda".to_string(),
                        social_id: "1083545787011006465".to_string(),
                        options: Some(CreateMessageOptions {
                            skip_link_previews: true,
                        }),
                        date: "2025-06-26T04:36:35.056Z".to_string(),
                    })
                ))
            }
        );
    }

    #[test]
    fn test_deserialize_domain_event_create_message2() {
        let event = serde_json::from_str::<StreamingMessage>(
            r#"{
                "_class": "core:class:TxDomainEvent",
                "_id": "686bb8fede876734154057ef",
                "domain": "communication",
                "event": {
                    "cardId": "685a65fefc4c285b40977554",
                    "cardType": "chat:masterTag:Channel",
                    "content": "asdasda",
                    "date": "2025-07-07T12:09:34.729Z",
                    "messageId": "11089496186573",
                    "messageType": "message",
                    "socialId": "1083545787011006465",
                    "type": "createMessage"
                },
                "modifiedBy": "1083586469763645441",
                "modifiedOn": 1751890174741,
                "objectSpace": "core:space:Domain",
                "space": "core:space:Tx"
            }"#,
        )
        .unwrap();
        assert_eq!(
            event,
            StreamingMessage {
                params: CommonParams {
                    id: "686bb8fede876734154057ef".to_string(),
                    space: "core:space:Tx".to_string(),
                    object_space: "core:space:Domain".to_string(),
                    modified_by: "1083586469763645441".to_string(),
                    modified_on: 1751890174741,
                },
                kind: StreamingMessageKind::Domain(DomainEventKind::Communication(
                    CommunicationDomainEventKind::CreateMessage(CreateMessage {
                        id: None,
                        message_id: "11089496186573".to_string(),
                        message_type: MessageType::Message,
                        card_id: "685a65fefc4c285b40977554".to_string(),
                        card_type: "chat:masterTag:Channel".to_string(),
                        content: "asdasda".to_string(),
                        options: None,
                        social_id: "1083545787011006465".to_string(),
                        date: "2025-07-07T12:09:34.729Z".to_string(),
                    })
                ))
            }
        );
    }

    #[test]
    fn test_deserialize_domain_event_attachment_patch() {
        let event = serde_json::from_str::<StreamingMessage>(
            r#"{
                "_id": "689dd365bd0103b355bb6064",
                "space": "core:space:Tx",
                "objectSpace": "core:space:Domain",
                "_class": "core:class:TxDomainEvent",
                "domain": "communication",
                "event": {
                    "type": "attachmentPatch",
                    "cardId": "685a65fefc4c285b40977554",
                    "messageId": "11417859338547",
                    "operations": [
                        {
                            "opcode": "add",
                            "attachments": [
                                {
                                    "id": "9e381200-6e19-4583-8b97-d9e5ced5a02a",
                                    "type": "image/jpeg",
                                    "params": {
                                        "blobId": "9e381200-6e19-4583-8b97-d9e5ced5a02a",
                                        "mimeType": "image/jpeg",
                                        "fileName": "359f65cd-0b92-48ba-accb-e83d5d2ea424.jpeg",
                                        "size": 30857,
                                        "metadata": {
                                            "originalHeight": 640,
                                            "originalWidth": 640,
                                            "pixelRatio": 1
                                        }
                                    }
                                }
                            ]
                        }
                    ],
                    "socialId": "1083545787011006465",
                    "_id": "689dd3667b679a515ae0466b",
                    "date": "2025-08-14T12:15:33.866Z"
                },
                "modifiedBy": "1083545787011006465",
                "modifiedOn": 1755173733877
            }"#,
        )
        .unwrap();
        assert_eq!(
            event,
            StreamingMessage {
                params: CommonParams {
                    id: "689dd365bd0103b355bb6064".to_string(),
                    space: "core:space:Tx".to_string(),
                    object_space: "core:space:Domain".to_string(),
                    modified_by: "1083545787011006465".to_string(),
                    modified_on: 1755173733877,
                },
                kind: StreamingMessageKind::Domain(DomainEventKind::Communication(
                    CommunicationDomainEventKind::AttachmentPatch(AttachmentPatch {
                        card_id: "685a65fefc4c285b40977554".to_string(),
                        message_id: "11417859338547".to_string(),
                        operations: vec![AttachmentPatchOperation {
                            opcode: "add".to_string(),
                            attachments: vec![AttachmentPatchOperationAttachment {
                                id: "9e381200-6e19-4583-8b97-d9e5ced5a02a".to_string(),
                                mime_type: "image/jpeg".to_string(),
                                params: serde_json::json!({
                                    "blobId": "9e381200-6e19-4583-8b97-d9e5ced5a02a",
                                    "mimeType": "image/jpeg",
                                    "fileName": "359f65cd-0b92-48ba-accb-e83d5d2ea424.jpeg",
                                    "size": 30857,
                                    "metadata": {
                                        "originalHeight": 640,
                                        "originalWidth": 640,
                                        "pixelRatio": 1
                                    }
                                })
                            }]
                            .into_iter()
                            .collect()
                        }],
                        social_id: "1083545787011006465".to_string(),
                    })
                ))
            }
        );
    }

    #[test]
    fn test_deserialize_domain_event_reaction_patch() {
        let event = serde_json::from_str::<StreamingMessage>(
            r#"{
                "_id": "689dd365bd0103b355bb6064",
                "space": "core:space:Tx",
                "objectSpace": "core:space:Domain",
                "_class": "core:class:TxDomainEvent",
                "domain": "communication",
                "event": {
                    "type": "reactionPatch",
                    "cardId": "685a65fefc4c285b40977554",
                    "messageId": "11417859338547",
                    "operation":{
                        "opcode": "add",
                        "reaction": "👍"
                    },
                    "socialId": "1083545787011006465",
                    "_id": "689dd3667b679a515ae0466b",
                    "date": "2025-08-14T12:15:33.866Z"
                },
                "modifiedBy": "1083545787011006465",
                "modifiedOn": 1755173733877
            }"#,
        )
        .unwrap();
        assert_eq!(
            event,
            StreamingMessage {
                params: CommonParams {
                    id: "689dd365bd0103b355bb6064".to_string(),
                    space: "core:space:Tx".to_string(),
                    object_space: "core:space:Domain".to_string(),
                    modified_by: "1083545787011006465".to_string(),
                    modified_on: 1755173733877,
                },
                kind: StreamingMessageKind::Domain(DomainEventKind::Communication(
                    CommunicationDomainEventKind::ReactionPatch(ReactionPatch {
                        card_id: "685a65fefc4c285b40977554".to_string(),
                        message_id: "11417859338547".to_string(),
                        operation: ReactionPatchOperation {
                            opcode: "add".to_string(),
                            reaction: "👍".to_string(),
                        },
                        social_id: "1083545787011006465".to_string(),
                    })
                ))
            }
        );
    }
    #[test]
    fn test_deserialize_domain_event_thread_patch() {
        let attach_thread_event = serde_json::from_str::<StreamingMessage>(
            r#"{
                "_id": "68a4b49a2eab7e2ce6351589",
                "space": "core:space:Tx",
                "objectSpace": "core:space:Domain",
                "_class": "core:class:TxDomainEvent",
                "domain": "communication",
                "event": {
                    "type": "threadPatch",
                    "cardId": "68778a5f01da65d0472a371a",
                    "messageId": "11462932882058",
                    "operation": {
                        "opcode": "attach",
                        "threadId": "68a4b49a0b40986b3b3c11fc",
                        "threadType": "chat:masterTag:Thread"
                    },
                    "socialId": "1064398389519122433",
                    "_id": "68a4b49a0b40986b3b3c11fd",
                    "date": "2025-08-19T17:30:02.362Z"
                },
                "modifiedBy": "1064398389519122433",
                "modifiedOn": 1755624602371
            }"#,
        )
        .unwrap();
        assert_eq!(
            attach_thread_event,
            StreamingMessage {
                params: CommonParams {
                    id: "68a4b49a2eab7e2ce6351589".to_string(),
                    space: "core:space:Tx".to_string(),
                    object_space: "core:space:Domain".to_string(),
                    modified_by: "1064398389519122433".to_string(),
                    modified_on: 1755624602371
                },
                kind: StreamingMessageKind::Domain(DomainEventKind::Communication(
                    CommunicationDomainEventKind::ThreadPatch(ThreadPatch {
                        card_id: "68778a5f01da65d0472a371a".to_string(),
                        message_id: "11462932882058".to_string(),
                        social_id: "1064398389519122433".to_string(),
                        operation: ThreadPatchOperation::Attach(AttachThreadOperation {
                            thread_id: "68a4b49a0b40986b3b3c11fc".to_string(),
                            thread_type: "chat:masterTag:Thread".to_string(),
                        })
                    })
                ))
            }
        );
        let update_thread_event = serde_json::from_str::<StreamingMessage>(
            r#"{
                   "_id": "68a4b7c52eab7e2ce6351689",
                   "space": "core:space:Tx",
                   "objectSpace": "core:space:Domain",
                   "_class": "core:class:TxDomainEvent",
                   "domain": "communication",
                   "event": {
                       "type": "threadPatch",
                       "cardId": "68778a5f01da65d0472a371a",
                       "messageId": "11462932882058",
                       "operation": {
                           "opcode": "update",
                           "threadId": "68a4b49a0b40986b3b3c11fc",
                           "updates": {
                               "lastReply": "2025-08-19T17:43:33.012Z",
                               "repliesCountOp": "increment"
                           }
                       },
                       "socialId": "1064398389519122433",
                       "date": "2025-08-19T17:43:33.012Z"
                   },
                   "modifiedBy": "1064398389519122433",
                   "modifiedOn": 1755625413138
                }"#,
        )
        .unwrap();
        assert_eq!(
            update_thread_event,
            StreamingMessage {
                params: CommonParams {
                    id: "68a4b7c52eab7e2ce6351689".to_string(),
                    space: "core:space:Tx".to_string(),
                    object_space: "core:space:Domain".to_string(),
                    modified_by: "1064398389519122433".to_string(),
                    modified_on: 1755625413138
                },
                kind: StreamingMessageKind::Domain(DomainEventKind::Communication(
                    CommunicationDomainEventKind::ThreadPatch(ThreadPatch {
                        card_id: "68778a5f01da65d0472a371a".to_string(),
                        message_id: "11462932882058".to_string(),
                        social_id: "1064398389519122433".to_string(),
                        operation: ThreadPatchOperation::Update(UpdateThreadOperation {
                            thread_id: "68a4b49a0b40986b3b3c11fc".to_string(),
                            updates: UpdateThreadOperationUpdates {
                                replies_count_op: Some(ThreadRepliesCountOp::Increment),
                                last_reply: Some("2025-08-19T17:43:33.012Z".to_string()),
                                ..Default::default()
                            }
                        })
                    })
                ))
            }
        );
    }

    #[test]
    fn test_deserialize_unknown_event() {
        let event = serde_json::from_str::<StreamingMessage>(
            r#"{
                "_id": "685cce537a178870b1f19a30",
                "_class": "core:class:TxUpdateDoc",
                "space": "core:space:Tx",
                "modifiedBy": "1083545787011006465",
                "modifiedOn": 1750912595092,
                "objectId": "685a65fefc4c285b40977554",
                "objectClass": "chat:masterTag:Channel",
                "objectSpace": "card:space:Default",
                "operations": {},
                "retrieve": false,
                "createdOn": 1750912595092,
                "%hash%": "197aa85f494"
            }"#,
        )
        .unwrap();
        assert_eq!(
            event,
            StreamingMessage {
                params: CommonParams {
                    id: "685cce537a178870b1f19a30".to_string(),
                    space: "core:space:Tx".to_string(),
                    object_space: "card:space:Default".to_string(),
                    modified_by: "1083545787011006465".to_string(),
                    modified_on: 1750912595092,
                },
                kind: StreamingMessageKind::Unknown(serde_json::json!({
                    "_class": "core:class:TxUpdateDoc",
                    "objectId": "685a65fefc4c285b40977554",
                    "objectClass": "chat:masterTag:Channel",
                    "operations": {},
                    "retrieve": false,
                    "createdOn": 1750912595092i64,
                    "%hash%": "197aa85f494"
                }))
            }
        );
    }
}
