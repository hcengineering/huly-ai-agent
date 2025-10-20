// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use anyhow::Result;
use hulyrs::services::{
    core::classes::Ref,
    pulse::{Expiration, PulseClient, PutMode},
};
use serde::Serialize;

pub struct TypingClient {
    client: PulseClient,
    social_id: Ref,
}

pub const THINKING_KEY: &str = "communication:string:IsThinking";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TypingInfo {
    social_id: Ref,
    object_id: Ref,
    status: Option<String>,
}

impl TypingClient {
    pub fn new(client: PulseClient, social_id: &str) -> Self {
        Self {
            client,
            social_id: social_id.to_string(),
        }
    }

    pub async fn set_typing(
        &self,
        object_id: &str,
        status: Option<String>,
        seconds: u64,
    ) -> Result<()> {
        tracing::debug!("Set typing for {}, status: {:?}", object_id, status);
        let key = format!("typing/{object_id}/{}", &self.social_id);
        let status = if status.as_ref().is_some_and(|s| s != THINKING_KEY) {
            Some(format!("embedded:embedded:{}", status.unwrap()))
        } else {
            status
        };
        let info = TypingInfo {
            social_id: self.social_id.clone(),
            status,
            object_id: object_id.to_string(),
        };
        self.client
            .put(
                &key,
                serde_json::to_string(&info)?,
                Some(Expiration::InSeconds(seconds)),
                PutMode::Upsert,
            )
            .await?;
        Ok(())
    }

    pub async fn reset_typing(&self, object_id: &str) -> Result<()> {
        self.client
            .delete(
                &format!("typing/{object_id}/{}", &self.social_id),
                PutMode::Upsert,
            )
            .await?;
        Ok(())
    }
}
