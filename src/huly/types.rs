// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use hulyrs::services::{
    core::{
        SocialIdType,
        classes::{AttachedDoc, Ref, Timestamp},
    },
    event::{Class, HasId},
    transactor::tx::Doc,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AvatarType {
    Color,
    Image,
    Gravatar,
    External,
    #[serde(untagged)]
    Unknown(String),
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AvatarProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    #[serde(flatten)]
    pub doc: Doc,

    pub name: String,

    pub avatar_type: AvatarType,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<Ref>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_props: Option<AvatarProps>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
}

impl Class for Contact {
    const CLASS: &'static str = "contact:class:Contact";
}

impl HasId for Contact {
    fn id(&self) -> &str {
        &self.doc.id
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    #[serde(flatten)]
    pub contact: Contact,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub person_uuid: Option<Uuid>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub birthday: Option<Timestamp>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub social_ids: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Ref>,
}

impl Class for Person {
    const CLASS: &'static str = "contact:class:Person";
}

impl HasId for Person {
    fn id(&self) -> &str {
        self.contact.id()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SocialIdentity {
    #[serde(flatten)]
    pub doc: AttachedDoc,
    pub key: String,
    pub r#type: SocialIdType,
    pub value: String,
    #[serde(with = "chrono::serde::ts_milliseconds_option")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_on: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_deleted: Option<bool>,
}

impl Class for SocialIdentity {
    const CLASS: &'static str = "contact:class:SocialIdentity";
}

impl HasId for SocialIdentity {
    fn id(&self) -> &str {
        &self.doc.doc.id
    }
}
