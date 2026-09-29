//! A PR's discussion over REST: inline review comments, conversation
//! comments and reviews, bodies cut down to their readable text.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{GitHubProvider, PAGE};
use crate::error::Result;

/// Comment and review bodies are cut past this many chars.
pub(crate) const BODY_LIMIT: usize = 1500;

impl GitHubProvider {
    pub async fn pr_discussion(&self, pr: Option<u64>) -> Result<PrDiscussion> {
        let number = match pr {
            Some(n) => n,
            None => self.current_pr().await?,
        };

        let pull = format!("repos/{{owner}}/{{repo}}/pulls/{number}");
        let head: RestPull = self.api_json(&pull).await?;
        let sources = [
            ("inline", format!("{pull}/comments")),
            ("comment", format!("repos/{{owner}}/{{repo}}/issues/{number}/comments")),
            ("review", format!("{pull}/reviews")),
        ];
        let mut notes = Vec::new();
        for (kind, path) in sources {
            for page in 1.. {
                let batch: Vec<RestNote> =
                    self.api_json(&format!("{path}?per_page={PAGE}&page={page}")).await?;
                let full = batch.len() == PAGE;
                notes.extend(
                    batch
                        .into_iter()
                        .filter(|n| n.state.as_deref() != Some("PENDING"))
                        .map(|n| n.into_note(kind)),
                );
                if !full {
                    break;
                }
            }
        }
        notes.sort_by(|a, b| a.at.cmp(&b.at));

        Ok(PrDiscussion {
            pr: number,
            state: if head.merged { "merged".to_string() } else { head.state },
            notes,
        })
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PrDiscussion {
    pub pr: u64,
    pub state: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<PrNote>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PrNote {
    pub id: u64,
    pub kind: &'static str,
    pub author: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip)]
    pub at: Option<String>,
    pub body: String,
}

#[derive(Deserialize)]
struct RestPull {
    state: String,
    merged: bool,
}

#[derive(Deserialize)]
struct RestNote {
    id: u64,
    user: Option<User>,
    body: Option<String>,
    created_at: Option<String>,
    submitted_at: Option<String>,
    path: Option<String>,
    line: Option<u64>,
    original_line: Option<u64>,
    state: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct User {
    pub login: String,
}

/// A deleted account comes back as `user: null`; GitHub shows it as ghost.
pub(crate) fn login(user: Option<User>) -> String {
    user.map_or_else(|| "ghost".to_string(), |u| u.login)
}

impl RestNote {
    fn into_note(self, kind: &'static str) -> PrNote {
        PrNote {
            id: self.id,
            kind,
            author: login(self.user),
            path: self.path,
            line: self.line.or(self.original_line),
            state: self.state,
            at: self.created_at.or(self.submitted_at),
            body: readable_body(self.body.as_deref().unwrap_or_default()),
        }
    }
}

fn readable_body(body: &str) -> String {
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
    clip(lines.join("\n").trim().to_string())
}

pub(crate) fn clip(text: String) -> String {
    match text.char_indices().nth(BODY_LIMIT) {
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
