//! Issues over the Gitea/Forgejo REST API. Needs a token with `read:issue`.

use std::fmt;

use serde::Deserialize;
use serde::de::IgnoredAny;

use super::ForgejoClient;
use crate::error::Result;
use crate::provider::issues::{IssueNote, IssueSummary, IssueThread, LIST_LIMIT};
use crate::provider::text::readable_by;

impl ForgejoClient {
    /// The most recently updated issues of `owner/name` in `state`:
    /// open | closed | all. `type=issues` leaves pull requests out.
    pub async fn issues(&self, owner: &str, name: &str, state: &str) -> Result<Vec<IssueSummary>> {
        let url = self.url(format_args!(
            "{}/issues?type=issues&state={state}&sort=recentupdate&limit={LIST_LIMIT}",
            repo_path(owner, name)
        ));
        let issues: Vec<Listed> = self.get_json(&url).await?;
        Ok(issues.into_iter().map(Listed::into_summary).collect())
    }

    /// Issue `number` with every comment. `None` when `number` is a pull request.
    pub async fn issue(&self, owner: &str, name: &str, number: u64) -> Result<Option<IssueThread>> {
        let base = repo_path(owner, name);
        let url = self.url(format_args!("{base}/issues/{number}"));
        let issue: Issue = self.get_json(&url).await?;
        if issue.pull_request.is_some() {
            return Ok(None);
        }

        let url = self.url(format_args!("{base}/issues/{number}/comments"));
        let comments: Vec<Comment> = self.get_json(&url).await?;
        let mut notes: Vec<IssueNote> = comments.into_iter().map(Comment::into_note).collect();
        notes.sort_by(|a, b| a.at.cmp(&b.at));
        Ok(Some(issue.into_thread(notes)))
    }
}

/// `/repos/{owner}/{name}`, written straight into the URL being built.
pub(super) fn repo_path<'a>(owner: &'a str, name: &'a str) -> impl fmt::Display + 'a {
    fmt::from_fn(move |f| {
        write!(f, "/repos/{}/{}", urlencoding::Encoded(owner), urlencoding::Encoded(name))
    })
}

/// An issue as the list needs it: no body or assignees, which a list of 30
/// would otherwise decode only to drop.
#[derive(Deserialize)]
struct Listed {
    number: u64,
    title: String,
    state: String,
    user: Option<User>,
    #[serde(default)]
    original_author: String,
    #[serde(default)]
    labels: Vec<Label>,
    #[serde(default)]
    comments: u64,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
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
    created_at: String,
    /// Set only when the number is a pull request.
    pull_request: Option<IgnoredAny>,
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

impl Listed {
    fn into_summary(self) -> IssueSummary {
        IssueSummary {
            number: self.number,
            labels: self.labels.into_iter().map(|l| l.name).collect(),
            author: author(self.user, self.original_author),
            title: self.title,
            state: self.state,
            comments: self.comments,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

impl Issue {
    fn into_thread(self, notes: Vec<IssueNote>) -> IssueThread {
        let labels = self.labels.into_iter().map(|l| l.name).collect();
        let author = author(self.user, self.original_author);
        IssueThread {
            number: self.number,
            labels,
            assignees: self.assignees.unwrap_or_default().into_iter().map(|u| u.login).collect(),
            body: readable_by(&author, &self.body),
            author,
            created_at: self.created_at,
            title: self.title,
            state: self.state,
            notes,
        }
    }
}

impl Comment {
    fn into_note(self) -> IssueNote {
        let author = author(self.user, self.original_author);
        IssueNote {
            id: self.id,
            body: readable_by(&author, &self.body),
            author,
            at: self.created_at,
        }
    }
}

#[cfg(test)]
mod tests;
