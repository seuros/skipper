//! Forge comment bodies cut down to their readable text.

pub(crate) mod noise;

/// `pr_watch` event bodies are cut past this many chars; the event's `url` and
/// `skipper://pr/{number}/comments` carry the whole text.
#[cfg(feature = "github")]
pub(crate) const EVENT_BODY_LIMIT: usize = 1500;

/// `author`'s `body`, readable, without the noise of a known reviewer bot.
pub(crate) fn readable_by(author: &str, body: &str) -> String {
    readable(&noise::strip(author, body))
}

/// `body` without HTML comments, markup tags or alert banners, blank runs
/// squeezed. Collapsed `<details>` open up: the summary stays as a line, the
/// content as written. Never cut: what remains is the text a reader acts on.
pub(crate) fn readable(body: &str) -> String {
    let mut kept = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(ch) = rest.chars().next() {
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map_or("", |end| &rest[end + 3..]);
        } else if let Some(len) = html_tag_len(rest) {
            rest = &rest[len..];
        } else {
            kept.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }

    let mut lines: Vec<String> = Vec::new();
    let mut in_fence = false;
    for line in kept.lines().map(str::trim_end).filter(|l| !l.trim_start().starts_with("> [!")) {
        if line.is_empty() && lines.last().is_none_or(|last| last.is_empty()) {
            continue;
        }
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }
        // GitHub shows entities decoded, except in code.
        lines.push(if in_fence { line.to_string() } else { decode_entities(line) });
    }
    let rule = |l: &String| matches!(l.trim(), "" | "---");
    while lines.last().is_some_and(rule) {
        lines.pop();
    }
    let lead = lines.iter().take_while(|l| rule(l)).count();
    lines[lead..].join("\n").trim().to_string()
}

/// The entities markdown writers escape; `&amp;` last, so `&amp;lt;` stays text.
fn decode_entities(line: &str) -> String {
    if !line.contains('&') {
        return line.to_string();
    }
    line.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// `text` cut past `limit` chars, marked with `…`.
#[cfg(feature = "github")]
pub(crate) fn clip(text: String, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text,
    }
}

fn html_tag_len(rest: &str) -> Option<usize> {
    const TAGS: [&str; 17] = [
        "a", "img", "sub", "sup", "br", "p", "div", "span", "b", "i", "strong", "em", "summary",
        "details", "picture", "source", "hr",
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
