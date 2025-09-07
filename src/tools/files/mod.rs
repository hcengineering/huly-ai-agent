// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{
    collections::HashMap,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

use anyhow::Result;
use async_trait::async_trait;
use grep_printer::StandardBuilder;
use grep_regex::RegexMatcher;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use serde::Deserialize;

use crate::{
    config::Config,
    context::AgentContext,
    state::AgentState,
    tools::{ToolImpl, ToolSet},
    types::ToolResultContent,
    utils::{normalize_path, workspace_to_string},
};

pub struct FilesToolSet;

impl ToolSet for FilesToolSet {
    fn get_name(&self) -> &str {
        "fs"
    }

    async fn get_tools<'a>(
        &self,
        config: &'a Config,
        _context: &'a AgentContext,
        _state: &'a AgentState,
    ) -> Vec<Box<dyn ToolImpl>> {
        let mut descriptions = serde_json::from_str::<Vec<serde_json::Value>>(
            &include_str!("tools.json")
                .replace("${WORKSPACE}", &workspace_to_string(&config.workspace)),
        )
        .unwrap()
        .into_iter()
        .map(|v| (v["function"]["name"].as_str().unwrap().to_string(), v))
        .collect::<HashMap<String, serde_json::Value>>();
        vec![
            Box::new(ReadFileTool {
                workspace: config.workspace.clone(),
                description: descriptions.remove("fs_read").unwrap(),
            }),
            Box::new(WriteToFileTool {
                workspace: config.workspace.clone(),
                description: descriptions.remove("fs_write").unwrap(),
            }),
            Box::new(ListFilesTool {
                workspace: config.workspace.clone(),
                description: descriptions.remove("fs_list").unwrap(),
            }),
            Box::new(ReplaceInFileTool {
                workspace: config.workspace.clone(),
                description: descriptions.remove("fs_replace").unwrap(),
            }),
            Box::new(SearchFilesTool {
                workspace: config.workspace.clone(),
                description: descriptions.remove("fs_search").unwrap(),
            }),
        ]
    }

    fn get_system_prompt(&self, _config: &Config) -> String {
        include_str!("system_prompt.txt").to_string()
    }
}

fn create_patch(original: &str, modified: &str) -> String {
    diffy::create_patch(original, modified)
        .to_string()
        .lines()
        .skip(2)
        .collect::<String>()
}

struct ReadFileTool {
    workspace: PathBuf,
    description: serde_json::Value,
}

#[derive(Deserialize)]
struct ReadFileToolArgs {
    path: String,
}

#[async_trait]
impl ToolImpl for ReadFileTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(&mut self, args: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<ReadFileToolArgs>(args)?;
        let path = normalize_path(&self.workspace, &args.path);
        tracing::info!("Reading file {}", path);
        Ok(vec![ToolResultContent::text(fs::read_to_string(path)?)])
    }
}

#[derive(Deserialize)]
struct WriteToFileToolArgs {
    pub path: String,
    pub content: String,
}

struct WriteToFileTool {
    pub workspace: PathBuf,
    description: serde_json::Value,
}

#[async_trait]
impl ToolImpl for WriteToFileTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(&mut self, args: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<WriteToFileToolArgs>(args)?;
        let path = normalize_path(&self.workspace, &args.path);
        tracing::info!("Write to file '{}'", path);
        let diff = create_patch("", &args.content);
        fs::create_dir_all(Path::new(&path).parent().unwrap())?;
        fs::write(path, args.content)?;
        Ok(vec![ToolResultContent::text(format!(
            "The user made the following updates to your content:\n\n{diff}"
        ))])
    }
}

#[derive(Deserialize)]
struct ListFilesToolArgs {
    pub path: String,
    pub max_depth: Option<usize>,
}

struct ListFilesTool {
    workspace: PathBuf,
    description: serde_json::Value,
}

