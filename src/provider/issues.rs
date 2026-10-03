//! Issues in one shape, whichever forge serves them.

use serde::Serialize;

/// Issues per list read.
pub(crate) const LIST_LIMIT: usize = 30;

/// An issue as listed: enough to triage and pick one to read. Every field is
/// always present, so an empty `labels` means none, not unsupported.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IssueSummary {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub author: String,
    pub labels: Vec<String>,
    pub comments: u64,
    pub created_at: String,
    pub updated_at: String,
}

/// An issue with its whole body and discussion, oldest comment first.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IssueThread {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub author: String,
    pub created_at: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub body: String,
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
