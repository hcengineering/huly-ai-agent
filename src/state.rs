use anyhow::Result;

use crate::{context::AgentContext, database::DbClient, task::Task, types::Message};

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

    pub async fn add_task_message(
        &mut self,
        context: &AgentContext,
        task: &Task,
        message: Message,
    ) -> Result<Message> {
        if let Some(channel_log_writer) = &context.channel_log_writer {
            channel_log_writer.trace_message(&message);
        }
        self.db_client.add_task_message(task, message).await
    }

    pub async fn update_task_messages(&mut self, task_id: i64, messages: &[Message]) {
        if let Err(err) = self.db_client.update_task_messages(task_id, messages).await {
            tracing::error!(?err, "Failed to update task messages");
        }
    }

    pub async fn set_task_done(&mut self, task_id: i64) -> Result<()> {
        self.db_client.set_task_done(task_id).await
    }
}
