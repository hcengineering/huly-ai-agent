// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use anyhow::Result;
use hulyrs::services::core::WorkspaceUuid;
use reqwest::{
    Client, Url,
    multipart::{Form, Part},
};
use secrecy::{ExposeSecret, SecretString};

use super::ServerConfig;

#[derive(Clone)]
pub struct BlobClient {
    upload_url: Url,
    token: SecretString,
    http: Client,
}

impl BlobClient {
    pub fn new(
        config: &ServerConfig,
        workspace: WorkspaceUuid,
        token: impl Into<SecretString>,
    ) -> Result<Self> {
        let base = Url::parse(
            &config
                .upload_url
                .replace(":workspace", workspace.to_string().as_str()),
        )?;

        let http = Client::new();
        Ok(Self {
            upload_url: base,
            token: token.into(),
            http,
        })
    }

    pub async fn upload_file(
        &self,
        blob_id: &str,
        mime_type: &str,
        content: Vec<u8>,
    ) -> Result<()> {
        tracing::debug!(
            %blob_id,
            %mime_type,
            "Uploading file"
        );
        let size = content.len();
        let file = Part::bytes(content)
            .file_name(blob_id.to_string())
            .mime_str(mime_type)?;

        let form = Form::new()
            .text("filename", blob_id.to_string())
            .text("contentType", mime_type.to_owned())
            .text("knownLength", size.to_string())
            .part("file", file);

        let request = self
            .http
            .post(self.upload_url.clone())
            .bearer_auth(self.token.expose_secret())
            .multipart(form);

        match request.send().await {
            Ok(response) => {
                if response.status().is_success() {
                    let _ = response.bytes().await?;
                    tracing::debug!("Uploading file successfully");
                    Ok(())
                } else {
                    tracing::error!(%blob_id,
                        status = %response.status(),
                        "Error status, while uploading file"
                    );
                    Err(anyhow::anyhow!(
                        "Error status={}, while uploading file",
                        response.status()
                    ))
                }
            }

            Err(error) => {
                tracing::error!(%blob_id, %error, "Error while uploading file");
                Err(anyhow::anyhow!(
                    "Error while uploading file={}, error: {}",
                    blob_id,
                    error
                ))
            }
        }
    }
}
