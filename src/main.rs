use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

const USAGE: &str = r#"fe - cached fast access to QMD retrieval

Usage:
  fe "<query>" [qmd query options]          # cached qmd query --no-rerank
  fe fast-query "<query>" [qmd query opts]  # same as default
  fe query "<query>" [qmd query options]    # cached full qmd query
  fe search "<query>" [qmd search options]  # cached BM25/full-text
  fe bm25 "<query>" [qmd search options]    # alias for fe search
  fe vsearch "<query>" [qmd vsearch opts]   # cached vector-only
  fe qmd get '#docid:line:count' [opts]      # exact source slice passthrough
  fe --refresh <mode> ...                   # bypass read cache, store result
  fe --no-cache <mode> ...                  # bypass read/write cache
  fe --cache-stats
  fe --clear-cache
  fe qmd <command> [args...]                # raw qmd passthrough

Use fffctl/fff for fastest local filename/content discovery when exact or fuzzy
local lookup is enough. Use fe for QMD-backed BM25, hybrid, and vector lookup.
Use --format json with qmd 2.5+ commands when machine-readable output matters.
Use fe qmd get for qmd:// or #docid line ranges returned by indexed retrieval.
"#;

#[derive(Default)]
struct WrapperFlags {
    refresh: bool,
    no_cache: bool,
    cache_stats: bool,
    clear_cache: bool,
    help: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code as u8),
        Err(err) => {
            eprintln!("fe: {err}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<i32, Box<dyn std::error::Error>> {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let flags = parse_flags(&mut args);

    if flags.cache_stats {
        return cache_stats();
    }
    if flags.clear_cache {
        return clear_cache();
    }
    if flags.help || args.is_empty() {
        print!("{USAGE}");
        return Ok(0);
    }

    let qmd_force_cpu = env::var("QMD_FORCE_CPU").unwrap_or_else(|_| "1".to_string());

    if args[0] == "qmd" {
        let status = Command::new("qmd")
            .args(&args[1..])
            .env("QMD_FORCE_CPU", qmd_force_cpu)
            .status()?;
        return Ok(status.code().unwrap_or(1));
    }

    let qmd_args = qmd_args_for(&args);
    if flags.no_cache {
        return run_qmd(&qmd_args, &qmd_force_cpu);
    }

    let meta = cache_meta(&qmd_args, &qmd_force_cpu);
    let key = fnv1a64_hex(meta.as_bytes());
    let root = cache_root();
    fs::create_dir_all(&root)?;
    let out_path = root.join(format!("{key}.out"));
    let err_path = root.join(format!("{key}.err"));
    let code_path = root.join(format!("{key}.code"));
    let meta_path = root.join(format!("{key}.meta"));

    if !flags.refresh
        && out_path.exists()
        && err_path.exists()
        && code_path.exists()
        && meta_path.exists()
        && fs::read_to_string(&meta_path).unwrap_or_default() == meta
    {
        io::stdout().write_all(&fs::read(&out_path)?)?;
        io::stderr().write_all(&fs::read(&err_path)?)?;
        let code = fs::read_to_string(&code_path)?.trim().parse::<i32>()?;
        return Ok(code);
    }

    let output = Command::new("qmd")
        .args(&qmd_args)
        .env("QMD_FORCE_CPU", qmd_force_cpu)
        .output()?;
    io::stdout().write_all(&output.stdout)?;
    io::stderr().write_all(&output.stderr)?;
    let code = output.status.code().unwrap_or(1);

    if code == 0 {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let pid = std::process::id();
        let tmp_out = root.join(format!("{key}.{pid}.{stamp}.out.tmp"));
        let tmp_err = root.join(format!("{key}.{pid}.{stamp}.err.tmp"));
        let tmp_code = root.join(format!("{key}.{pid}.{stamp}.code.tmp"));
        let tmp_meta = root.join(format!("{key}.{pid}.{stamp}.meta.tmp"));
        fs::write(&tmp_out, &output.stdout)?;
        fs::write(&tmp_err, &output.stderr)?;
        fs::write(&tmp_code, code.to_string())?;
        fs::write(&tmp_meta, meta)?;
        fs::rename(tmp_out, out_path)?;
        fs::rename(tmp_err, err_path)?;
        fs::rename(tmp_code, code_path)?;
        fs::rename(tmp_meta, meta_path)?;
    }

    Ok(code)
}

/// Wrapper flags are only recognized before the first non-flag argument, so
/// flags and words like "help" that appear after the mode reach qmd untouched.
fn parse_flags(args: &mut Vec<String>) -> WrapperFlags {
    let mut flags = WrapperFlags::default();
    while !args.is_empty() {
        match args[0].as_str() {
            "--refresh" => flags.refresh = true,
            "--no-cache" => flags.no_cache = true,
            "--cache-stats" => flags.cache_stats = true,
            "--clear-cache" => flags.clear_cache = true,
            "-h" | "--help" | "help" => flags.help = true,
            _ => break,
        }
        args.remove(0);
    }
    flags
}

fn qmd_args_for(args: &[String]) -> Vec<String> {
    match args[0].as_str() {
        "search" | "vsearch" | "query" => args.to_vec(),
        "bm25" => {
            let mut out = vec!["search".to_string()];
            out.extend_from_slice(&args[1..]);
            out
        }
        "fast-query" => {
            let mut out = vec!["query".to_string(), "--no-rerank".to_string()];
            out.extend_from_slice(&args[1..]);
            out
        }
        _ => {
            let mut out = vec!["query".to_string(), "--no-rerank".to_string()];
            out.extend_from_slice(args);
            out
        }
    }
}

fn run_qmd(args: &[String], qmd_force_cpu: &str) -> Result<i32, Box<dyn std::error::Error>> {
    let status = Command::new("qmd")
        .args(args)
        .env("QMD_FORCE_CPU", qmd_force_cpu)
        .status()?;
    Ok(status.code().unwrap_or(1))
}

fn cache_root() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_CACHE_HOME") {
        PathBuf::from(xdg).join("qmd").join("fe-cache-v2")
    } else {
        home_dir().join(".cache").join("qmd").join("fe-cache-v2")
    }
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn qmd_home() -> PathBuf {
    env::var_os("QMD_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".cache").join("qmd"))
}

fn qmd_index_name(args: &[String]) -> String {
    for index in 0..args.len() {
        let arg = &args[index];
        if arg == "--index" && index + 1 < args.len() {
            return args[index + 1].clone();
        }
        if let Some(value) = arg.strip_prefix("--index=") {
            return value.to_string();
        }
    }
    env::var("QMD_INDEX").unwrap_or_else(|_| "index".to_string())
}

fn cache_meta(args: &[String], qmd_force_cpu: &str) -> String {
    let index_name = qmd_index_name(args);
    let index_file = qmd_home().join(format!("{index_name}.sqlite"));
    let qmd_bin = find_executable("qmd").unwrap_or_else(|| PathBuf::from("qmd"));
    let mut meta = String::new();
    meta.push_str("v=3\n");
    meta.push_str(&format!("qmd_bin={}\n", qmd_bin.display()));
    meta.push_str(&format!("qmd_stat={}\n", stat_fingerprint(&qmd_bin)));
    meta.push_str(&format!("index_name={index_name}\n"));
    meta.push_str(&format!("index_file={}\n", index_file.display()));
    meta.push_str(&format!(
        "index_fingerprint={}\n",
        qmd_index_fingerprint(&index_file)
    ));
    meta.push_str(&format!("QMD_FORCE_CPU={qmd_force_cpu}\n"));
    meta.push_str(&format!(
        "QMD_EDITOR_URI={}\n",
        env::var("QMD_EDITOR_URI").unwrap_or_default()
    ));
    meta.push_str("argv=");
    for arg in args {
        meta.push_str(arg);
        meta.push('\0');
    }
    meta
}

fn stat_fingerprint(path: &Path) -> String {
    match fs::metadata(path) {
        Ok(meta) => {
            let modified = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            format!("exists:size={}:mtime_ns={modified}", meta.len())
        }
        Err(_) => "missing".to_string(),
    }
}

fn qmd_index_fingerprint(path: &Path) -> String {
    if !path.exists() {
        return "missing".to_string();
    }
    // O(1)-ish summary instead of scanning every row: data_version changes on
    // any external write, and counts/max-timestamps catch content changes.
    let sql = r#"
PRAGMA data_version;
SELECT count(*), COALESCE(max(modified_at), '') FROM documents WHERE active = 1;
SELECT count(*), COALESCE(max(embedded_at), '') FROM content_vectors;
SELECT count(*) FROM store_collections;
"#;
    match Command::new("sqlite3").arg(path).arg(sql).output() {
        Ok(output) if output.status.success() => {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(
                format!(
                    "size={}\n",
                    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
                )
                .as_bytes(),
            );
            bytes.extend_from_slice(&output.stdout);
            format!("sqlite:{}", fnv1a64_hex(&bytes))
        }
        _ => format!("stat:{}", stat_fingerprint(path)),
    }
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let candidate = PathBuf::from(name);
    if candidate.components().count() > 1 && is_executable(&candidate) {
        return Some(candidate);
    }
    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn fnv1a64_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn cache_stats() -> Result<i32, Box<dyn std::error::Error>> {
    let root = cache_root();
    fs::create_dir_all(&root)?;
    let mut entries = 0u64;
    let mut bytes = 0u64;
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();
        // Remove tmp files orphaned by interrupted cache writes.
        if path.extension().and_then(|ext| ext.to_str()) == Some("tmp") {
            let _ = fs::remove_file(&path);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("out") {
            entries += 1;
        }
        bytes += entry.metadata()?.len();
    }
    println!(
        "{{\n  \"cache\": \"{}\",\n  \"entries\": {},\n  \"bytes\": {}\n}}",
        json_escape(&root.display().to_string()),
        entries,
        bytes
    );
    Ok(0)
}

fn clear_cache() -> Result<i32, Box<dyn std::error::Error>> {
    let root = cache_root();
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    fs::create_dir_all(&root)?;
    println!(
        "{{\n  \"cache\": \"{}\",\n  \"deleted\": true\n}}",
        json_escape(&root.display().to_string())
    );
    Ok(0)
}

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn bare_query_uses_fast_query_defaults() {
        assert_eq!(
            qmd_args_for(&strings(&["vector search", "--index", "memory"])),
            strings(&["query", "--no-rerank", "vector search", "--index", "memory"])
        );
    }

    #[test]
    fn fast_query_maps_to_qmd_query_no_rerank() {
        assert_eq!(
            qmd_args_for(&strings(&["fast-query", "agent memory"])),
            strings(&["query", "--no-rerank", "agent memory"])
        );
    }

    #[test]
    fn bm25_maps_to_search() {
        assert_eq!(
            qmd_args_for(&strings(&["bm25", "privacy"])),
            strings(&["search", "privacy"])
        );
    }

    #[test]
    fn explicit_modes_pass_through() {
        assert_eq!(
            qmd_args_for(&strings(&["search", "privacy", "-c", "docs"])),
            strings(&["search", "privacy", "-c", "docs"])
        );
        assert_eq!(
            qmd_args_for(&strings(&["query", "privacy"])),
            strings(&["query", "privacy"])
        );
        assert_eq!(
            qmd_args_for(&strings(&["vsearch", "privacy"])),
            strings(&["vsearch", "privacy"])
        );
    }

    #[test]
    fn parse_wrapper_flags_only_consumes_leading_flags() {
        let mut args = strings(&[
            "--refresh",
            "--no-cache",
            "search",
            "privacy",
            "--index",
            "memory",
        ]);
        let flags = parse_flags(&mut args);

        assert!(flags.refresh);
        assert!(flags.no_cache);
        assert_eq!(args, strings(&["search", "privacy", "--index", "memory"]));
    }

    #[test]
    fn parse_wrapper_flags_leaves_trailing_flags_for_qmd() {
        let mut args = strings(&["qmd", "query", "--help"]);
        let flags = parse_flags(&mut args);

        assert!(!flags.help);
        assert_eq!(args, strings(&["qmd", "query", "--help"]));
    }

    #[test]
    fn parse_wrapper_flags_leaves_help_as_query_text() {
        let mut args = strings(&["search", "help"]);
        let flags = parse_flags(&mut args);

        assert!(!flags.help);
        assert_eq!(args, strings(&["search", "help"]));
    }

    #[test]
    fn json_escape_handles_control_characters() {
        assert_eq!(json_escape("a\nb\x01c"), "a\\nb\\u0001c");
    }

    #[test]
    fn index_name_prefers_explicit_flag_forms() {
        assert_eq!(
            qmd_index_name(&strings(&["search", "privacy", "--index", "memory"])),
            "memory"
        );
        assert_eq!(
            qmd_index_name(&strings(&["search", "privacy", "--index=research"])),
            "research"
        );
    }

    #[test]
    fn fnv_hash_is_stable_for_cache_keys() {
        assert_eq!(fnv1a64_hex(b"fe"), "08985f07b541dd74");
    }

    #[test]
    fn json_escape_handles_path_text() {
        assert_eq!(json_escape(r#"a\b"c"#), r#"a\\b\"c"#);
    }
}
