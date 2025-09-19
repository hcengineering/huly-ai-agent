// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use anyhow::Result;
use hulyrs::services::{
    core::classes::Ref,
    pulse::{Expiration, PulseClient, PutMode},
};
use serde::Serialize;

pub struct TypingClient {
    client: PulseClient,
    person_id: Ref,
}

#[derive(Serialize)]
struct TypingInfo {
    person_id: Ref,
    object_id: Ref,
}

impl TypingClient {
    pub fn new(client: PulseClient, person_id: &str) -> Self {
        Self {
            client,
            person_id: person_id.to_string(),
        }
    }

    pub async fn set_typing(&self, object_id: &str, seconds: u64) -> Result<()> {
        let key = format!("{object_id}/{}", &self.person_id);
        let info = TypingInfo {
            person_id: self.person_id.clone(),
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
            .delete(&format!("{object_id}/{}", &self.person_id), PutMode::Upsert)
            .await?;
        Ok(())
    }
}
