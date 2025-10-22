use std::{
    collections::HashMap,
    fmt::Display,
    time::{Duration, Instant},
};

use anyhow::Result;
use hulyrs::services::transactor::{TransactorClient, backend::http::HttpBackend};
use indexmap::IndexMap;
use itertools::Itertools;
use tokio::{select, sync::mpsc};
use tokio_util::sync::CancellationToken;

use crate::{
    HulyAccountInfo,
    communication::types::{CommunicationEvent, ReceivedMessage},
    config::{AgentMode, Config, JobSchedule, RgbRole},
    context::AgentContext,
    types::Message,
    utils,
};

pub const MAX_FOLLOW_MESSAGES: u8 = 10;
pub const TASK_START_DELAY: Duration = Duration::from_secs(1);
pub const TASK_DEFAULT_COMPLEXITY: u32 = 10;
pub const CHECK_CONTROL_CARD_INTERVAL: Duration = Duration::from_secs(5 * 60); // 5 mins

#[derive(Debug, Clone)]
pub struct Task {
    pub id: i64,
    pub kind: TaskKind,
    pub complexity: u32,
    #[allow(dead_code)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[allow(dead_code)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[allow(dead_code)]
    pub state: TaskState,
    pub cancel_token: CancellationToken,
}

#[derive(Debug, Clone, Default)]
#[repr(u8)]
pub enum TaskState {
    #[default]
    Created = 0,
    Started = 1,
    Completed = 2,
    Cancelled = 3,
    Postponed = 4,
}

impl TaskState {
    pub fn from_i64(value: i64) -> Self {
        match value {
            0 => TaskState::Created,
            1 => TaskState::Postponed,
            2 => TaskState::Started,
            3 => TaskState::Completed,
            4 => TaskState::Cancelled,
            _ => TaskState::Cancelled,
        }
    }
}

#[derive(Debug)]
pub struct ScheduledAssistantTask {
    pub id: i64,
    pub content: String,
    pub schedule: JobSchedule,
}

impl Task {
    pub fn new(kind: TaskKind) -> Self {
        Self {
            id: 0,
            kind,
            state: Default::default(),
            complexity: TASK_DEFAULT_COMPLEXITY,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            cancel_token: CancellationToken::new(),
        }
    }
}

