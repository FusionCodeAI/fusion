//! Integration and unit tests for `fusion::agent::mesh_network`.
//!
//! Tests message serialization, peer discovery handshakes, TTL expiry,
//! task offer and accept roundtrips, broadcast messaging, and TCP fallback.

#[path = "../src/agent/mesh_network.rs"]
pub mod mesh_network;

use std::path::PathBuf;
use std::time::Duration;

use mesh_network::*;
use tempfile::tempdir;
use uuid::Uuid;

// ============================================================================
// 1. PeerId and PeerStatus Tests
// ============================================================================

#[test]
fn test_peer_id_generation_and_traits() {
    let id1 = PeerId::new();
    let id2 = PeerId::new();

    // Verify uniqueness
    assert_ne!(id1, id2);

    // Verify valid UUID string
    assert!(Uuid::parse_str(id1.as_str()).is_ok());
    assert_eq!(id1.as_str(), &*id1);
    assert_eq!(format!("{id1}"), id1.0);

    // Conversions
    let custom_str = "peer-alpha-123";
    let from_str: PeerId = custom_str.into();
    assert_eq!(from_str.as_str(), custom_str);

    let from_string: PeerId = String::from("peer-beta-456").into();
    assert_eq!(from_string.as_str(), "peer-beta-456");

    let string_out: String = from_str.into();
    assert_eq!(string_out, "peer-alpha-123");
}

#[test]
fn test_peer_id_serialization_roundtrip() {
    let original = PeerId::new();
    let json = serde_json::to_string(&original).expect("serialize PeerId");

    // PeerId uses #[serde(transparent)], so it should serialize as a plain JSON string
    assert_eq!(json, format!("\"{}\"", original.as_str()));

    let deserialized: PeerId = serde_json::from_str(&json).expect("deserialize PeerId");
    assert_eq!(original, deserialized);
}

#[test]
fn test_peer_status_states_and_serde() {
    let idle = PeerStatus::Idle;
    let busy = PeerStatus::Busy;
    let error = PeerStatus::Error;

    assert!(idle.is_idle());
    assert!(!idle.is_busy());

    assert!(busy.is_busy());
    assert!(!busy.is_idle());

    assert!(error.is_error());
    assert!(!error.is_busy());

    // Display
    assert_eq!(format!("{idle}"), "idle");
    assert_eq!(format!("{busy}"), "busy");
    assert_eq!(format!("{error}"), "error");

    // Serialization (snake_case)
    assert_eq!(serde_json::to_string(&idle).unwrap(), "\"idle\"");
    assert_eq!(serde_json::to_string(&busy).unwrap(), "\"busy\"");
    assert_eq!(serde_json::to_string(&error).unwrap(), "\"error\"");

    // Deserialization with aliases (case insensitivity)
    let d_idle: PeerStatus = serde_json::from_str("\"Idle\"").unwrap();
    assert_eq!(d_idle, PeerStatus::Idle);

    let d_busy: PeerStatus = serde_json::from_str("\"BUSY\"").unwrap();
    assert_eq!(d_busy, PeerStatus::Busy);

    let d_error: PeerStatus = serde_json::from_str("\"ERROR\"").unwrap();
    assert_eq!(d_error, PeerStatus::Error);
}

// ============================================================================
// 2. MeshMessage Serialization & Wire Format Tests
// ============================================================================

