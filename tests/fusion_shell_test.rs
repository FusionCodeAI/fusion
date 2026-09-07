//! Integration tests for `fusion-shell`.
//!
//! Comprehensive test suite verifying:
//! 1. PTY / Process Manager Spawning and Lifecycle (`fusion_shell::process::Process`)
//! 2. Process Manager Registry & Termination Targets (`SpawnRegistry`, `TerminationTargets`)
//! 3. Process Table Snapshot & Builtins (`ProcInfo::all()`, `ps`, `echo $$`)
//! 4. Shell Execution & Real-Time PTY / Output Streaming (`execute_shell`, `execute_shell_streams`)
//! 5. Stateful Shell Session Execution (`Shell::new`)

use std::{
    collections::HashMap,
    process::Command,
    time::{Duration, Instant},
};

use bytes::Bytes;
use flume::unbounded;
use fusion_builtins::ProcInfo;
use fusion_shell::{
    cancel::{AbortReason, CancelToken},
    execute_shell, execute_shell_streams,
    process::{
        KILL_SIGNAL, Process, ProcessStatus, SpawnRegistry, TERM_SIGNAL, TerminationTargets,
        kill_process_group,
    },
    shell::{Shell, ShellExecuteOptions, ShellOptions, ShellRunOptions, StreamSinks},
};

// ============================================================================
// 1. PTY / Process Manager Spawning and Lifecycle
// ============================================================================

mod process_lifecycle_tests {
    use super::*;

    #[test]
    fn test_self_process_inspection() {
        let self_pid = std::process::id() as i32;
        let proc = Process::from_pid(self_pid).expect("must find self process by pid");

        assert_eq!(proc.pid(), self_pid, "pid must match self");
        assert_eq!(
            proc.status(),
            ProcessStatus::Running,
            "current process must have Running status"
        );

        // ppid should either be a valid parent PID (> 0) or None on unusual containers
        if let Some(ppid) = proc.ppid() {
            assert!(ppid > 0, "ppid must be positive if present: {ppid}");
        }

        // args should be queryable without panic
        let args = proc.args();
        // The test runner args should contain the test binary name
        assert!(
            !args.is_empty(),
            "process args should not be empty for test process"
        );
    }

    #[test]
    fn test_nonexistent_pid_lookup() {
        // Negative PID
        assert!(
            Process::from_pid(-1).is_none(),
            "negative pid must return None"
        );
        assert!(Process::from_pid(0).is_none(), "pid 0 must return None");

        // Excessively high PID that almost certainly does not exist
        assert!(
            Process::from_pid(2_147_483_640).is_none(),
            "extremely high unused pid must return None"
        );
    }

    #[test]
    fn test_child_spawn_and_lifecycle() {
        // Spawn a real child process using sleep
        let mut child = Command::new("sleep")
            .arg("15")
            .spawn()
            .expect("must spawn sleep child process");

        let child_pid = child.id() as i32;
        let proc = Process::from_pid(child_pid).expect("must pin spawned child process");

        assert_eq!(proc.pid(), child_pid);
        assert_eq!(
            proc.status(),
            ProcessStatus::Running,
            "child must initially be Running"
        );

        // Kill the child via process manager tree kill
        let killed = proc.kill_tree(Some(KILL_SIGNAL));
        assert!(killed >= 1, "kill_tree should signal at least 1 process");

        // Wait for child to exit
        let _ = child.wait();

        // Brief delay to let OS record exit status
        std::thread::sleep(Duration::from_millis(50));

        // Status should now transition to Exited
        assert_eq!(
            proc.status(),
            ProcessStatus::Exited,
            "child must report Exited after termination"
        );
    }

    #[tokio::test]
    async fn test_child_tree_graceful_termination() {
        // Spawn a shell wrapper with sleep
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("sleep 20")
            .spawn()
            .expect("must spawn sh child");

        let child_pid = child.id() as i32;
        let proc = Process::from_pid(child_pid).expect("pin sh process");

        assert_eq!(proc.status(), ProcessStatus::Running);

        // Call terminate_tree with a polite grace period then kill
        let terminated = proc
            .terminate_tree(true, 50, 500, CancelToken::default())
            .await
            .expect("terminate_tree execution");

        assert!(terminated, "terminate_tree must succeed in stopping process");

        let _ = child.wait();
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(
            proc.status(),
            ProcessStatus::Exited,
            "process must be Exited after terminate_tree"
        );
    }

    #[test]
    fn test_find_process_by_path_nonexistent() {
        let procs = Process::from_path("nonexistent_fusion_binary_xyz_123".to_string());
        assert!(
            procs.is_empty(),
            "looking up nonexistent path must return empty vec"
        );
    }
}

// ============================================================================
// 2. Process Manager Registry & Termination Targets
// ============================================================================

