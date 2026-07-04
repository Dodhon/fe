//! End-to-end regression tests using a stub `qmd` binary.
//!
//! Each test gets an isolated HOME/XDG_CACHE_HOME so cache state and the
//! stub qmd never touch the real environment.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

struct TestEnv {
    root: PathBuf,
    bin_dir: PathBuf,
    calls_file: PathBuf,
}

impl TestEnv {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("fe-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let bin_dir = root.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let calls_file = root.join("qmd-calls.log");

        // Stub qmd: logs its argv, echoes a deterministic result.
        let stub = format!(
            "#!/bin/sh\necho \"$@\" >> {}\necho \"result: $@\"\necho \"warn: $@\" >&2\n",
            calls_file.display()
        );
        let qmd_path = bin_dir.join("qmd");
        fs::write(&qmd_path, stub).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&qmd_path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self {
            root,
            bin_dir,
            calls_file,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.bin_dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(env!("CARGO_BIN_EXE_fe"))
            .args(args)
            .env("PATH", path)
            .env("HOME", &self.root)
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env_remove("QMD_INDEX")
            .env_remove("QMD_HOME")
            .env_remove("QMD_EDITOR_URI")
            .output()
            .unwrap()
    }

    fn qmd_call_count(&self) -> usize {
        fs::read_to_string(&self.calls_file)
            .unwrap_or_default()
            .lines()
            .count()
    }

    fn cache_dir(&self) -> PathBuf {
        self.root.join("cache").join("qmd").join("fe-cache-v2")
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn cache_hit_replays_output_without_rerunning_qmd() {
    let env = TestEnv::new("cache-hit");

    let first = env.run(&["search", "alpha"]);
    assert!(first.status.success());
    assert_eq!(stdout(&first), "result: search alpha\n");
    assert_eq!(env.qmd_call_count(), 1);

    let second = env.run(&["search", "alpha"]);
    assert!(second.status.success());
    assert_eq!(stdout(&second), stdout(&first));
    assert_eq!(
        String::from_utf8_lossy(&second.stderr),
        "warn: search alpha\n"
    );
    assert_eq!(env.qmd_call_count(), 1, "cache hit must not invoke qmd");
}

#[test]
fn different_queries_get_different_cache_entries() {
    let env = TestEnv::new("distinct-keys");
    env.run(&["search", "alpha"]);
    env.run(&["search", "beta"]);
    assert_eq!(env.qmd_call_count(), 2);
    assert_eq!(
        stdout(&env.run(&["search", "beta"])),
        "result: search beta\n"
    );
    assert_eq!(env.qmd_call_count(), 2);
}

#[test]
fn refresh_bypasses_read_but_updates_cache() {
    let env = TestEnv::new("refresh");
    env.run(&["search", "alpha"]);
    env.run(&["--refresh", "search", "alpha"]);
    assert_eq!(env.qmd_call_count(), 2, "--refresh must rerun qmd");
    env.run(&["search", "alpha"]);
    assert_eq!(env.qmd_call_count(), 2, "refreshed result must be cached");
}

#[test]
fn no_cache_neither_reads_nor_writes() {
    let env = TestEnv::new("no-cache");
    env.run(&["--no-cache", "search", "alpha"]);
    env.run(&["--no-cache", "search", "alpha"]);
    assert_eq!(env.qmd_call_count(), 2);
    let entries = fs::read_dir(env.cache_dir())
        .map(|dir| dir.count())
        .unwrap_or(0);
    assert_eq!(entries, 0, "--no-cache must not write cache files");
}

#[test]
fn bare_query_defaults_to_fast_query() {
    let env = TestEnv::new("bare-query");
    let output = env.run(&["some query"]);
    assert_eq!(stdout(&output), "result: query --no-rerank some query\n");
}

#[test]
fn qmd_passthrough_forwards_flags_untouched() {
    let env = TestEnv::new("passthrough");
    let output = env.run(&["qmd", "query", "--help"]);
    assert!(output.status.success());
    assert_eq!(stdout(&output), "result: query --help\n");
}

#[test]
fn help_word_after_mode_is_treated_as_query_text() {
    let env = TestEnv::new("help-query");
    let output = env.run(&["search", "help"]);
    assert_eq!(stdout(&output), "result: search help\n");
}

#[test]
fn nonzero_exit_is_propagated_and_not_cached() {
    let env = TestEnv::new("failure");
    let qmd = env.bin_dir.join("qmd");
    fs::write(&qmd, "#!/bin/sh\necho boom >&2\nexit 3\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&qmd, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let output = env.run(&["search", "alpha"]);
    assert_eq!(output.status.code(), Some(3));
    let cached = fs::read_dir(env.cache_dir())
        .map(|dir| {
            dir.filter(|entry| {
                entry
                    .as_ref()
                    .map(|e| e.path().extension().is_some_and(|ext| ext == "out"))
                    .unwrap_or(false)
            })
            .count()
        })
        .unwrap_or(0);
    assert_eq!(cached, 0, "failed runs must not be cached");
}

#[test]
fn index_change_invalidates_cache() {
    let env = TestEnv::new("index-invalidation");
    let qmd_home = env.root.join(".cache").join("qmd");
    fs::create_dir_all(&qmd_home).unwrap();
    let index = qmd_home.join("index.sqlite");

    let make_index = |doc_time: &str| {
        let conn = rusqlite::Connection::open(&index).unwrap();
        conn.execute_batch(&format!(
            "DROP TABLE IF EXISTS documents;
             DROP TABLE IF EXISTS content_vectors;
             DROP TABLE IF EXISTS store_collections;
             CREATE TABLE documents (collection TEXT, path TEXT, hash TEXT, modified_at TEXT, active INT);
             CREATE TABLE content_vectors (hash TEXT, seq INT, model TEXT, embed_fingerprint TEXT, embedded_at TEXT);
             CREATE TABLE store_collections (name TEXT, path TEXT, pattern TEXT, context TEXT);
             INSERT INTO documents VALUES ('c', 'p', 'h', '{doc_time}', 1);"
        ))
        .unwrap();
    };

    make_index("2026-01-01T00:00:00Z");
    env.run(&["search", "alpha"]);
    env.run(&["search", "alpha"]);
    assert_eq!(env.qmd_call_count(), 1, "unchanged index should hit cache");

    make_index("2026-02-02T00:00:00Z");
    env.run(&["search", "alpha"]);
    assert_eq!(
        env.qmd_call_count(),
        2,
        "index change must invalidate cache"
    );
}

#[test]
fn cache_stats_reports_entries_and_sweeps_tmp_files() {
    let env = TestEnv::new("stats");
    env.run(&["search", "alpha"]);
    let orphan = env.cache_dir().join("deadbeef.123.456.out.tmp");
    fs::write(&orphan, "orphan").unwrap();

    let output = env.run(&["--cache-stats"]);
    let text = stdout(&output);
    assert!(text.contains("\"entries\": 1"), "stats output: {text}");
    assert!(!orphan.exists(), "orphaned tmp files must be swept");
}

#[test]
fn clear_cache_empties_directory() {
    let env = TestEnv::new("clear");
    env.run(&["search", "alpha"]);
    env.run(&["--clear-cache"]);
    let entries = fs::read_dir(env.cache_dir()).unwrap().count();
    assert_eq!(entries, 0);
    env.run(&["search", "alpha"]);
    assert_eq!(env.qmd_call_count(), 2, "cleared cache must miss");
}
