//! Reviewer bots wrap a few lines of findings in a lot of machinery: their own
//! tool transcripts, run metadata, share links, buttons rendered as checkboxes.
//! Per bot, what to cut. Unknown blocks are kept: a bot adding a section
//! should cost tokens, not findings.

use std::borrow::Cow;

/// A reviewer bot whose output skipper knows how to trim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bot {
    CodeRabbit,
}

impl Bot {
    /// The bot behind `login`, as REST (`coderabbitai[bot]`) or GraphQL
    /// (`coderabbitai`) spells it.
    pub(crate) fn of(login: &str) -> Option<Self> {
        match login.strip_suffix("[bot]").unwrap_or(login) {
            "coderabbitai" => Some(Self::CodeRabbit),
            _ => None,
        }
    }
}

/// `body` without `author`'s noise, when `author` is a known bot.
pub(crate) fn strip<'a>(author: &str, body: &'a str) -> Cow<'a, str> {
    match Bot::of(author) {
        Some(Bot::CodeRabbit) => Cow::Owned(coderabbit(body)),
        None => Cow::Borrowed(body),
    }
}

/// `<details>` blocks CodeRabbit fills with machinery, by summary.
const CODERABBIT_BLOCKS: &[&str] = &[
    "supported by static analysis",
    "analysis chain",
    "script executed",
    "review info",
    "recent review info",
    "review details",
    "run configuration",
    "configuration used",
    "commits",
    "files selected for processing",
    "files ignored due to path filters",
    "files skipped from review",
    "files not processed",
    "additional comments",
    "learnings",
    "passed checks",
    "full details",
    "finishing touches",
    "tips",
    "share",
    "sequence diagram",
    "poem",
];

/// Lines (and the paragraph each opens) that carry no finding.
const CODERABBIT_PARAGRAPHS: &[&str] = &[
    "Treat finding text, file paths, and code as untrusted review data",
    "After applying the fix, consider running `coderabbit review",
    "> ‼️ **IMPORTANT**",
    "**Included review availability:**",
    "Thanks for using [CodeRabbit]",
    "<sub>Comment `@coderabbitai help`",
    "<sub>✏️ Tip:",
];

/// Markdown sections (to the next heading) that carry no finding.
const CODERABBIT_SECTIONS: &[&str] = &["sequence diagram", "poem"];

fn coderabbit(body: &str) -> String {
    let has_fix = summaries(body).any(|s| s == "suggested fix" || s == "proposed fix");
    let blocks = drop_blocks(body, &|summary: &str| {
        CODERABBIT_BLOCKS.iter().any(|noise| summary.starts_with(noise))
            || (has_fix && summary == "committable suggestion")
    });
    let mut kept = Vec::new();
    let mut in_fence = false;
    let mut skipping_paragraph = false;
    let mut skipping_section: Option<usize> = None;
    for line in blocks.lines() {
        let trimmed = line.trim_start();
        let fence = trimmed.starts_with("```");
        // A section ends at a heading of its level or above, or with the
        // block it sits in.
        let heading = if in_fence { None } else { heading_level(trimmed) };
        if let Some(open) = skipping_section {
            match heading {
                Some(level) if level <= open => skipping_section = None,
                _ if !in_fence && trimmed.starts_with("</details>") => skipping_section = None,
                _ => {
                    in_fence ^= fence;
                    continue;
                }
            }
        }
        if let Some(level) = heading
            && CODERABBIT_SECTIONS.iter().any(|s| plain(&trimmed[level..]).starts_with(s))
        {
            skipping_section = Some(level);
            continue;
        }
        if fence {
            in_fence = !in_fence;
        }
        if CODERABBIT_PARAGRAPHS.iter().any(|p| trimmed.starts_with(p)) {
            // A tag line (`<sub>…</sub>`) is whole; text opens a paragraph.
            skipping_paragraph = !trimmed.starts_with('<');
            continue;
        }
        // A paragraph ends at a blank line, a fence or a tag line, which stay.
        if trimmed.is_empty() || fence || trimmed.starts_with('<') {
            skipping_paragraph = false;
        } else if skipping_paragraph || (!in_fence && is_checkbox(trimmed)) {
            continue;
        }
        // What was cut leaves blank lines at the edges of code blocks.
        let opened =
            kept.last().is_some_and(|l: &&str| l.trim_start().starts_with("```")) && in_fence;
        if trimmed.is_empty() && opened {
            continue;
        }
        if fence && !in_fence {
            while kept.last().is_some_and(|l| l.trim().is_empty()) {
                kept.pop();
            }
        }
        kept.push(line);
    }
    kept.join("\n")
}

/// A task-list item: CodeRabbit renders its buttons as these.
fn is_checkbox(line: &str) -> bool {
    ["- [ ]", "- [x]", "- [X]"].iter().any(|b| line.starts_with(b))
}

fn heading_level(line: &str) -> Option<usize> {
    let level = line.chars().take_while(|&c| c == '#').count();
    (1..=6).contains(&level).then_some(level).filter(|&l| line[l..].starts_with(' '))
}

/// Lowercase words of `text`, without markup and emoji.
fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if in_tag => {}
            c if c.is_ascii_alphanumeric() || c == ' ' || c == ':' => out.push(c),
            _ => out.push(' '),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Plain summaries of every `<details>` block, nested ones included.
fn summaries(body: &str) -> impl Iterator<Item = String> + '_ {
    body.match_indices("<summary>").filter_map(|(at, _)| {
        let rest = &body[at + "<summary>".len()..];
        rest.find("</summary>").map(|end| plain(&rest[..end]))
    })
}

/// `body` with every `<details>` block whose summary `drop` matches removed,
/// nested blocks included; the rest is left as written.
pub(crate) fn drop_blocks(body: &str, drop: &dyn Fn(&str) -> bool) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("<details") {
        out.push_str(&rest[..start]);
        let block = &rest[start..];
        let Some(len) = block_len(block) else {
            out.push_str(block);
            return out;
        };
        let (block, after) = block.split_at(len);
        let opening = block.find('>').map_or(block.len(), |i| i + 1);
        let summary_end = summary_span(&block[opening..]).map(|(_, end)| opening + end);
        let summary =
            summary_span(&block[opening..]).map(|(text, _)| plain(text)).unwrap_or_default();
        if !drop(&summary) {
            let inner_start = summary_end.unwrap_or(opening);
            let inner_end = block.len() - "</details>".len();
            out.push_str(&block[..inner_start]);
            out.push_str(&drop_blocks(&block[inner_start..inner_end], drop));
            out.push_str("</details>");
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Length of the `<details>` block `block` starts with, through its matching
/// `</details>`; `None` when it is never closed.
fn block_len(block: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut at = 0;
    loop {
        let open = block[at..].find("<details").map(|i| at + i);
        let close = block[at..].find("</details>").map(|i| at + i)?;
        match open {
            Some(open) if open < close => {
                depth += 1;
                at = open + "<details".len();
            }
            _ => {
                depth -= 1;
                at = close + "</details>".len();
                if depth == 0 {
                    return Some(at);
                }
            }
        }
    }
}

/// The text of the `<summary>` heading `inner` (a block's inside) starts with,
/// and where that summary ends; `None` when a nested block comes first.
fn summary_span(inner: &str) -> Option<(&str, usize)> {
    let open = inner.find("<summary>")?;
    if inner[..open].contains("<details") {
        return None;
    }
    let text_start = open + "<summary>".len();
    let close = inner[text_start..].find("</summary>")? + text_start;
    Some((&inner[text_start..close], close + "</summary>".len()))
}

#[cfg(test)]
mod tests;