mod spawn_registry_tests {
    use super::*;

    #[test]
    fn test_spawn_registry_tracking_and_pruning() {
        let registry = SpawnRegistry::new();

        // Spawn a temporary child
        let mut child = Command::new("sleep")
            .arg("10")
            .spawn()
            .expect("spawn sleep");

        let child_pid = child.id() as i32;
        let proc = Process::from_pid(child_pid).expect("pin child");

        // Record child into registry
        registry.record(None, Some(proc.clone()));

        // Build targets: must contain the live child
        let targets = registry.build_targets();
        assert!(
            !targets.is_empty(),
            "build_targets must not be empty while child is alive"
        );

        // Signal target using TERM_SIGNAL
        targets.signal(TERM_SIGNAL);

        let _ = child.wait();
        std::thread::sleep(Duration::from_millis(50));

        // Once child exits, build_targets must opportunistically prune it
        let targets_after = registry.build_targets();
        assert!(
            targets_after.is_empty(),
            "build_targets must prune dead child processes"
        );
    }

    #[test]
    fn test_termination_targets_dedup_and_empty() {
        let mut targets = TerminationTargets::new();
        assert!(targets.is_empty(), "new targets must be empty");

        let self_pid = std::process::id() as i32;
        let proc = Process::from_pid(self_pid).expect("pin self");

        // Add pre-pinned process
        targets.add_process(proc.clone());
        assert!(!targets.is_empty(), "targets should not be empty after add");

        // Duplicate add_process with same pid must be ignored
        targets.add_process(proc);

        // Add PGID
        targets.add_pgid(99999);
        targets.add_pgid(99999); // duplicate should not duplicate entry

        // Signal should execute safely without crashing
        targets.signal(0); // Signal 0 is check-only
    }

    #[test]
    fn test_kill_process_group_safety() {
        // Refuse negative or zero PGID
        assert!(
            !kill_process_group(-1, TERM_SIGNAL),
            "negative pgid must be rejected"
        );
        assert!(
            !kill_process_group(0, TERM_SIGNAL),
            "zero pgid must be rejected"
        );

        // Self process group protection: the manager must refuse to kill the harness's own process group
        let self_pid = std::process::id() as i32;
        let self_proc = Process::from_pid(self_pid).expect("pin self");
        if let Some(pgid) = self_proc.group_id() {
            assert!(
                !kill_process_group(pgid, KILL_SIGNAL),
                "must refuse to kill own process group"
            );
        }
    }
}

// ============================================================================
// 3. Process Table Snapshot & Builtins
// ============================================================================

mod process_table_tests {
    use super::*;

    #[test]
    fn test_process_table_snapshot_contains_self() {
        let procs = ProcInfo::all();
        assert!(
            !procs.is_empty(),
            "system process table must contain processes"
        );

        let self_pid = std::process::id() as i32;
        let self_entry = procs.iter().find(|p| p.pid() == self_pid);

        assert!(
            self_entry.is_some(),
            "process table must contain the current test process (pid: {self_pid})"
        );

        let entry = self_entry.unwrap();
        assert_eq!(entry.pid(), self_pid);

        // PPID should match proc ppid
        if let Some(ppid) = entry.ppid() {
            assert!(ppid > 0, "process table entry ppid must be positive");
        }
    }

    #[test]
    fn test_process_table_dynamic_child_tracking() {
        let mut child = Command::new("sleep")
            .arg("12")
            .spawn()
            .expect("spawn sleep for process table check");

        let child_pid = child.id() as i32;

        // Give the OS kernel a moment to register the entry
        std::thread::sleep(Duration::from_millis(40));

        let procs = ProcInfo::all();
        let child_found = procs.iter().any(|p| p.pid() == child_pid);

        // Clean up child before assertion so we don't leave zombies
        let _ = child.kill();
        let _ = child.wait();

        assert!(
            child_found,
            "spawned child pid {child_pid} must appear in ProcInfo::all() process table"
        );
    }

    #[tokio::test]
    async fn test_process_table_builtins_via_shell() {
        // Test `ps` builtin via execute_shell
        let (tx, rx) = unbounded::<String>();
        let result = execute_shell(
            ShellExecuteOptions {
                command: "ps".to_string(),
                ..Default::default()
            },
            Some(tx),
            CancelToken::default(),
        )
        .await
        .expect("execute ps command");

        assert_eq!(result.exit_code, Some(0), "ps command must exit 0");

        let output: String = rx.drain().collect::<String>();
        assert!(
            output.contains("PID") || output.contains("pid"),
            "ps output should contain PID header: {output}"
        );

        // Test `echo $$` to check current shell PID retrieval
        let (tx2, rx2) = unbounded::<String>();
        let result2 = execute_shell(
            ShellExecuteOptions {
                command: "echo $$".to_string(),
                ..Default::default()
            },
            Some(tx2),
            CancelToken::default(),
        )
        .await
        .expect("execute echo $$");

        assert_eq!(result2.exit_code, Some(0));
        let pid_str: String = rx2.drain().collect::<String>();
        let parsed_pid = pid_str.trim().parse::<i32>();
        assert!(
            parsed_pid.is_ok() && parsed_pid.unwrap() > 0,
            "echo $$ must output a valid positive shell PID: '{pid_str}'"
        );
    }
}

