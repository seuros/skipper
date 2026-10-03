//! Issues in one shape, whichever forge serves them.

use serde::Serialize;

/// Issues per list read.
pub(crate) const LIST_LIMIT: usize = 30;

/// An issue as listed: enough to pick one to read.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IssueSummary {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub author: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "is_zero")]
    pub comments: u64,
    pub updated_at: String,
}

/// An issue with its discussion, oldest comment first.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IssueThread {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub author: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub assignees: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub body: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<IssueNote>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IssueNote {
    pub id: u64,
    pub author: String,
    #[serde(skip)]
    pub at: String,
    pub body: String,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}
