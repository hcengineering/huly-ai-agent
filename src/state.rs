use anyhow::Result;

use crate::{context::AgentContext, database::DbClient, task::Task, types::Message};

#[derive(Debug, Clone)]
pub struct AgentState {
    db_client: DbClient,
    has_new_tasks: bool,
    balance: u32,
}

impl AgentState {
    pub async fn new(db_client: DbClient) -> Result<Self> {
        let balance = db_client.balance().await?;
        Ok(Self {
            db_client,
            balance,
            has_new_tasks: true,
        })
    }

    pub async fn latest_task(&mut self) -> Result<Option<Task>> {
        if !self.has_new_tasks {
            return Ok(None);
        }
        let task = self.db_client.latest_task().await?;
        if task.is_none() {
            self.has_new_tasks = false;
        }
        Ok(task)
    }

    pub fn balance(&self) -> u32 {
        self.balance
    }

    pub async fn set_balance(&mut self, balance: u32) -> Result<()> {
        self.db_client.set_balance(balance).await?;
        self.balance = balance;
        Ok(())
    }

    pub async fn add_task(&mut self, task: Task) -> Result<()> {
        self.db_client.add_task(task).await?;
        self.has_new_tasks = true;
        Ok(())
    }

    pub async fn task_messages(&self, task_id: i64) -> Result<Vec<Message>> {
        self.db_client.task_messages(task_id).await
    }

    pub async fn add_task_message(
        &mut self,
        context: &AgentContext,
        task: &mut Task,
        message: Message,
    ) -> Result<Message> {
        if let Some(channel_log_writer) = &context.channel_log_writer {
            channel_log_writer.trace_message(&message);
        }
        self.db_client.add_task_message(task, message).await
    }

    pub async fn set_task_done(&mut self, task_id: i64) -> Result<()> {
        self.db_client.set_task_done(task_id).await
    }
}
