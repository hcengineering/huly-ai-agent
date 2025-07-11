// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    config::Config,
    context::AgentContext,
    state::AgentState,
    tools::{ToolImpl, ToolSet},
};

macro_rules! create_mem_tool {
    ($func_name:ident, $tool_name:ident) => {
        paste::paste! {
            struct [<$func_name Tool>] {
                state: AgentState,
            }

            #[async_trait]
            impl ToolImpl for [<$func_name Tool>] {
                fn name(&self) -> &str {
                    stringify!($tool_name)
                }

                async fn call(&mut self, args: serde_json::Value) -> Result<String> {
                    let name = self.name().to_string();
                    call_memory_tool(&mut self.state, &name, args).await
                }
            }
        }
    };
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    #[serde(rename = "entityName")]
    pub entity_name: String,
    pub observations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AddObservationsResult {
    #[serde(rename = "entityName")]
    entity_name: String,
    #[serde(rename = "addedObservations")]
    added_observations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entity {
    #[serde(skip)]
    pub id: i64,
    pub name: String,
    #[serde(rename = "entityType")]
    pub entity_type: String,
    pub observations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub from: String,
    pub to: String,
    #[serde(rename = "relationType")]
    pub relation_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeGraph {
    pub entities: Vec<Entity>,
    pub relations: Vec<Relation>,
}

pub struct MemoryToolSet;

async fn call_memory_tool(
    state: &mut AgentState,
    toolname: &str,
    args: serde_json::Value,
) -> Result<String> {
    match toolname {
        "create_entities" => {
            let mut entities: Vec<Entity> = serde_json::from_value(args["entities"].clone())?;
            entities = state.mem_add_entities(&mut entities).await?;
            Ok(serde_json::to_string_pretty(&entities)?)
        }
        "create_relations" => {
            let mut relations: Vec<Relation> = serde_json::from_value(args["relations"].clone())?;
            relations = state.mem_add_relations(&mut relations).await?;
            Ok(serde_json::to_string_pretty(&relations)?)
        }
        "add_observations" => {
            let observations: Vec<Observation> =
                serde_json::from_value(args["observations"].clone())?;

            let added_observations = state.mem_add_observations(observations).await?;

            let result = added_observations
                .into_iter()
                .map(|observation| AddObservationsResult {
                    entity_name: observation.entity_name,
                    added_observations: observation.observations,
                })
                .collect::<Vec<_>>();
            Ok(serde_json::to_string_pretty(&result)?)
        }
        "delete_entities" => {
            let entity_names: Vec<String> = serde_json::from_value(args["entityNames"].clone())?;
            state.mem_delete_entities(&entity_names).await?;
            Ok("Entities deleted successfully".to_string())
        }
        "delete_observations" => {
            let observations: Vec<Observation> = serde_json::from_value(args["deletions"].clone())?;
            state.mem_delete_observations(&observations).await?;
            Ok("Observations deleted successfully".to_string())
        }
        "delete_relations" => {
            let relations: Vec<Relation> = serde_json::from_value(args["relations"].clone())?;
            state.mem_delete_relations(&relations).await?;
            Ok("Relations deleted successfully".to_string())
        }
        "read_graph" => {
            let result = state.mem_search_nodes(None).await?;
            Ok(serde_json::to_string(&result).unwrap())
        }
        "search_nodes" => {
            let query = args["query"].as_str().unwrap();
            let result = state.mem_search_nodes(Some(query)).await?;
            Ok(serde_json::to_string(&result).unwrap())
        }
        "open_nodes" => {
            let names: Vec<String> = serde_json::from_value(args["names"].clone())?;
            let entities: Vec<Entity> = state.mem_list_entities(&names).await?;
            Ok(serde_json::to_string(&entities).unwrap())
        }
        _ => Err(anyhow::anyhow!("Unknown tool: {}", toolname)),
    }
}

create_mem_tool!(MemoryCreateEntities, create_entities);
create_mem_tool!(MemoryCreateRelations, create_relations);
create_mem_tool!(MemoryAddObservations, add_observations);
create_mem_tool!(MemoryDeleteEntities, delete_entities);
create_mem_tool!(MemoryDeleteObservations, delete_observations);
create_mem_tool!(MemoryDeleteRelations, delete_relations);
create_mem_tool!(MemoryReadGraph, read_graph);
create_mem_tool!(MemorySearchNodes, search_nodes);
create_mem_tool!(MemoryOpenNodes, open_nodes);

impl ToolSet for MemoryToolSet {
    fn get_tools<'a>(
        _config: &'a Config,
        _context: &'a AgentContext,
        state: &'a AgentState,
    ) -> Vec<Box<dyn ToolImpl>> {
        vec![
            Box::new(MemoryCreateEntitiesTool {
                state: state.clone(),
            }),
            Box::new(MemoryCreateRelationsTool {
                state: state.clone(),
            }),
            Box::new(MemoryAddObservationsTool {
                state: state.clone(),
            }),
            Box::new(MemoryDeleteEntitiesTool {
                state: state.clone(),
            }),
            Box::new(MemoryDeleteObservationsTool {
                state: state.clone(),
            }),
            Box::new(MemoryDeleteRelationsTool {
                state: state.clone(),
            }),
            Box::new(MemorySearchNodesTool {
                state: state.clone(),
            }),
            Box::new(MemoryOpenNodesTool {
                state: state.clone(),
            }),
            Box::new(MemoryReadGraphTool {
                state: state.clone(),
            }),
        ]
    }

    fn get_tool_descriptions(_config: &Config) -> Vec<serde_json::Value> {
        serde_json::from_str(include_str!("tools.json")).unwrap()
    }

    fn get_system_prompt(_config: &Config) -> String {
        include_str!("system_prompt.txt").to_string()
    }
}