#[test]
fn test_message_serialization_ping_pong() {
    // Ping
    let ping = MeshMessage::ping("peer-node-1");
    let ping_json = serde_json::to_string(&ping).expect("serialize Ping");
    assert!(ping_json.contains("\"type\":\"ping\""));
    assert!(ping_json.contains("\"peer_id\":\"peer-node-1\""));

    let ping_back: MeshMessage = serde_json::from_str(&ping_json).expect("deserialize Ping");
    assert_eq!(ping, ping_back);

    // Deserialization with PascalCase alias
    let alias_ping: MeshMessage =
        serde_json::from_str(r#"{"type":"Ping","peer_id":"peer-node-1"}"#).unwrap();
    assert_eq!(ping, alias_ping);

    // Pong
    let pong = MeshMessage::pong("peer-node-2", "claude-3-5-sonnet", PeerStatus::Idle);
    let pong_json = serde_json::to_string(&pong).expect("serialize Pong");
    assert!(pong_json.contains("\"type\":\"pong\""));
    assert!(pong_json.contains("\"active_model\":\"claude-3-5-sonnet\""));
    assert!(pong_json.contains("\"status\":\"idle\""));

    let pong_back: MeshMessage = serde_json::from_str(&pong_json).expect("deserialize Pong");
    assert_eq!(pong, pong_back);
}

#[test]
fn test_message_serialization_task_lifecycle() {
    // 1. TaskOffer
    let offer = MeshMessage::task_offer(
        "task-101",
        "Refactor authentication middleware to use JWT",
        vec!["rust".to_string(), "security".to_string()],
    );
    let offer_json = serde_json::to_string(&offer).expect("serialize TaskOffer");
    assert!(offer_json.contains("\"type\":\"task_offer\""));
    assert!(offer_json.contains("\"task_id\":\"task-101\""));
    assert!(offer_json.contains("\"rust\""));

    let offer_back: MeshMessage =
        serde_json::from_str(&offer_json).expect("deserialize TaskOffer");
    assert_eq!(offer, offer_back);

    // 2. TaskAccept
    let accept = MeshMessage::task_accept("task-101", "worker-node-99");
    let accept_json = serde_json::to_string(&accept).expect("serialize TaskAccept");
    assert!(accept_json.contains("\"type\":\"task_accept\""));
    assert!(accept_json.contains("\"worker_id\":\"worker-node-99\""));

    let accept_back: MeshMessage =
        serde_json::from_str(&accept_json).expect("deserialize TaskAccept");
    assert_eq!(accept, accept_back);

    // 3. TaskResult (Success)
    let result_ok = MeshMessage::task_result(
        "task-101",
        "worker-node-99",
        "JWT middleware tests passing 100%",
        true,
    );
    let result_json = serde_json::to_string(&result_ok).expect("serialize TaskResult");
    assert!(result_json.contains("\"type\":\"task_result\""));
    assert!(result_json.contains("\"success\":true"));

    let result_back: MeshMessage =
        serde_json::from_str(&result_json).expect("deserialize TaskResult");
    assert_eq!(result_ok, result_back);

    // 4. TaskResult (Failure)
    let result_err = MeshMessage::task_result(
        "task-102",
        "worker-node-99",
        "Failed to resolve circular dependency in auth/jwt.rs",
        false,
    );
    let result_err_json = serde_json::to_string(&result_err).expect("serialize TaskResult fail");
    assert!(result_err_json.contains("\"success\":false"));

    let err_back: MeshMessage =
        serde_json::from_str(&result_err_json).expect("deserialize TaskResult fail");
    assert_eq!(result_err, err_back);
}

// ============================================================================
// 3. PeerRegistry and TTL Expiry Tests
// ============================================================================

#[tokio::test]
async fn test_peer_registry_registration_and_touch() {
    let registry = PeerRegistry::new(DEFAULT_PEER_TTL);
    assert_eq!(registry.len().await, 0);
    assert!(registry.is_empty().await);

    let endpoint = PeerEndpoint::Unix(PathBuf::from("/tmp/mesh_test.sock"));
    registry
        .register(
            "peer-test-1",
            "claude-3-5-sonnet",
            PeerStatus::Idle,
            endpoint.clone(),
            Some(1234),
        )
        .await;

    assert_eq!(registry.len().await, 1);
    assert!(!registry.is_empty().await);

    let peer = registry.get_peer("peer-test-1").await.expect("peer exists");
    assert_eq!(peer.peer_id, "peer-test-1");
    assert_eq!(peer.active_model, "claude-3-5-sonnet");
    assert_eq!(peer.status, PeerStatus::Idle);
    assert_eq!(peer.pid, Some(1234));
    assert_eq!(peer.endpoint, endpoint);

    // Touch peer
    let touched = registry.touch("peer-test-1").await;
    assert!(touched);

    let not_touched = registry.touch("non-existent").await;
    assert!(!not_touched);

    // Update status
    let updated = registry
        .update_status("peer-test-1", PeerStatus::Busy)
        .await;
    assert!(updated);

    let peer = registry.get_peer("peer-test-1").await.unwrap();
    assert_eq!(peer.status, PeerStatus::Busy);
}

#[tokio::test]
async fn test_peer_registry_ttl_expiry() {
    // Create registry with a very short 50ms TTL for testing expiry
    let short_ttl = Duration::from_millis(50);
    let registry = PeerRegistry::new(short_ttl);

    let endpoint = PeerEndpoint::Unix(PathBuf::from("/tmp/mesh_ttl.sock"));
    registry
        .register(
            "peer-short-lived",
            "model-x",
            PeerStatus::Idle,
            endpoint,
            Some(9999),
        )
        .await;

    // Immediately available
    assert_eq!(registry.len().await, 1);
    assert!(registry.get_peer("peer-short-lived").await.is_some());

    // Wait past the 50ms TTL
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Record is now expired
    assert!(registry.get_peer("peer-short-lived").await.is_none());
    assert_eq!(registry.len().await, 0);

    // Prune should report purge
    let purged = registry.prune_expired().await;
    assert_eq!(purged, 0); // Already removed by get_peer
}

// ============================================================================
// 4. Socket Lifecycle & Peer Handshake Tests
// ============================================================================

#[tokio::test]
async fn test_mesh_node_socket_lifecycle() {
    let temp = tempdir().expect("tempdir");
    let node = MeshNode::in_dir(temp.path(), "claude-3-5-sonnet")
        .await
        .expect("start MeshNode");

    assert!(node.socket_path().exists());
    assert_eq!(node.active_model().await, "claude-3-5-sonnet");
    assert_eq!(node.status().await, PeerStatus::Idle);

    let sock_path = node.socket_path().to_path_buf();

    // Clean shutdown removes socket file
    node.shutdown().await;
    assert!(!sock_path.exists());
}

#[tokio::test]
async fn test_peer_handshake_and_discovery() {
    let temp = tempdir().expect("tempdir");

    // Start Node A and Node B with distinct simulated PIDs in the same run directory
    let node_a = MeshNode::in_dir_with_pid(temp.path(), 80001, "claude-3-5-sonnet")
        .await
        .expect("start node A");

    let node_b = MeshNode::in_dir_with_pid(temp.path(), 80002, "deepseek-coder")
        .await
        .expect("start node B");

    // Node A discovers peers in the directory
    let discovered = node_a.discover_peers().await.expect("discovery on A");
    assert_eq!(discovered.len(), 1);

    let peer_b = &discovered[0];
    assert_eq!(peer_b.peer_id, node_b.peer_id());
    assert_eq!(peer_b.active_model, "deepseek-coder");
    assert_eq!(peer_b.status, PeerStatus::Idle);
    assert_eq!(peer_b.pid, Some(80002));

    // Direct ping from Node A to Node B
    let pong = node_a
        .ping(node_b.peer_id())
        .await
        .expect("ping from A to B");

    match pong {
        MeshMessage::Pong {
            peer_id,
            active_model,
            status,
        } => {
            assert_eq!(peer_id, node_b.peer_id());
            assert_eq!(active_model, "deepseek-coder");
            assert_eq!(status, PeerStatus::Idle);
        }
        other => panic!("Expected Pong, got {other:?}"),
    }

    // Clean up
    node_a.shutdown().await;
    node_b.shutdown().await;
}

// ============================================================================
// 5. Task Offer, Accept & Result Roundtrips
// ============================================================================

#[tokio::test]
async fn test_task_offer_and_accept_roundtrip_auto_accept() {
    let temp = tempdir().expect("tempdir");

    let requester = MeshNode::in_dir_with_pid(temp.path(), 81001, "orchestrator")
        .await
        .expect("start requester");

    let worker = MeshNode::in_dir_with_pid(temp.path(), 81002, "worker-agent")
        .await
        .expect("start worker");

    // Enable auto-accept on worker
    worker.set_auto_accept(true).await;
    assert!(worker.auto_accept().await);

    // Requester and worker discover each other in the mesh
    let discovered = requester.discover_peers().await.expect("requester discover");
    assert_eq!(discovered.len(), 1);
    let worker_discovered = worker.discover_peers().await.expect("worker discover");
    assert_eq!(worker_discovered.len(), 1);
    // Subscribe requester to incoming messages to receive TaskResult
    let mut requester_rx = requester.subscribe();

    // 1. Requester offers a task to worker
    let task_id = "task-calc-100";
    let accept_resp = requester
        .offer_task(
            worker.peer_id(),
            task_id,
            "Compute checksum for assets",
            vec!["crypto".to_string()],
            Duration::from_secs(5),
        )
        .await
        .expect("offer task");

    // Verify worker automatically accepted
    match accept_resp {
        MeshMessage::TaskAccept {
            task_id: accepted_id,
            worker_id,
        } => {
            assert_eq!(accepted_id, task_id);
            assert_eq!(worker_id, worker.peer_id());
        }
        other => panic!("Expected TaskAccept, got {other:?}"),
    }

    // Verify worker status switched to Busy
    assert_eq!(worker.status().await, PeerStatus::Busy);

    // 2. Worker executes task and returns TaskResult
    worker
        .send_task_result(
            requester.peer_id(),
            task_id,
            "Checksum: 0xDEADBEEF verified",
            true,
        )
        .await
        .expect("send task result");

    // Verify worker status switched back to Idle
    assert_eq!(worker.status().await, PeerStatus::Idle);

    // 3. Requester receives TaskResult on its subscription
    let incoming = requester
        .next_message(&mut requester_rx, Duration::from_secs(3))
        .await
        .expect("receive TaskResult");

    match incoming {
        MeshMessage::TaskResult {
            task_id: res_id,
            worker_id,
            output,
            success,
        } => {
            assert_eq!(res_id, task_id);
            assert_eq!(worker_id, worker.peer_id());
            assert_eq!(output, "Checksum: 0xDEADBEEF verified");
            assert!(success);
        }
        other => panic!("Expected TaskResult, got {other:?}"),
    }

    requester.shutdown().await;
    worker.shutdown().await;
}

#[tokio::test]
async fn test_manual_task_offer_and_accept_roundtrip() {
    let temp = tempdir().expect("tempdir");

    let requester = MeshNode::in_dir_with_pid(temp.path(), 82001, "orchestrator")
        .await
        .expect("start requester");

    let worker = MeshNode::in_dir_with_pid(temp.path(), 82002, "worker-agent")
        .await
        .expect("start worker");

    // Auto-accept disabled (manual handling)
    worker.set_auto_accept(false).await;

    // Both discover each other
    requester.discover_peers().await.unwrap();
    worker.discover_peers().await.unwrap();

    let mut worker_rx = worker.subscribe();
    let mut requester_rx = requester.subscribe();

    let task_id = "task-manual-200";

    // Requester sends one-way TaskOffer
    let offer_msg = MeshMessage::task_offer(
        task_id,
        "Analyze database migration scripts",
        vec!["sql".to_string()],
    );
    requester
        .direct_send(worker.peer_id(), &offer_msg)
        .await
        .expect("send offer");

    // Worker receives offer on subscription
    let offer_recv = worker
        .next_message(&mut worker_rx, Duration::from_secs(3))
        .await
        .expect("worker receives offer");

    match offer_recv {
        MeshMessage::TaskOffer {
            task_id: t_id,
            description,
            required_skills,
        } => {
            assert_eq!(t_id, task_id);
            assert_eq!(description, "Analyze database migration scripts");
            assert_eq!(required_skills, vec!["sql".to_string()]);
        }
        other => panic!("Expected TaskOffer, got {other:?}"),
    }

    // Worker accepts task
    worker
        .accept_task(requester.peer_id(), task_id)
        .await
        .expect("worker accepts");

    assert_eq!(worker.status().await, PeerStatus::Busy);

    // Requester receives TaskAccept
    let accept_recv = requester
        .next_message(&mut requester_rx, Duration::from_secs(3))
        .await
        .expect("requester receives accept");

    match accept_recv {
        MeshMessage::TaskAccept {
            task_id: a_id,
            worker_id,
        } => {
            assert_eq!(a_id, task_id);
            assert_eq!(worker_id, worker.peer_id());
        }
        other => panic!("Expected TaskAccept, got {other:?}"),
    }

    // Worker completes and returns result
    worker
        .send_task_result(
            requester.peer_id(),
            task_id,
            "Migration safe: no breaking schema alterations",
            true,
        )
        .await
        .expect("send result");

    assert_eq!(worker.status().await, PeerStatus::Idle);

    let result_recv = requester
        .next_message(&mut requester_rx, Duration::from_secs(3))
        .await
        .expect("requester receives result");

    match result_recv {
        MeshMessage::TaskResult {
            task_id: r_id,
            output,
            success,
            ..
        } => {
            assert_eq!(r_id, task_id);
            assert!(success);
            assert!(output.contains("Migration safe"));
        }
        other => panic!("Expected TaskResult, got {other:?}"),
    }

    requester.shutdown().await;
    worker.shutdown().await;
}

// ============================================================================
// 6. Async Broadcast Messaging Tests
// ============================================================================

#[tokio::test]
async fn test_broadcast_messaging_to_multiple_peers() {
    let temp = tempdir().expect("tempdir");

    let node_root = MeshNode::in_dir_with_pid(temp.path(), 83001, "broadcaster")
        .await
        .expect("start root");

    let node_worker1 = MeshNode::in_dir_with_pid(temp.path(), 83002, "worker-1")
        .await
        .expect("start worker 1");

    let node_worker2 = MeshNode::in_dir_with_pid(temp.path(), 83003, "worker-2")
        .await
        .expect("start worker 2");

    // Root discovers both workers
    let peers = node_root.discover_peers().await.expect("discover");
    assert_eq!(peers.len(), 2);

    let mut rx1 = node_worker1.subscribe();
    let mut rx2 = node_worker2.subscribe();

    // Broadcast a message
    let broadcast_msg = MeshMessage::task_offer(
        "task-broadcast-001",
        "Global workspace re-indexing",
        vec!["indexing".to_string()],
    );

    let broadcast_results = node_root.broadcast(&broadcast_msg).await;
    assert_eq!(broadcast_results.len(), 2);

    for (peer_id, res) in broadcast_results {
        assert!(res.is_ok(), "Broadcast to {peer_id} should succeed");
    }

    // Both workers should have received the broadcast message
    let msg1 = node_worker1
        .next_message(&mut rx1, Duration::from_secs(3))
        .await
        .expect("worker 1 received");
    let msg2 = node_worker2
        .next_message(&mut rx2, Duration::from_secs(3))
        .await
        .expect("worker 2 received");

    assert_eq!(msg1, broadcast_msg);
    assert_eq!(msg2, broadcast_msg);

    node_root.shutdown().await;
    node_worker1.shutdown().await;
    node_worker2.shutdown().await;
}

// ============================================================================
// 7. TCP Localhost Fallback Tests (Windows compatibility)
// ============================================================================

#[tokio::test]
async fn test_tcp_localhost_fallback_mode() {
    let temp = tempdir().expect("tempdir");

    // Explicitly configure force_tcp = true to test TCP fallback transport
    let options_a = MeshNodeOptions {
        run_dir: Some(temp.path().to_path_buf()),
        pid: Some(84001),
        active_model: "tcp-node-a".to_string(),
        force_tcp: true,
        ..Default::default()
    };

    let options_b = MeshNodeOptions {
        run_dir: Some(temp.path().to_path_buf()),
        pid: Some(84002),
        active_model: "tcp-node-b".to_string(),
        force_tcp: true,
        ..Default::default()
    };

    let node_a = MeshNode::with_options(options_a)
        .await
        .expect("start TCP node A");
    let node_b = MeshNode::with_options(options_b)
        .await
        .expect("start TCP node B");

    // Check that endpoints are TCP
    assert!(matches!(node_a.endpoint(), PeerEndpoint::Tcp(_)));
    assert!(matches!(node_b.endpoint(), PeerEndpoint::Tcp(_)));

    // Verify TCP address port file was written
    assert!(node_a.socket_path().exists());
    let content = std::fs::read_to_string(node_a.socket_path()).unwrap();
    assert!(content.starts_with("tcp:127.0.0.1:"));

    // Discovery over TCP
    let discovered = node_a.discover_peers().await.expect("discover over TCP");
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].peer_id, node_b.peer_id());

    // Ping / Pong over TCP
    let pong = node_a
        .ping(node_b.peer_id())
        .await
        .expect("ping over TCP");
    match pong {
        MeshMessage::Pong { active_model, .. } => {
            assert_eq!(active_model, "tcp-node-b");
        }
        other => panic!("Expected Pong, got {other:?}"),
    }

    node_a.shutdown().await;
    node_b.shutdown().await;
}

// ============================================================================
// 8. Stale Socket File Resilience Tests
// ============================================================================

#[tokio::test]
async fn test_stale_socket_cleanup_resilience() {
    let temp = tempdir().expect("tempdir");

    // Create a bogus stale socket/port file in the run directory
    let stale_path = temp.path().join("mesh_99999.sock");
    std::fs::write(&stale_path, "tcp:127.0.0.1:1\n").unwrap();

    let node = MeshNode::in_dir_with_pid(temp.path(), 85001, "resilient-node")
        .await
        .expect("start node");

    // Discovery should not crash or panic when encountering dead socket
    let discovered = node.discover_peers().await.expect("discover with stale file");
    assert_eq!(discovered.len(), 0);

    node.shutdown().await;
}
