// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

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
