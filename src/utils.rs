// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::path::Path;

use hulyrs::services::{
    event::Class,
    transactor::{
        TransactorClient,
        backend::http::HttpBackend,
        document::{DocumentClient, FindOptionsBuilder},
    },
};
use serde_json::json;

use crate::huly::types::CommunicationDirect;

pub fn safe_truncated(s: &str, len: usize) -> String {
    let mut new_len = usize::min(len, s.len());
    let mut s = s.to_string();
    while !s.is_char_boundary(new_len) {
        new_len -= 1;
    }
    s.truncate(new_len);
    s
}

#[inline]
pub fn workspace_to_string(workspace: &Path) -> String {
    workspace.to_str().unwrap().to_string().replace("\\", "/")
}

pub fn normalize_path(workspace: &Path, path: &str) -> String {
    let path = path.to_string().replace("\\", "/");
    let workspace = workspace_to_string(workspace);
    if !path.starts_with(&workspace) {
        format!("{workspace}/{path}")
    } else {
        path
    }
}

pub async fn get_control_card_id(tx_client: TransactorClient<HttpBackend>) -> Option<String> {
    tracing::debug!("Get control card id");
    tx_client
        .find_all::<_, CommunicationDirect>(
            CommunicationDirect::CLASS,
            json!({}),
            &FindOptionsBuilder::default().build(),
        )
        .await
        .ok()?
        .value
        .iter()
        .find_map(|card| {
            if card.members.len() == 1 {
                Some(card.doc.id.clone())
            } else {
                None
            }
        })
}