impl TaskKind {
    fn rgb_role(&self, config: &Config) -> Option<RgbRole> {
        match self {
            TaskKind::FollowChat { content, .. } => {
                if config.huly.person.as_ref().is_some_and(|p| {
                    p.rgb_opponents
                        .iter()
                        .all(|(person_id, _)| content.contains(person_id))
                }) {
                    config.huly.person.as_ref().map(|p| p.rgb_role.clone())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn system_prompt(&self, config: &Config) -> String {
        match self {
            TaskKind::FollowChat { .. } => {
                if let AgentMode::Employee(_) = config.agent_mode {
                    if let Some(role) = self.rgb_role(config) {
                        let rbg_prompt = match role {
                            RgbRole::Red => include_str!("templates/rgb_protocol/red.md"),
                            RgbRole::Green => include_str!("templates/rgb_protocol/green.md"),
                            RgbRole::Blue => include_str!("templates/rgb_protocol/blue.md"),
                        };
                        format!(
                            "{}\n\n{}",
                            include_str!("templates/tasks/follow_chat/system_prompt_employee.md"),
                            rbg_prompt
                        )
                    } else {
                        include_str!("templates/tasks/follow_chat/system_prompt_employee.md")
                            .to_string()
                    }
                } else {
                    include_str!("templates/tasks/follow_chat/system_prompt_assistant.md")
                        .to_string()
                }
            }
            TaskKind::AssistantChat { .. } => {
                include_str!("templates/tasks/assistant_chat/system_prompt.md").to_string()
            }
            TaskKind::Sleep => include_str!("templates/tasks/sleep/system_prompt.md").to_string(),
            TaskKind::AssistantTask { .. } => {
                include_str!("templates/tasks/assistant_task/system_prompt.md").to_string()
            }
            _ => String::new(),
        }
    }

    pub fn context(&self, config: &Config, agent_context: &AgentContext) -> String {
        match self {
            TaskKind::FollowChat { .. } => {
                let mut context =
                    include_str!("templates/tasks/follow_chat/context.md").to_string();
                if self.rgb_role(config).is_some() {
                    context = format!("${{RGB_ROLES}}\n{context}");
                }
                context
            }
            TaskKind::AssistantChat { card_id, .. } => {
                include_str!("templates/tasks/assistant_chat/context.md")
                    .replace("${CARD_ID}", &format!("Conversation Card Id: {card_id}"))
                    .to_string()
            }
            TaskKind::AssistantTask {
                sheduled_task_id, ..
            } => {
                let card_id = agent_context
                    .account_info
                    .control_card_id
                    .clone()
                    .unwrap_or("".to_string());
                include_str!("templates/tasks/assistant_task/context.md")
                    .replace("${CARD_ID}", &format!("Conversation Card Id: {card_id}"))
                    .replace("${TASK_ID}", &sheduled_task_id.to_string())
                    .to_string()
            }
            _ => String::new(),
        }
    }
}
pub struct Attachment {
    pub file_name: String,
    pub url: String,
}

pub struct Reaction {
    pub person: String,
    pub reaction: String,
}

pub struct CardMessage {
    pub message_id: String,
    pub person_info: String,
    pub date: String,
    pub content: String,
    pub attachments: Vec<Attachment>,
    pub reactions: Vec<Reaction>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TaskKind {
    FollowChat {
        card_id: String,
        card_title: String,
        message_id: String,
        content: String,
    },
    AssistantTask {
        sheduled_task_id: i64,
        content: String,
    },
    AssistantChat {
        card_id: String,
        message_id: String,
        content: String,
    },
    MemoryMantainance,
    Sleep,
}

impl Display for TaskKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            TaskKind::FollowChat { .. } => "follow_chat",
            TaskKind::AssistantTask { .. } => "assistant_task",
            TaskKind::AssistantChat { .. } => "assistant_chat",
            TaskKind::MemoryMantainance => "memory_mantainance",
            TaskKind::Sleep => "sleep",
        };
        f.write_str(name)
    }
}

impl TaskKind {
    pub fn to_message(&self) -> Message {
        match self {
            TaskKind::FollowChat {
                card_id,
                card_title,
                content,
                ..
            } => Message::user(&format!(
                "|follow_chat|card:[{card_title}]({card_id})|chat_log:{content}"
            )),
            TaskKind::AssistantTask { content, .. } => {
                Message::user(&format!("|assistant_task|{content}"))
            }
            TaskKind::AssistantChat { .. } => Message::user("|assistant_chat|"),
            TaskKind::MemoryMantainance => Message::user("|memory_mantainance|"),
            TaskKind::Sleep => Message::user("|sleep|"),
        }
    }

    pub fn can_skip(&self, other: &Self) -> bool {
        match (self, other) {
            (
                TaskKind::FollowChat { card_id: id1, .. },
                TaskKind::FollowChat { card_id: id2, .. },
            ) => id1 == id2,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TaskFinishReason {
    Completed,
    Skipped,
    Cancelled,
}

fn format_messages<'a>(messages: impl IntoIterator<Item = &'a CardMessage>) -> String {
    messages
        .into_iter()
        .map(|m| {
            let attachements_block = if m.attachments.is_empty() {
                "".to_string()
            } else {
                format!(
                    "\n- attachments\n{}",
                    m.attachments
                        .iter()
                        .map(|a| format!("  - [{}]({})", a.file_name, a.url))
                        .join("\n")
                )
            };
            let reactions_block = if m.reactions.is_empty() {
                "".to_string()
            } else {
                format!(
                    "\n- reactions\n{}",
                    m.reactions
                        .iter()
                        .map(|r| format!("  - {}|{}", r.person, r.reaction))
                        .join("\n")
                )
            };

            format!(
                "{}|{} _{}_:\n{}{}{}",
                m.message_id, m.person_info, m.date, m.content, attachements_block, reactions_block
            )
        })
        .join("\n\n")
}

async fn process_incoming_event(
    receiver: &mut mpsc::UnboundedReceiver<CommunicationEvent>,
    card_messages: &mut HashMap<String, IndexMap<String, CardMessage>>,
    social_id: &str,
) -> (bool, Option<ReceivedMessage>) {
    let Some(event) = receiver.recv().await else {
        return (false, None);
    };
    tracing::debug!("Received event: {:?}", event);
    match event {
        CommunicationEvent::Reaction(reaction) => {
            if let Some(messages) = card_messages.get_mut(&reaction.card_id)
                && let Some(message) = messages.get_mut(&reaction.message_id)
            {
                message.reactions.push(Reaction {
                    person: reaction.person,
                    reaction: reaction.reaction,
                });
            }
            return (true, None);
        }
        CommunicationEvent::Attachment(attachement) => {
            if let Some(messages) = card_messages.get_mut(&attachement.card_id)
                && let Some(message) = messages.get_mut(&attachement.message_id)
            {
                message.attachments.push(Attachment {
                    file_name: attachement.file_name,
                    url: attachement.url,
                });
            }
            return (true, None);
        }
        _ => {}
    }

    let CommunicationEvent::Message(new_message) = event else {
        return (true, None);
    };
    card_messages
        .entry(new_message.card_id.clone())
        .or_default()
        .insert(
            new_message.message_id.clone(),
            CardMessage {
                message_id: new_message.message_id.clone(),
                person_info: new_message.person_info.to_string(),
                date: new_message.date.clone(),
                content: new_message.content.clone(),
                attachments: vec![],
                reactions: vec![],
            },
        );

    // skip messages from the same social_id for follow mode
    if new_message.social_id == social_id {
        return (true, None);
    }
    (true, Some(new_message))
}

pub async fn task_multiplexer(
    mut receiver: mpsc::UnboundedReceiver<CommunicationEvent>,
    sender: mpsc::UnboundedSender<Task>,
    agent_mode: AgentMode,
    account_info: HulyAccountInfo,
    tx_client: TransactorClient<HttpBackend>,
) -> Result<()> {
    tracing::debug!("Start task multiplexer");
    let mut last_check_control_card = Instant::now();
    let mut control_card_id = account_info.control_card_id.clone();

    let mut card_messages = HashMap::<String, IndexMap<String, CardMessage>>::new();
    let mut waiting_messages = IndexMap::<String, (ReceivedMessage, Instant)>::new();

    let mut delay = Duration::from_secs(u64::MAX);
    let recalculate_delay = |waiting_messages: &IndexMap<String, (ReceivedMessage, Instant)>| {
        let now = Instant::now();
        waiting_messages
            .iter()
            .map(|(_, (_, time))| time)
            .min()
            .map_or(Duration::from_secs(u64::MAX), |time| {
                time.duration_since(now)
            })
    };
    loop {
        select! {
            (should_continue, new_message) = process_incoming_event(&mut receiver, &mut card_messages, &account_info.social_id) => {
                if !should_continue {
                    break;
                }
                if let Some(new_message) = new_message && !waiting_messages.contains_key(&new_message.card_id) {
                    waiting_messages.insert(new_message.card_id.clone(), (new_message, Instant::now().checked_add(TASK_START_DELAY).unwrap()));
                }
                delay = recalculate_delay(&waiting_messages);
            },
            _ = tokio::time::sleep(delay) => {
                let now = Instant::now();
                let mut check_control_card = false;
                waiting_messages.retain(|_, (message, time)| if *time > now {
                    true
                } else {
                    match agent_mode {
                        AgentMode::Employee(_) => {
                            let messages = card_messages.get(&message.card_id).unwrap();
                            sender.send(Task::new(TaskKind::FollowChat {
                                card_id: message.card_id.clone(),
                                card_title: message.card_title.clone().unwrap_or_default(),
                                message_id: message.message_id.clone(),
                                content: format_messages(
                                    messages.values(),
                                ),
                            })).unwrap();
                            if messages.len() > MAX_FOLLOW_MESSAGES as usize {
                                card_messages.remove(&message.card_id);
                            }
                        }
                        AgentMode::PersonalAssistant(_) => {
                            if control_card_id.is_none() && now.saturating_duration_since(last_check_control_card) > CHECK_CONTROL_CARD_INTERVAL {
                                check_control_card = true;
                                last_check_control_card = now;
                            }

                            if let Some(control_card_id) = &control_card_id
                                && (message.card_id == *control_card_id || message.card_id.starts_with(&format!("{control_card_id}_"))) {
                                sender.send(Task::new(TaskKind::AssistantChat {
                                    card_id: message.card_id.clone(),
                                    message_id: message.message_id.clone(),
                                    content: message.content.clone()
                                })).unwrap();
                            } else {
                                let messages = card_messages.get(&message.card_id).unwrap();
                                sender.send(Task::new(TaskKind::FollowChat {
                                    card_id: message.card_id.clone(),
                                    card_title: message.card_title.clone().unwrap_or_default(),
                                    message_id: message.message_id.clone(),
                                    content: format_messages(
                                        messages.values(),
                                    ),
                                })).unwrap();
                                if messages.len() > MAX_FOLLOW_MESSAGES as usize {
                                    card_messages.remove(&message.card_id);
                                }
                            }
                        }
                    }
                    false
                });
                if check_control_card {
                    control_card_id = utils::get_control_card_id(tx_client.clone()).await;
                }
                delay = recalculate_delay(&waiting_messages);
            },
        }
    }
    Ok(())
}
