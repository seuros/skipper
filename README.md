# skipper

MCP server for working in a git repository: local git in-process, and
GitHub, GitLab, Gitea, and Forgejo behind one small set of tools.

## Why

The existing forge MCP servers expose the API, not a workflow. Counted from
their own sources: GitHub's has ~125 tools, GitLab's ~88, Gitea's ~54. One
tool per endpoint, three servers, three vocabularies, and an agent that has
to work out which one it is talking to.

Skipper registers 15, and shows fewer than that: only the ones this
repository can actually use.

An agent does not need to be emperor of your forge. It does not need to
create or destroy an organization on any given turn. It needs to know what
this repository is, whether the build passed, and what broke. Skipper exposes
that and skips the rest.

**No skipper tool can write to a forge.** There is no create, delete, merge,
close, or fork. The only state any tool changes is local: staging,
committing, and local branches. There is no push, fetch, or pull, so an
agent cannot move a commit to a remote through skipper at all.

The destructive operations still exist in the CLIs. To delete a release, an
agent calls `gh` directly, which your harness can allow or deny with one
rule. That is one binary to gate instead of 267 tools to audit.

## How it talks to things

| Layer | Transport |
|-------|-----------|
| local git | [gitoxide](https://github.com/GitoxideLabs/gitoxide), in-process |
| GitHub | REST and GraphQL over [rama](https://crates.io/crates/rama), token from `gh auth token` |
| GitLab | the `glab` CLI |
| Gitea, Forgejo | REST over rama, token from tea's config |

## Authentication

Local git needs no credentials: it reads and writes the repository directly,
never the network.

For forges, skipper reuses the credentials the CLIs already hold. It stores
no tokens of its own. Log in as usual:

```
gh auth login
glab auth login
tea login add
```

For GitHub, skipper asks `gh auth token` for the token once per host (it
honours `GH_TOKEN`), keeps it in memory only, and calls the API itself: one
HTTP request instead of a `gh` process per query. A 401 fetches the token
again, so `gh auth refresh` or a rotated token needs no restart.

For Gitea and Forgejo, skipper reads the token from tea's config instead of
running `tea`, which can block on an interactive prompt that never resolves
under a non-interactive server. `tea login add` is still how you set it up.

## Requirements

Building from source requires Rust 1.99 or newer. The repository pins Rust 1.99.0
for local and release builds.

| Provider | CLI | Minimum | Auth |
|----------|-----|---------|------|
| GitHub | `gh` | 2.50 | `gh auth login` |
| GitLab | `glab` | 1.40 | `glab auth login` |
| Gitea / Forgejo | `tea` | 0.9 | `tea login add` |

Local git needs no binary: it runs in-process on
[gitoxide](https://github.com/GitoxideLabs/gitoxide).

A missing, outdated, or unauthenticated CLI just hides that provider's tools.
An auth check that fails on the network (timeout, connection reset) is not
treated as logged out: skipper retries it with backoff for a few minutes and
announces the tools with `tools/list_changed` once it answers. Detection
results are logged to stderr, which MCP clients keep in their server logs.

## Tool visibility

Tools follow the repository, not the toolbox. A forge's tools appear only when
all of the following hold:

1. the workspace is a git repository,
2. one of its remotes points at that forge, and
3. the forge's CLI is installed, new enough, and authenticated.

Git tools need only a repository; they do not depend on any CLI.

So an installed `gh` does not put GitHub tools in front of a checkout hosted on
Forgejo. `github.com`, `gitlab.com`, and `codeberg.org` are recognized out of
the box; self-hosted instances are mapped in config:

```toml
# ~/.config/skipper/config.toml (global) or ./skipper.toml (per repo)
[hosts]
"github.enterprise.com" = "github"
"gitlab.internal"       = "gitlab"
"192.168.3.20"          = "forgejo"
```

Forge names accept aliases: `github`/`gh`, `gitlab`/`glab`, and
`gitea`/`forgejo`/`tea` (Gitea and Forgejo are both driven by `tea`). Remotes
whose host isn't mapped simply expose no forge tools. The unmapped hosts are
logged at startup so you know what to add.

Visibility is re-evaluated while the server runs. A background watcher polls
the workspace's remotes, so `git init`, `git remote add`, re-pointing a
remote, or removing one updates the tool set within a few seconds and emits
`notifications/tools/list_changed`. No restart needed.

## Installation

```bash
curl -fsSL https://raw.githubusercontent.com/seuros/skipper/master/scripts/install.sh | bash
```

It picks the latest release and the right build for your machine, and
installs to `~/.local/bin`. Pass `--prefix` for somewhere else or `--version`
for a specific tag. Prebuilt for Linux and macOS, x86_64 and arm64.

Or build it yourself:

```bash
cargo install skipper
```

Either way you get the `skipper-mcp` binary. Skipper is a server, not a library:
the Rust API exists so the binary and the tests can share code, and it is not
a supported interface. Depend on the MCP tools and resources, not on
`skipper::*`.

## MCP setup

Skipper speaks MCP over stdio. Add it to your client config:

```json
{
  "mcpServers": {
    "skipper": { "command": "skipper-mcp" }
  }
}
```

## Tools

| Tool | Provider | Purpose |
|------|----------|---------|
| `repo_search` | Gitea/Forgejo | Search repositories by name, paginated |
| `gh_repo_list` | GitHub | List repositories for the authenticated user |
| `glab_project_list` | GitLab | List projects for the authenticated user |
| `build_status` | GitHub/GitLab | CI run or pipeline status for the current repository |
| `build_watch` | GitHub/GitLab | Wait for a CI run's status to change, up to a bounded time |
| `pr_build_wait` | GitHub | Wait for a PR's checks to finish; structured verdict (task-capable) |
| `pr_watch` | GitHub | Watch PRs for merge, close, comments, reviews, checks, and pushes (task-capable) |
| `git_status` | git | Staged, unstaged, and untracked files |
| `git_diff` | git | Scoped patches, stats, or changed paths |
| `git_log` | git | Recent commits |
| `git_show` | git | Commit details with subject, body, trailers |
| `git_show_file` | git | A tracked file at a revision |
| `git_blame` | git | Per-line attribution |
| `git_add` | git | Stage explicit paths |
| `git_commit` | git | Commit staged changes, with trailers or amend |
| `git_branch` | git | Create or delete a local branch |

## Resources

Questions about *this* repository are resources, not tools, and they are
scoped by the workspace's remotes rather than your account.

| URI | Provider | Contents |
|-----|----------|----------|
| `skipper://workspace` | any | The current workspace: working directory, repo root, branch, HEAD, dirty state, remotes (credentials stripped) and the forge each maps to, the current remote marked |
| `skipper://repo` | Gitea/Forgejo | The repository this workspace's remote points at, trying the current remote, then `origin`, when several match |
| `skipper://issues/{state}` | GitHub, Gitea/Forgejo | The current remote's last 30 issues in `open`, `closed` or `all` state, latest update first: number, title, state, author, labels, comment count, created and updated times |
| `skipper://issue/{number}` | GitHub, Gitea/Forgejo | One issue of the current remote: title, state, author, creation time, labels, assignees, the full body and every comment, cleaned like PR comments |
| `skipper://pr/{number}/checks` | GitHub | Check matrix for a PR; `current` selects the current branch's |
| `skipper://pr/{number}/comments` | GitHub | A PR's state (open/closed/merged) and, compactly, every inline review comment, conversation comment and review, bodies without collapsed `<details>`, HTML comments or HTML tags |
| `skipper://pr/{number}/comments/{kind}` | GitHub | The same, keeping only `inline`, `comment` or `review` notes (`all` keeps every one) |
| `skipper://watch/comments/{kind}` | GitHub | `skipper://pr/{number}/comments/{kind}` for every PR under `pr_watch`, in one read; recently finished PRs report their final state, and a PR that cannot be read reports its `error` |
| `skipper://prs/{state}/{author}` | GitHub | The last 30 PRs in `open`, `closed`, `merged` or `all` state by an author (`me` for the authenticated user): number, state, merge time |
| `skipper://watch` | GitHub | PRs under `pr_watch`: status, checks, recent and undelivered events |

### Current remote

Issue resources read from one remote, picked fresh on every read:

1. the remote the checked-out branch tracks (`branch.<name>.remote`);
2. else the only remote;
3. else `origin`.

With several remotes and none of these, the read fails and says so rather
than guess. Switching branches or running `git branch -u` moves the next read
to the other forge; no restart. Every issue payload names the `remote`,
`forge` and `repo` it came from. A Gitea/Forgejo token needs the `read:issue`
scope.

GitHub-only reads (PR checks and discussion, CI runs, `pr_watch`) take the
current remote when it is on GitHub, else the one GitHub repo among the
remotes, so they keep working while the branch tracks a mirror on another
forge. Two different GitHub repos (a fork and its upstream) with neither
current is an error naming both.

Claude Code never lists MCP resources to its model, so for `claude-code`
clients skipper appends the resources visible to that session to its server
instructions. Other clients get the plain instructions. Claude Code also
replaces any failed resource read with its own text, so for `claude-code` the
failure comes back as content naming the reason (`#5 is a pull request; read
skipper://pr/5/comments`). Both quirks are client specs in
`src/mcp/instructions.rs`; add a version constraint (`claude-code < 2.93`)
once a release fixes one.

## Build monitoring

`build_watch` blocks until the run's status changes or `wait_secs` elapses
(default 60), then returns the status with a `terminal` flag. Call it again
while `terminal` is false. It polls; it does not rely on the client
surfacing server notifications.

## PR check monitoring (GitHub)

`pr_build_wait` blocks until every check on a PR concludes, or until the
first failure with `fail_fast` (the default), and returns the overall
conclusion, per-bucket counts, and the failing checks with links. It declares
optional MCP task support. On `timeout_secs` (default 30 min) it returns the
pending snapshot rather than an error.

The `skipper://pr/{number}/checks` resource template returns the check
matrix for any PR (checks grouped by workflow with bucket and links), and `skipper://pr/current/checks` resolves the current branch's PR.

## PR watching (GitHub)

`pr_watch` keeps one watch per server. The first call blocks until
something happens on any watched PR (up to `wait_secs`), then returns the
events. A `pr_watch` call made while another one blocks only adds its PR and
returns at once; the blocking call reports that PR's events too. Merged,
closed, and unreadable PRs always wake the call and leave the watch; `until`
picks which of `comment`, `review`, `checks`, and `push` also do. Read
`skipper://watch` for the current state without blocking or consuming events.

It is built not to hammer the API. Every read carries the last ETag, so an
unchanged PR costs a free `304`; comments and reviews are fetched only after
the PR itself changed. Each PR polls every 10s, backing off to 2 minutes while
quiet (5 when no call waits) and back to 10s on any change; running checks
cap it at 30s. `Retry-After` and a low `X-RateLimit-Remaining` stretch it
further, and a process-wide token bucket caps the request rate. A watch with
no calls or resource reads for 30 minutes stops.

## Configuration

Optional: `skipper.toml` in the repo root or `~/.config/skipper/config.toml`
(local overrides global). See [`skipper.example.toml`](skipper.example.toml).

## Testing

```bash
cargo test                       # unit + detection tests (no auth needed)
SKIPPER_LIVE_TESTS=1 cargo test  # live forge commands via your CLIs
```

## Contributing

Contributions are welcome for tools that come up in real workflows. The bar
is one question: **did you need this while actually working, or does it just
exist in the API?** "Check whether my PR is green" passes. "Transfer
repository ownership" does not.

A few things that help a proposal land:

- say which workflow it serves, and how often you hit it
- prefer read-only; if it writes, explain why the CLI isn't good enough
- prefer one tool that answers a question over three that expose endpoints
- scope it to the current repository where that makes sense; skipper's
  tools follow the workspace's remotes, not your whole account

Creating an organization is banned until the heat death of the universe.
Fifteen-plus years of GitHub and that has never once needed to be done
programmatically, let alone by an agent mid-task. The same goes for deleting
organizations, transferring ownership, and managing billing. If you genuinely
need those, the CLIs still have them, and that is exactly the point: they
live behind one binary your harness can deny, instead of inside a tool
surface you have to audit.
