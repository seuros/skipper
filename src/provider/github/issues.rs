//! Issues of an explicit repository: the list over GraphQL, which leaves pull
//! requests out and sorts by update; one issue and its comments over REST.

use serde::Deserialize;

use super::{GitHubProvider, PAGE, User, login};
use crate::error::{CliError, Result};
use crate::provider::issues::{IssueNote, IssueSummary, IssueThread, LIST_LIMIT};
use crate::provider::text::readable;

const LIST_QUERY: &str = r"query($owner: String!, $name: String!, $first: Int!, $states: [IssueState!]) {
  repository(owner: $owner, name: $name) {
    issues(first: $first, states: $states, orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes {
        number title state createdAt updatedAt author { login }
        labels(first: 20) { nodes { name } } comments { totalCount }
      }
    }
  }
}";

impl GitHubProvider {
    /// The most recently updated issues of `owner/name` in `state`:
    /// open | closed | all.
    pub async fn issues(
        &self,
        host: &str,
        owner: &str,
        name: &str,
        state: &str,
    ) -> Result<Vec<IssueSummary>> {
        let states = match state {
            "open" => serde_json::json!(["OPEN"]),
            "closed" => serde_json::json!(["CLOSED"]),
            _ => serde_json::Value::Null,
        };
        let variables = serde_json::json!({
            "owner": owner, "name": name, "first": LIST_LIMIT, "states": states,
        });
        let data: GqlData = self.graphql(host, LIST_QUERY, variables).await?;
        let repository = data.repository.ok_or_else(|| {
            CliError::no_target(format!("no repository {owner}/{name} on {host}"))
        })?;
        Ok(repository.issues.nodes.into_iter().map(GqlIssue::into_summary).collect())
    }

    /// Issue `number` with every comment. `None` when `number` is a pull request.
    pub async fn issue(
        &self,
        host: &str,
        owner: &str,
        name: &str,
        number: u64,
    ) -> Result<Option<IssueThread>> {
        let base = format!(
            "repos/{}/{}/issues/{number}",
            urlencoding::encode(owner),
            urlencoding::encode(name)
        );
        let issue: RestIssue = self.api_json_at(host, &base).await?;
        if issue.pull_request.is_some() {
            return Ok(None);
        }

        let mut notes = Vec::new();
        for page in 1.. {
            let batch: Vec<RestComment> = self
                .api_json_at(host, &format!("{base}/comments?per_page={PAGE}&page={page}"))
                .await?;
            let full = batch.len() == PAGE;
            notes.extend(batch.into_iter().map(RestComment::into_note));
            if !full {
                break;
            }
        }
        notes.sort_by(|a, b| a.at.cmp(&b.at));

        Ok(Some(IssueThread {
            number: issue.number,
            title: issue.title,
            state: issue.state,
            author: login(issue.user),
            created_at: issue.created_at,
            labels: issue.labels.into_iter().map(|l| l.name).collect(),
            assignees: issue.assignees.unwrap_or_default().into_iter().map(|u| u.login).collect(),
            body: readable(issue.body.as_deref().unwrap_or_default()),
            notes,
        }))
    }
}

#[derive(Deserialize)]
struct GqlData {
    repository: Option<GqlRepository>,
}

#[derive(Deserialize)]
struct GqlRepository {
    issues: Nodes<GqlIssue>,
}

#[derive(Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlIssue {
    number: u64,
    title: String,
    state: String,
    created_at: String,
    updated_at: String,
    author: Option<User>,
    labels: Option<Nodes<Label>>,
    comments: Count,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Count {
    total_count: u64,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

impl GqlIssue {
    fn into_summary(self) -> IssueSummary {
        IssueSummary {
            number: self.number,
            title: self.title,
            state: self.state.to_lowercase(),
            author: login(self.author),
            labels: self
                .labels
                .map(|l| l.nodes.into_iter().map(|l| l.name).collect())
                .unwrap_or_default(),
            comments: self.comments.total_count,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[derive(Deserialize)]
struct RestIssue {
    number: u64,
    title: String,
    state: String,
    user: Option<User>,
    created_at: String,
    #[serde(default)]
    labels: Vec<Label>,
    assignees: Option<Vec<User>>,
    body: Option<String>,
    /// Present only when the number is a pull request.
    pull_request: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct RestComment {
    id: u64,
    user: Option<User>,
    body: Option<String>,
    created_at: String,
}

impl RestComment {
    fn into_note(self) -> IssueNote {
        IssueNote {
            id: self.id,
            author: login(self.user),
            at: self.created_at,
            body: readable(self.body.as_deref().unwrap_or_default()),
        }
    }
}

#[cfg(test)]
mod tests;
