//! A PR's discussion over REST: inline review comments, conversation
//! comments and reviews, bodies cut down to their readable text.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{GitHubProvider, PAGE};
use crate::error::Result;
use crate::provider::text::readable;
use crate::workspace::ForgeRepo;

impl GitHubProvider {
    pub async fn pr_discussion(&self, repo: &ForgeRepo, number: u64) -> Result<PrDiscussion> {
        let base = format!(
            "repos/{}/{}",
            urlencoding::encode(&repo.owner),
            urlencoding::encode(&repo.name)
        );
        let pull = format!("{base}/pulls/{number}");
        let head: RestPull = self.api_json_at(&repo.host, &pull).await?;
        let sources = [
            ("inline", format!("{pull}/comments")),
            ("comment", format!("{base}/issues/{number}/comments")),
            ("review", format!("{pull}/reviews")),
        ];
        let mut notes = Vec::new();
        for (kind, path) in sources {
            for page in 1.. {
                let batch: Vec<RestNote> = self
                    .api_json_at(&repo.host, &format!("{path}?per_page={PAGE}&page={page}"))
                    .await?;
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
            body: readable(self.body.as_deref().unwrap_or_default()),
        }
    }
}
