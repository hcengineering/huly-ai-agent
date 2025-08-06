// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

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

#[derive(Debug)]
pub struct ReceivedMessage {
    pub card_id: String,
    pub card_title: Option<String>,
    pub content: String,
    pub social_id: String,
    pub person_name: Option<String>,
    pub person_id: Option<String>,
    pub message_id: String,
    pub date: String,
}

impl From<CreateMessage> for ReceivedMessage {
    fn from(value: CreateMessage) -> Self {
        ReceivedMessage {
            card_id: value.card_id,
            card_title: None,
            content: value.content,
            social_id: value.social_id,
            person_name: None,
            person_id: None,
            message_id: value.message_id,
            date: value.date,
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
