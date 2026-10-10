//! A PR's discussion over REST: its description, inline review comments,
//! conversation comments and reviews, bodies cut down to their readable text.

use std::borrow::Cow;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::runs::repo_path;
use super::{GitHubProvider, PAGE, rest_url};
use crate::error::Result;
use crate::provider::text::readable_by;
use crate::workspace::ForgeRepo;

impl GitHubProvider {
    pub async fn pr_discussion(&self, repo: &ForgeRepo, number: u64) -> Result<PrDiscussion> {
        let base = repo_path(repo);
        let url = rest_url(&repo.host, format_args!("{base}/pulls/{number}"));
        let head: RestPull = self.api_json_at(&repo.host, &url).await?;
        let sources = [
            ("inline", "pulls", "comments"),
            ("comment", "issues", "comments"),
            ("review", "pulls", "reviews"),
        ];
        let mut notes = Vec::new();
        let author = login(head.user);
        let description = readable_by(&author, head.body.as_deref().unwrap_or_default());
        if !description.is_empty() {
            notes.push(PrNote {
                id: head.id,
                kind: "description",
                author,
                path: None,
                line: None,
                state: None,
                at: Some(head.created_at),
                body: description,
            });
        }
        for (kind, under, what) in sources {
            for page in 1.. {
                let url = rest_url(
                    &repo.host,
                    format_args!("{base}/{under}/{number}/{what}?per_page={PAGE}&page={page}"),
                );
                let batch: Vec<RestNote> = self.api_json_at(&repo.host, &url).await?;
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
            state: if head.merged { Cow::Borrowed("merged") } else { Cow::Owned(head.state) },
            notes,
        })
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PrDiscussion {
    pub pr: u64,
    /// open | closed | merged
    pub state: Cow<'static, str>,
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
    id: u64,
    state: String,
    merged: bool,
    user: Option<User>,
    body: Option<String>,
    created_at: String,
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
        let author = login(self.user);
        PrNote {
            id: self.id,
            kind,
            body: readable_by(&author, self.body.as_deref().unwrap_or_default()),
            author,
            path: self.path,
            line: self.line.or(self.original_line),
            state: self.state,
            at: self.created_at.or(self.submitted_at),
        }
    }
}
