// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::path::Path;

pub fn escape_markdown(msg: &str) -> String {
    msg.replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace('*', "\\*")
        .replace('~', "\\~")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

pub fn safe_truncated(s: &str, len: usize) -> String {
    let mut new_len = usize::min(len, s.len());
    let mut s = s.to_string();
    while !s.is_char_boundary(new_len) {
        new_len -= 1;
    }
    s.truncate(new_len);
    s
}

#[inline]
pub fn workspace_to_string(workspace: &Path) -> String {
    workspace.to_str().unwrap().to_string().replace("\\", "/")
}

pub fn normalize_path(workspace: &Path, path: &str) -> String {
    let path = path.to_string().replace("\\", "/");
    let workspace = workspace_to_string(workspace);
    if !path.starts_with(&workspace) {
        format!("{workspace}/{path}")
    } else {
        path
    }
}