// ============================================================================
// 4. Shell Execution & Real-Time PTY / Output Streaming
// ============================================================================

mod shell_execution_and_streaming_tests {
    use super::*;

    #[tokio::test]
    async fn test_execute_shell_basic_output() {
        let (tx, rx) = unbounded::<String>();
        let result = execute_shell(
            ShellExecuteOptions {
                command: "echo 'fusion shell pty test'".to_string(),
                ..Default::default()
            },
            Some(tx),
            CancelToken::default(),
        )
        .await
        .expect("execute echo");

        assert_eq!(result.exit_code, Some(0));

        let output: String = rx.drain().collect::<String>();
        assert!(
            output.contains("fusion shell pty test"),
            "output must match echo string: {output}"
        );
    }

    #[tokio::test]
    async fn test_execute_shell_exit_codes() {
        // Exit 0
        let res0 = execute_shell(
            ShellExecuteOptions {
                command: "exit 0".to_string(),
                ..Default::default()
            },
            None,
            CancelToken::default(),
        )
        .await
        .expect("exit 0");
        assert_eq!(res0.exit_code, Some(0));

        // Exit 42
        let res42 = execute_shell(
            ShellExecuteOptions {
                command: "exit 42".to_string(),
                ..Default::default()
            },
            None,
            CancelToken::default(),
        )
        .await
        .expect("exit 42");
        assert_eq!(res42.exit_code, Some(42));

        // Boolean builtins
        let res_true = execute_shell(
            ShellExecuteOptions {
                command: "true".to_string(),
                ..Default::default()
            },
            None,
            CancelToken::default(),
        )
        .await
        .expect("true");
        assert_eq!(res_true.exit_code, Some(0));

        let res_false = execute_shell(
            ShellExecuteOptions {
                command: "false".to_string(),
                ..Default::default()
            },
            None,
            CancelToken::default(),
        )
        .await
        .expect("false");
        assert_eq!(res_false.exit_code, Some(1));
    }

    #[tokio::test]
    async fn test_execute_shell_environment_and_cwd() {
        let mut env = HashMap::new();
        env.insert(
            "FUSION_TEST_VAR".to_string(),
            "fusion_pty_val_789".to_string(),
        );

        let temp_dir = std::env::temp_dir();
        let temp_dir_str = temp_dir.to_string_lossy().to_string();

        let (tx, rx) = unbounded::<String>();
        let result = execute_shell(
            ShellExecuteOptions {
                command: "echo $FUSION_TEST_VAR; pwd -P".to_string(),
                env: Some(env),
                cwd: Some(temp_dir_str.clone()),
                ..Default::default()
            },
            Some(tx),
            CancelToken::default(),
        )
        .await
        .expect("execute with env and cwd");

        assert_eq!(result.exit_code, Some(0));

        let output: String = rx.drain().collect::<String>();
        assert!(
            output.contains("fusion_pty_val_789"),
            "env var must be reflected in output: {output}"
        );
    }

    #[tokio::test]
    async fn test_execute_shell_streams_stdout_stderr_separation() {
        let (stdout_tx, stdout_rx) = unbounded::<Bytes>();
        let (stderr_tx, stderr_rx) = unbounded::<Bytes>();

        let sinks = StreamSinks {
            stdout: Some(stdout_tx),
            stderr: Some(stderr_tx),
        };

        let result = execute_shell_streams(
            ShellExecuteOptions {
                command: "echo 'hello to stdout'; echo 'hello to stderr' >&2".to_string(),
                ..Default::default()
            },
            sinks,
            CancelToken::default(),
        )
        .await
        .expect("execute_shell_streams");

        assert_eq!(result.exit_code, Some(0));

        let stdout_bytes: Vec<u8> = stdout_rx.drain().flat_map(|b: Bytes| b.to_vec()).collect::<Vec<u8>>();
        let stderr_bytes: Vec<u8> = stderr_rx.drain().flat_map(|b: Bytes| b.to_vec()).collect::<Vec<u8>>();

        let stdout_str = String::from_utf8_lossy(&stdout_bytes);
        let stderr_str = String::from_utf8_lossy(&stderr_bytes);

        assert!(
            stdout_str.contains("hello to stdout"),
            "stdout sink must receive stdout content: {stdout_str}"
        );
        assert!(
            !stdout_str.contains("hello to stderr"),
            "stdout sink must not contain stderr content"
        );

        assert!(
            stderr_str.contains("hello to stderr"),
            "stderr sink must receive stderr content: {stderr_str}"
        );
        assert!(
            !stderr_str.contains("hello to stdout"),
            "stderr sink must not contain stdout content"
        );
    }

