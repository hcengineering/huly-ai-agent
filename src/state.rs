use anyhow::Result;
use itertools::Itertools;

use crate::{
    database::DbClient,
    task::{Task, TaskState},
    types::Message,
};

const MAX_ASSISTANT_MESSAGES: usize = 20;

#[derive(Debug, Clone)]
pub struct AgentState {
    db_client: DbClient,
    balance: u32,
}

impl AgentState {
    pub async fn new(db_client: DbClient) -> Result<Self> {
        let balance = db_client.balance().await?;
        Ok(Self { db_client, balance })
    }

    pub fn balance(&self) -> u32 {
        self.balance
    }

    pub async fn set_balance(&mut self, balance: u32) -> Result<()> {
        self.db_client.set_balance(balance).await?;
        self.balance = balance;
        Ok(())
    }

    pub async fn task_messages(&self, task_id: i64) -> Result<Vec<Message>> {
        self.db_client.task_messages(task_id).await
    }

    pub async fn add_task_message(&mut self, task: &Task, message: Message) -> Result<Message> {
        tracing::trace!(message = ?message, "message");
        self.db_client.add_task_message(task, message).await
    }

    pub async fn update_task_messages(&mut self, task_id: i64, messages: &[Message]) {
        if let Err(err) = self.db_client.update_task_messages(task_id, messages).await {
            tracing::error!(?err, "Failed to update task messages");
        }
    }

    pub async fn update_task_complexity(
        &mut self,
        task: &mut Task,
        result_content: &str,
    ) -> Option<u32> {
        if result_content.starts_with("<complexity>")
            && let Some(Some(complexity)) = result_content
                .split("</complexity>")
                .nth(0)
                .map(|s| s[12..].trim().parse::<u32>().ok())
        {
            if let Err(err) = self
                .db_client
                .set_task_complexity(task.id, complexity)
                .await
            {
                tracing::error!(?err, "Failed to set task complexity");
            }
            task.complexity = complexity;
            return Some(complexity);
        }
        None
    }

    pub async fn set_task_state(&mut self, task_id: i64, state: TaskState) -> Result<()> {
        self.db_client.set_task_state(task_id, state).await
    }

    pub async fn get_assistant_messages(&self, card_id: &str) -> Result<Vec<Message>> {
        Ok(serde_json::from_str(
            &self.db_client.get_assistant_messages(card_id).await,
        )?)
    }

    pub async fn set_assistant_messages(&self, card_id: &str, messages: &[Message]) -> Result<()> {
        if messages.len() > MAX_ASSISTANT_MESSAGES {
            let _: () = self
                .db_client
                .set_assistant_messages(
                    card_id,
                    serde_json::to_string(
                        &messages
                            .iter()
                            .skip(messages.len() - MAX_ASSISTANT_MESSAGES)
                            .collect_vec(),
                    )?,
                )
                .await;
            Ok(())
        } else {
            let _: () = self
                .db_client
                .set_assistant_messages(card_id, serde_json::to_string(messages)?)
                .await;
            Ok(())
        }
    }
}
