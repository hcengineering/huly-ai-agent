// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use anyhow::Result;
use hulyrs::services::core::WorkspaceUuid;
use reqwest::{
    Client, Url,
    header::{self, HeaderMap, HeaderValue},
    multipart::{Form, Part},
};
use secrecy::{ExposeSecret, SecretString};

use super::ServerConfig;

#[derive(Clone)]
enum LakeProvider {
    Hulylake(Url),
    Datalake(Url),
}
#[derive(Clone)]
pub struct BlobClient {
    upload_url: LakeProvider,
    token: SecretString,
    http: Client,
}

impl BlobClient {
    pub fn new(
        config: &ServerConfig,
        workspace: WorkspaceUuid,
        token: impl Into<SecretString>,
    ) -> Result<Self> {
        let upload_url = if let Some(url) = &config.datalake_url {
            LakeProvider::Datalake(Url::parse(&format!("{url}/upload/form-data/{workspace}",))?)
        } else if let Some(url) = &config.hulylake_url {
            LakeProvider::Hulylake(Url::parse(&format!("{url}/api/${workspace}",))?)
        } else {
            anyhow::bail!("Hulylake URL is not configured")
        };

        let http = Client::new();
        Ok(Self {
            upload_url,
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
        let request = match &self.upload_url {
            LakeProvider::Datalake(url) => {
                let file = Part::bytes(content)
                    .file_name(blob_id.to_string())
                    .mime_str(mime_type)?;
                let form = Form::new()
                    .text("filename", blob_id.to_string())
                    .text("contentType", mime_type.to_owned())
                    .text("knownLength", size.to_string())
                    .part("file", file);

                self.http
                    .post(url.clone())
                    .bearer_auth(self.token.expose_secret())
                    .multipart(form)
            }
            LakeProvider::Hulylake(url) => self
                .http
                .put(url.clone().join(blob_id)?)
                .headers(HeaderMap::from_iter(vec![
                    (header::CONTENT_TYPE, HeaderValue::from_str(mime_type)?),
                    (
                        header::CONTENT_LENGTH,
                        HeaderValue::from_str(&size.to_string())?,
                    ),
                ]))
                .body(content)
                .bearer_auth(self.token.expose_secret()),
        };

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