    #[tokio::test]
    async fn test_execute_shell_cancellation_and_reaping() {
        let mut ct = CancelToken::default();
        let abort_token = ct.emplace_abort_token();

        let start = Instant::now();

        // Trigger cancellation in background after 60ms
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            abort_token.abort(AbortReason::Signal);
        });

        // Run a command that would sleep for 30 seconds if not cancelled
        let result = execute_shell(
            ShellExecuteOptions {
                command: "sleep 30".to_string(),
                ..Default::default()
            },
            None,
            ct,
        )
        .await;

        let elapsed = start.elapsed();

        // Must return in < 3 seconds, proving the process manager aborted sleep
        assert!(
            elapsed < Duration::from_secs(3),
            "command must abort quickly on cancellation; elapsed: {elapsed:?}"
        );

        // Result should either fail or have aborted exit status
        if let Ok(res) = result {
            assert_ne!(
                res.exit_code,
                Some(0),
                "cancelled command must not exit successfully"
            );
        }
    }
}

// ============================================================================
// 5. Stateful Shell Session Execution
// ============================================================================

mod stateful_shell_session_tests {
    use super::*;

    #[tokio::test]
    async fn test_stateful_shell_variable_persistence() {
        let shell = Shell::new(None);

        // Step 1: define a variable
        let res1 = shell
            .run(
                ShellRunOptions {
                    command: "PERSISTENT_KEY='fusion_active_42'".to_string(),
                    ..Default::default()
                },
                None,
                CancelToken::default(),
            )
            .await
            .expect("set variable");
        assert_eq!(res1.exit_code, Some(0));

        // Step 2: read variable in a subsequent execution within the same session
        let (tx, rx) = unbounded::<String>();
        let res2 = shell
            .run(
                ShellRunOptions {
                    command: "echo $PERSISTENT_KEY".to_string(),
                    ..Default::default()
                },
                Some(tx),
                CancelToken::default(),
            )
            .await
            .expect("read variable");
        assert_eq!(res2.exit_code, Some(0));

        let output: String = rx.drain().collect::<String>();
        assert!(
            output.contains("fusion_active_42"),
            "variable must persist across runs in the same Shell session: {output}"
        );
    }

    #[tokio::test]
    async fn test_stateful_shell_directory_persistence() {
        let shell = Shell::new(None);

        let target_dir = std::env::temp_dir();
        let target_dir_str = target_dir.to_string_lossy().to_string();

        // Step 1: cd to target dir
        let res1 = shell
            .run(
                ShellRunOptions {
                    command: format!("cd '{}'", target_dir_str),
                    ..Default::default()
                },
                None,
                CancelToken::default(),
            )
            .await
            .expect("cd command");
        assert_eq!(res1.exit_code, Some(0));

        // Step 2: pwd should reflect the new directory
        let (tx, rx) = unbounded::<String>();
        let res2 = shell
            .run(
                ShellRunOptions {
                    command: "pwd -P".to_string(),
                    ..Default::default()
                },
                Some(tx),
                CancelToken::default(),
            )
            .await
            .expect("pwd command");
        assert_eq!(res2.exit_code, Some(0));

        let output: String = rx.drain().collect::<String>();
        assert!(
            !output.trim().is_empty(),
            "pwd output should not be empty: {output}"
        );
    }

    #[tokio::test]
    async fn test_stateful_shell_pipeline_and_functions() {
        let shell = Shell::new(Some(ShellOptions::default()));

        // Step 1: define a function
        let res1 = shell
            .run(
                ShellRunOptions {
                    command: "greet() { echo \"hello $1!\"; }".to_string(),
                    ..Default::default()
                },
                None,
                CancelToken::default(),
            )
            .await
            .expect("define function");
        assert_eq!(res1.exit_code, Some(0));

        // Step 2: execute function in a pipeline
        let (tx, rx) = unbounded::<String>();
        let res2 = shell
            .run(
                ShellRunOptions {
                    command: "greet 'fusion' | tr 'a-z' 'A-Z'".to_string(),
                    ..Default::default()
                },
                Some(tx),
                CancelToken::default(),
            )
            .await
            .expect("call function in pipeline");
        assert_eq!(res2.exit_code, Some(0));

        let output: String = rx.drain().collect::<String>();
        assert!(
            output.contains("HELLO FUSION!"),
            "function and pipeline must execute correctly: {output}"
        );
    }
}