#[async_trait]
impl ToolImpl for ListFilesTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(&mut self, args: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<ListFilesToolArgs>(args)?;
        let path = normalize_path(&self.workspace, &args.path);
        let max_depth = args.max_depth.unwrap_or(1);
        let mut files: Vec<String> = Vec::default();
        for entry in ignore::WalkBuilder::new(path.clone())
            .max_depth(Some(max_depth))
            .filter_entry(|e| e.file_name() != "node_modules")
            .build()
            .filter_map(|e| e.ok())
        {
            files.push(
                entry
                    .path()
                    .strip_prefix(&path)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace("\\", "/")
                    .to_string(),
            );
        }
        let res = files.join("\n");
        Ok(vec![ToolResultContent::text(if res.is_empty() {
            "No results found".to_string()
        } else {
            res
        })])
    }
}

#[derive(Deserialize)]
struct ReplaceInFileToolArgs {
    pub path: String,
    pub diff: String,
}

struct ReplaceInFileTool {
    workspace: PathBuf,
    description: serde_json::Value,
}

#[async_trait]
impl ToolImpl for ReplaceInFileTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(&mut self, args: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<ReplaceInFileToolArgs>(args)?;
        let path = normalize_path(&self.workspace, &args.path);
        tracing::info!("Replace in file '{}'", path);
        let replace_diffs = parse_replace_diff(&args.diff)?;
        let original_content = fs::read_to_string(path.clone())?;
        let mut modified_content = original_content.clone();
        for replace_diff in replace_diffs {
            let search = &replace_diff.search;
            let replace = &replace_diff.replace;
            let start = original_content.find(search);
            if let Some(start) = start {
                let end = start + search.len();
                modified_content.replace_range(start..end, replace);
            } else {
                anyhow::bail!(format!("Search string not found: {}", search));
            }
        }
        let diff = create_patch(&original_content, &modified_content);
        fs::write(path, modified_content)?;
        Ok(vec![ToolResultContent::text(format!(
            "The user made the following updates to your content:\n\n{diff}"
        ))])
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ReplaceDiffBlock {
    pub search: String,
    pub replace: String,
}

fn parse_replace_diff(diff: &str) -> Result<Vec<ReplaceDiffBlock>, std::io::Error> {
    let mut diff_blocks = Vec::new();
    let mut current_block = ReplaceDiffBlock::default();
    let mut start_search = false;
    let mut start_replace = false;
    for line in diff.lines() {
        if line == "<<<<<<< SEARCH" {
            start_search = true;
            start_replace = false;
        } else if start_search && line == "=======" {
            start_replace = true;
            start_search = false;
        } else if line == ">>>>>>> REPLACE" {
            start_search = false;
            start_replace = false;
            diff_blocks.push(current_block);
            current_block = ReplaceDiffBlock::default();
        } else if start_search {
            current_block.search.push_str(line);
            current_block.search.push('\n');
        } else if start_replace {
            current_block.replace.push_str(line);
            current_block.replace.push('\n');
        }
    }
    Ok(diff_blocks)
}

struct SearchFilesTool {
    workspace: PathBuf,
    description: serde_json::Value,
}

#[derive(Deserialize)]
struct SearchFilesToolArgs {
    pub path: String,
    pub regex: String,
}

#[async_trait]
impl ToolImpl for SearchFilesTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(&mut self, args: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<SearchFilesToolArgs>(args)?;
        let path = normalize_path(&self.workspace, &args.path);
        let matcher = RegexMatcher::new_line_matcher(&args.regex)?;
        tracing::info!("Search for path '{}' and regex {}", path, args.regex);
        let mut searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(b'\x00'))
            .build();

        let mut buffer = Vec::new();
        let writer = Cursor::new(&mut buffer);
        let mut printer = StandardBuilder::new().build_no_color(writer);

        for entry in ignore::Walk::new(path).filter_map(|e| e.ok()) {
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let _ = searcher.search_path(
                &matcher,
                entry.path(),
                printer.sink_with_path(&matcher, entry.path()),
            );
        }
        let res = String::from_utf8(buffer).unwrap();
        Ok(vec![ToolResultContent::text(if res.is_empty() {
            "No results found".to_string()
        } else {
            res
        })])
    }
}
