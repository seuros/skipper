//! Forge comment bodies cut down to their readable text.

/// Comment and review bodies are cut past this many chars.
pub(crate) const NOTE_LIMIT: usize = 1500;

/// An issue's own body is often its spec; it gets more room than a comment.
pub(crate) const ISSUE_BODY_LIMIT: usize = 6000;

/// `body` without HTML comments, collapsed `<details>`, inline markup tags or
/// alert banners, blank runs squeezed, cut past `limit` chars.
pub(crate) fn readable(body: &str, limit: usize) -> String {
    let mut kept = String::with_capacity(body.len());
    let mut depth = 0usize;
    let mut rest = body;
    while let Some(ch) = rest.chars().next() {
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map_or("", |end| &rest[end + 3..]);
        } else if rest.starts_with("<details") {
            depth += 1;
            rest = &rest["<details".len()..];
        } else if rest.starts_with("</details>") {
            depth = depth.saturating_sub(1);
            rest = &rest["</details>".len()..];
        } else if let Some(len) = html_tag_len(rest) {
            rest = &rest[len..];
        } else {
            if depth == 0 {
                kept.push(ch);
            }
            rest = &rest[ch.len_utf8()..];
        }
    }

    let mut lines: Vec<&str> = Vec::new();
    for line in kept.lines().map(str::trim_end).filter(|l| !l.trim_start().starts_with("> [!")) {
        if line.is_empty() && lines.last().is_none_or(|last| last.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    clip(lines.join("\n").trim().to_string(), limit)
}

pub(crate) fn clip(text: String, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text,
    }
}

fn html_tag_len(rest: &str) -> Option<usize> {
    const TAGS: [&str; 16] = [
        "a", "img", "sub", "sup", "br", "p", "div", "span", "b", "i", "strong", "em", "summary",
        "picture", "source", "hr",
    ];
    let inner = rest.strip_prefix('<')?;
    let name = inner.strip_prefix('/').unwrap_or(inner);
    let end = name.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(name.len());
    if !TAGS.contains(&name[..end].to_ascii_lowercase().as_str()) {
        return None;
    }
    let close = rest.find('>').filter(|&i| !rest[..i].contains('\n'))?;
    Some(close + 1)
}

#[cfg(test)]
mod tests;
