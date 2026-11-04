use super::*;
use mcp_host::prelude::ClientRequester;
use mcp_host::protocol::version::ProtocolVersion;
use mcp_host::server::multiplexer::RequestMultiplexer;
use mcp_host::server::session::Session as McpSession;
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::mpsc;

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git").current_dir(cwd).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn initialize(server: &Server, id: &str, version: &str) {
    let response = server
        .handle_request(
            id,
            JsonRpcRequest::new(
                json!(1),
                "initialize",
                Some(json!({
                    "protocolVersion": version,
                    "capabilities": {},
                    "clientInfo": {"name": "skipper-regression", "version": "1"}
                })),
            ),
        )
        .await
        .unwrap();
    assert!(response.error.is_none(), "{response:?}");
    assert_eq!(response.result.unwrap()["protocolVersion"], version);
    server
        .handle_request(id, JsonRpcRequest::notification("notifications/initialized", None))
        .await;
}

#[tokio::test]
async fn watcher_delivery_honors_declared_capabilities_and_log_level() {
    for version in ["2025-06-18", "2025-11-25"] {
        for (logging, list_changed, quiet, expected) in [
            (true, true, false, 2),
            (true, true, true, 1),
            (false, true, false, 1),
            (true, false, false, 1),
            (false, false, false, 0),
        ] {
            let mut builder = Server::builder("skipper-test", "1").with_tools(list_changed);
            if logging {
                builder = builder.with_logging();
            }
            let server = builder.build();
            initialize(&server, "watcher", version).await;
            if quiet {
                let response = server
                    .handle_request(
                        "watcher",
                        JsonRpcRequest::new(
                            json!(2),
                            "logging/setLevel",
                            Some(json!({"level": "error"})),
                        ),
                    )
                    .await
                    .unwrap();
                assert!(response.error.is_none(), "{response:?}");
            }
            let before = server.notification_queue_stats().queued;
            notify_watcher(
                &server,
                &crate::watcher::Notification {
                    watcher: RemoteWatcher::NAME,
                    message: "remote changed".into(),
                    data: json!({"forges": ["github"]}),
                },
            );
            assert_eq!(
                server.notification_queue_stats().queued - before,
                expected,
                "{version}: logging={logging}, list_changed={list_changed}, quiet={quiet}"
            );
        }
    }
}

#[tokio::test]
async fn registered_git_tools_negotiate_and_reject_invalid_arguments() {
    for version in ["2025-06-18", "2025-11-25"] {
        let server = Server::builder("skipper-test", "1")
            .with_tools(true)
            .with_environment(McpEnvironment {
                env: Arc::new(crate::environment::SkipperEnvironment::with_hosts(
                    std::env::current_dir().unwrap(),
                    ForgeHosts::with_defaults(),
                )),
                registry: Arc::new(Registry::with_defaults(|_| true)),
                writes: WritesConfig::default(),
            })
            .build();
        server.register_router(git::router(), Arc::new(crate::git::GitServer)).unwrap();
        initialize(&server, "git", version).await;
        let listed = server
            .handle_request("git", JsonRpcRequest::new(json!(2), "tools/list", None))
            .await
            .unwrap()
            .result
            .unwrap();
        assert!(listed["tools"].as_array().unwrap().iter().any(|t| t["name"] == "git_log"));
        let response = server
            .handle_request(
                "git",
                JsonRpcRequest::new(
                    json!(3),
                    "tools/call",
                    Some(json!({"name": "git_log", "arguments": {"limit": "invalid"}})),
                ),
            )
            .await
            .unwrap();
        assert!(response.error.is_none(), "{response:?}");
        assert_eq!(response.result.unwrap()["isError"], true);
        let response = server
            .handle_request(
                "git",
                JsonRpcRequest::new(
                    json!(4),
                    "tools/call",
                    Some(json!({"name": "git_log", "arguments": {"limit": 1}})),
                ),
            )
            .await
            .unwrap();
        assert!(response.error.is_none(), "{response:?}");
        let result = response.result.unwrap();
        assert_ne!(result["isError"], true, "{result}");
        assert!(result["structuredContent"].is_object(), "{result}");
    }
}

#[tokio::test]
async fn client_without_elicitation_can_push_when_writes_are_enabled_and_confirm_is_defaulted_on() {
    let temp = tempfile::tempdir().unwrap();
    let bare = temp.path().join("remote.git");
    let work = temp.path().join("work");
    std::fs::create_dir(&work).unwrap();
    git(temp.path(), &["init", "--bare", bare.to_str().unwrap()]);
    git(&work, &["init", "--initial-branch=main"]);
    git(&work, &["config", "user.name", "Skipper test"]);
    git(&work, &["config", "user.email", "skipper-test@example.invalid"]);
    git(&work, &["commit", "--allow-empty", "-m", "initial"]);
    git(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);

    let env = Arc::new(crate::environment::SkipperEnvironment::with_hosts(
        &work,
        ForgeHosts::with_defaults(),
    ));
    let registry = Arc::new(Registry::with_defaults(|_| true));
    let writes = WritesConfig { enabled: true, confirm: true };
    let server = Server::builder("skipper-test", "1")
        .with_tools(true)
        .with_environment(McpEnvironment { env: env.clone(), registry: registry.clone(), writes })
        .build();
    server
        .register_router(
            tools::router().unwrap(),
            Arc::new(SkipperServer {
                #[cfg(any(feature = "github", feature = "gitlab"))]
                registry,
                #[cfg(feature = "tea")]
                cwd: work.clone(),
                env,
                writes,
                #[cfg(feature = "github")]
                pr_watcher: crate::pr_watch::PrWatcher::new(Arc::new(
                    crate::environment::SkipperEnvironment::with_hosts(
                        &work,
                        ForgeHosts::with_defaults(),
                    ),
                )),
            }),
        )
        .unwrap();
    server.register_router(git::router(), Arc::new(crate::git::GitServer)).unwrap();
    initialize(&server, "headless", "2025-11-25").await;

    let response = server
        .handle_request(
            "headless",
            JsonRpcRequest::new(
                json!(2),
                "tools/call",
                Some(json!({"name": "git_push", "arguments": {"remotes": ["origin"]}})),
            ),
        )
        .await
        .unwrap();
    assert!(response.error.is_none(), "{response:?}");
    let result = response.result.unwrap();
    assert_ne!(result["isError"], true, "{result}");
    assert_eq!(result["structuredContent"]["pushed"].as_array().unwrap().len(), 1);

    let pushed = Command::new("git")
        .args(["--git-dir", bare.to_str().unwrap(), "rev-parse", "refs/heads/main"])
        .output()
        .unwrap();
    assert!(pushed.status.success(), "headless git_push did not update the local remote");
}

#[tokio::test]
async fn write_confirmation_skips_clients_without_elicitation_support() {
    let session = Arc::new(McpSession::with_id("headless"));
    assert!(session.initialize(
        serde_json::from_value(json!({"name": "headless", "version": "1"})).unwrap(),
        serde_json::from_value(json!({})).unwrap(),
        ProtocolVersion::V2025_11_25,
        Default::default(),
    ));
    assert!(session.mark_ready());
    let (tx, mut rx) = mpsc::channel(1);
    let requester = ClientRequester::new(tx, Arc::new(RequestMultiplexer::new()), session);

    assert!(!requester.supports_elicitation());
    tools::request_write_confirmation(&requester, "push master to github").await.unwrap();
    assert!(rx.try_recv().is_err());
}
