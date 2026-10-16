use super::*;
use serde_json::json;

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
