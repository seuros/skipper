//! Issues over the Gitea/Forgejo REST API. Needs a token with `read:issue`.

use serde::Deserialize;

use super::ForgejoClient;
use crate::error::Result;
use crate::provider::issues::{IssueNote, IssueSummary, IssueThread, LIST_LIMIT};
use crate::provider::text::{ISSUE_BODY_LIMIT, NOTE_LIMIT, readable};

impl ForgejoClient {
    /// The most recently updated issues of `owner/name` in `state`:
    /// open | closed | all. `type=issues` leaves pull requests out.
    pub async fn issues(&self, owner: &str, name: &str, state: &str) -> Result<Vec<IssueSummary>> {
        let path = format!(
            "{}/issues?type=issues&state={state}&sort=recentupdate&limit={LIST_LIMIT}",
            repo_path(owner, name)
        );
        let issues: Vec<Issue> = self.get_json(&path).await?;
        Ok(issues.into_iter().map(Issue::into_summary).collect())
    }

    /// Issue `number` with every comment. `None` when `number` is a pull request.
    pub async fn issue(&self, owner: &str, name: &str, number: u64) -> Result<Option<IssueThread>> {
        let base = format!("{}/issues/{number}", repo_path(owner, name));
        let issue: Issue = self.get_json(&base).await?;
        if issue.pull_request.is_some() {
            return Ok(None);
        }

        let comments: Vec<Comment> = self.get_json(&format!("{base}/comments")).await?;
        let mut notes: Vec<IssueNote> = comments.into_iter().map(Comment::into_note).collect();
        notes.sort_by(|a, b| a.at.cmp(&b.at));
        Ok(Some(issue.into_thread(notes)))
    }
}

fn repo_path(owner: &str, name: &str) -> String {
    format!("/repos/{}/{}", urlencoding::encode(owner), urlencoding::encode(name))
}

#[derive(Deserialize)]
struct Issue {
    number: u64,
    title: String,
    state: String,
    user: Option<User>,
    #[serde(default)]
    original_author: String,
    #[serde(default)]
    labels: Vec<Label>,
    assignees: Option<Vec<User>>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    comments: u64,
    #[serde(default)]
    updated_at: String,
    /// Set only when the number is a pull request.
    pull_request: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Comment {
    id: u64,
    user: Option<User>,
    #[serde(default)]
    original_author: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    created_at: String,
}

#[derive(Deserialize)]
struct User {
    login: String,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

/// A migrated issue or comment is owned by a placeholder account; the
/// original author's name survives in `original_author`.
fn author(user: Option<User>, original_author: String) -> String {
    if !original_author.is_empty() {
        return original_author;
    }
    user.map_or_else(|| "ghost".to_string(), |u| u.login)
}

impl Issue {
    fn labels(&mut self) -> Vec<String> {
        std::mem::take(&mut self.labels).into_iter().map(|l| l.name).collect()
    }

    fn into_summary(mut self) -> IssueSummary {
        IssueSummary {
            number: self.number,
            labels: self.labels(),
            author: author(self.user, self.original_author),
            title: self.title,
            state: self.state,
            comments: self.comments,
            updated_at: self.updated_at,
        }
    }

    fn into_thread(mut self, notes: Vec<IssueNote>) -> IssueThread {
        IssueThread {
            number: self.number,
            labels: self.labels(),
            assignees: self.assignees.unwrap_or_default().into_iter().map(|u| u.login).collect(),
            author: author(self.user, self.original_author),
            title: self.title,
            state: self.state,
            body: readable(&self.body, ISSUE_BODY_LIMIT),
            notes,
        }
    }
}

impl Comment {
    fn into_note(self) -> IssueNote {
        IssueNote {
            id: self.id,
            author: author(self.user, self.original_author),
            at: self.created_at,
            body: readable(&self.body, NOTE_LIMIT),
        }
    }
}

#[cfg(test)]
mod tests;
