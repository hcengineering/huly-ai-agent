use serde::{Deserialize, Serialize};

use crate::task::Task;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub tasks: Vec<Task>,
    pub balance: u32,
}
