use std::env;
use std::ffi::CString;
use std::fs;
use std::io::Write;
use std::os::unix::process::parent_id;
use std::path::{Path, PathBuf};
use std::process;

use serde_json::Value;

unsafe extern "C" {
    fn notify_post(name: *const std::ffi::c_char) -> u32;
}

const NOTIFICATION_NAME: &str = "com.burakcokyildirim.clawde.session-changed";
const MAX_PID_WALK: usize = 8;

fn post_darwin_notification() {
    let name = CString::new(NOTIFICATION_NAME).expect("invalid notification name");
    unsafe {
        notify_post(name.as_ptr());
    }
}

fn get_ppid_of(pid: u32) -> Option<u32> {
    // proc_bsdinfo is 136 bytes on both arm64 and x86_64.
    // pbi_ppid is at offset 24 (u32).
    const PROC_PIDTBSDINFO: libc::c_int = 3;
    const PROC_PIDTBSDINFO_SIZE: libc::c_int = 136;

    let mut info = vec![0u8; PROC_PIDTBSDINFO_SIZE as usize];

    unsafe extern "C" {
        fn proc_pidinfo(
            pid: libc::c_int,
            flavor: libc::c_int,
            arg: u64,
            buffer: *mut libc::c_void,
            buffersize: libc::c_int,
        ) -> libc::c_int;
    }

    let ret = unsafe {
        proc_pidinfo(
            pid as libc::c_int,
            PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr() as *mut libc::c_void,
            PROC_PIDTBSDINFO_SIZE,
        )
    };

    if ret <= 0 {
        return None;
    }

    // pbi_ppid is at offset 24 in proc_bsdinfo (u32, native endian).
    let ppid = u32::from_ne_bytes([info[24], info[25], info[26], info[27]]);
    if ppid == 0 { None } else { Some(ppid) }
}

fn projects_dir() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".claude").join("projects")
}

fn find_cstatus_for_pid(projects_dir: &Path, pid: u32) -> Option<PathBuf> {
    let entries = match fs::read_dir(projects_dir) {
        Ok(e) => e,
        Err(_) => return None,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let sub_entries = match fs::read_dir(&path) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for sub_entry in sub_entries.flatten() {
            let file_path = sub_entry.path();
            if file_path.extension().and_then(|e| e.to_str()) != Some("cstatus") {
                continue;
            }
            let contents = match fs::read_to_string(&file_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let parsed: Value = match serde_json::from_str(&contents) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(file_pid) = parsed.get("pid").and_then(|v| v.as_u64())
                && file_pid == pid as u64
            {
                return Some(file_path);
            }
        }
    }
    None
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let dir = path
        .parent()
        .expect("cstatus file must have a parent directory");
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(data)?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_help() {
    eprintln!("set-session-name {VERSION}");
    eprintln!("Set a custom display name for the current Claude Code session");
    eprintln!();
    eprintln!("USAGE:");
    eprintln!("  set-session-name <session-name>");
    eprintln!();
    eprintln!("OPTIONS:");
    eprintln!("  -h, --help       Print this help message");
    eprintln!("  -V, --version    Print version");
    eprintln!();
    eprintln!("The session name is written to the .cstatus file for the current session,");
    eprintln!("identified by walking the process tree from CLAUDE_PID (or parent PID).");
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        eprintln!("set-session-name {VERSION}");
        return Ok(());
    }

    if args.len() < 2 || args[1].is_empty() {
        print_help();
        return Err("missing required argument: <session-name>".to_string());
    }
    let session_name = &args[1];

    let claude_pid: u32 = env::var("CLAUDE_PID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(parent_id);

    let projects = projects_dir();
    let mut current_pid = claude_pid;
    let mut cstatus_path: Option<PathBuf> = None;

    for _ in 0..MAX_PID_WALK {
        if current_pid <= 1 {
            break;
        }
        if let Some(path) = find_cstatus_for_pid(&projects, current_pid) {
            cstatus_path = Some(path);
            break;
        }
        match get_ppid_of(current_pid) {
            Some(ppid) if ppid > 0 => current_pid = ppid,
            _ => break,
        }
    }

    let cstatus_path =
        cstatus_path.ok_or("Error: Could not find .cstatus file for any ancestor PID")?;

    let contents = fs::read_to_string(&cstatus_path)
        .map_err(|e| format!("Error: Could not read {}: {}", cstatus_path.display(), e))?;

    let mut data: Value = serde_json::from_str(&contents)
        .map_err(|e| format!("Error: Could not parse {}: {}", cstatus_path.display(), e))?;

    data["session_name"] = Value::String(session_name.clone());

    let mut serialized = serde_json::to_string(&data)
        .map_err(|e| format!("Error: Could not serialize JSON: {}", e))?;
    serialized.push('\n');

    write_atomic(&cstatus_path, serialized.as_bytes())
        .map_err(|e| format!("Error: Could not write {}: {}", cstatus_path.display(), e))?;

    post_darwin_notification();
    println!("Session name set to: {}", session_name);
    Ok(())
}

fn main() {
    if let Err(msg) = run() {
        eprintln!("{}", msg);
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_projects_with_cstatus(pid: u32) -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path().join("-Volumes-Source-test-project");
        fs::create_dir_all(&project_dir).unwrap();
        let cstatus = project_dir.join("abc123.cstatus");
        let data = serde_json::json!({
            "session_id": "abc123",
            "pid": pid,
            "ppid": 1,
            "state": "active",
            "activity": "thinking",
            "timestamp": "2026-03-13T12:00:00Z",
            "cwd": "/tmp/project",
            "event": "UserPromptSubmit"
        });
        fs::write(&cstatus, serde_json::to_string(&data).unwrap() + "\n").unwrap();
        (tmp, cstatus)
    }

    #[test]
    fn find_cstatus_matches_pid() {
        let (tmp, expected_path) = make_projects_with_cstatus(12345);
        let result = find_cstatus_for_pid(tmp.path(), 12345);
        assert_eq!(result, Some(expected_path));
    }

    #[test]
    fn find_cstatus_no_match() {
        let (tmp, _) = make_projects_with_cstatus(12345);
        let result = find_cstatus_for_pid(tmp.path(), 99999);
        assert!(result.is_none());
    }

    #[test]
    fn find_cstatus_missing_dir() {
        let result = find_cstatus_for_pid(Path::new("/nonexistent/path"), 12345);
        assert!(result.is_none());
    }

    #[test]
    fn find_cstatus_skips_malformed_json() {
        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path().join("-test-project");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("bad.cstatus"), "not json\n").unwrap();
        let result = find_cstatus_for_pid(tmp.path(), 12345);
        assert!(result.is_none());
    }

    #[test]
    fn find_cstatus_skips_missing_pid_field() {
        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path().join("-test-project");
        fs::create_dir_all(&project_dir).unwrap();
        let data = serde_json::json!({"session_id": "abc", "state": "active"});
        fs::write(
            project_dir.join("nopid.cstatus"),
            serde_json::to_string(&data).unwrap(),
        )
        .unwrap();
        let result = find_cstatus_for_pid(tmp.path(), 12345);
        assert!(result.is_none());
    }

    #[test]
    fn write_atomic_creates_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.cstatus");
        write_atomic(&path, b"hello\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello\n");
    }

    #[test]
    fn write_atomic_overwrites_existing() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.cstatus");
        fs::write(&path, "old content").unwrap();
        write_atomic(&path, b"new content\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new content\n");
    }

    #[test]
    fn session_name_inserted_into_cstatus() {
        let (_tmp, cstatus_path) = make_projects_with_cstatus(12345);

        let contents = fs::read_to_string(&cstatus_path).unwrap();
        let mut data: Value = serde_json::from_str(&contents).unwrap();
        data["session_name"] = Value::String("My Session".to_string());
        let mut serialized = serde_json::to_string(&data).unwrap();
        serialized.push('\n');
        write_atomic(&cstatus_path, serialized.as_bytes()).unwrap();

        let result: Value =
            serde_json::from_str(&fs::read_to_string(&cstatus_path).unwrap()).unwrap();
        assert_eq!(result["session_name"], "My Session");
        assert_eq!(result["session_id"], "abc123");
        assert_eq!(result["pid"], 12345);
        assert_eq!(result["state"], "active");
    }

    #[test]
    fn session_name_overwrites_existing_name() {
        let (_tmp, cstatus_path) = make_projects_with_cstatus(12345);

        // Set initial name
        let contents = fs::read_to_string(&cstatus_path).unwrap();
        let mut data: Value = serde_json::from_str(&contents).unwrap();
        data["session_name"] = Value::String("Old Name".to_string());
        fs::write(&cstatus_path, serde_json::to_string(&data).unwrap()).unwrap();

        // Overwrite with new name
        let contents = fs::read_to_string(&cstatus_path).unwrap();
        let mut data: Value = serde_json::from_str(&contents).unwrap();
        data["session_name"] = Value::String("New Name".to_string());
        let mut serialized = serde_json::to_string(&data).unwrap();
        serialized.push('\n');
        write_atomic(&cstatus_path, serialized.as_bytes()).unwrap();

        let result: Value =
            serde_json::from_str(&fs::read_to_string(&cstatus_path).unwrap()).unwrap();
        assert_eq!(result["session_name"], "New Name");
    }

    #[test]
    fn get_ppid_of_returns_some_for_current_process() {
        let my_pid = process::id();
        let ppid = get_ppid_of(my_pid);
        assert!(ppid.is_some());
        assert!(ppid.unwrap() > 0);
    }

    #[test]
    fn get_ppid_of_returns_none_for_nonexistent_pid() {
        // Use a very high PID that almost certainly doesn't exist
        let ppid = get_ppid_of(4_000_000);
        assert!(ppid.is_none());
    }

    #[test]
    fn find_cstatus_multiple_projects() {
        let tmp = TempDir::new().unwrap();

        let dir_a = tmp.path().join("-project-a");
        fs::create_dir_all(&dir_a).unwrap();
        let data_a = serde_json::json!({"session_id": "aaa", "pid": 111, "state": "active"});
        fs::write(
            dir_a.join("aaa.cstatus"),
            serde_json::to_string(&data_a).unwrap(),
        )
        .unwrap();

        let dir_b = tmp.path().join("-project-b");
        fs::create_dir_all(&dir_b).unwrap();
        let data_b = serde_json::json!({"session_id": "bbb", "pid": 222, "state": "idle"});
        fs::write(
            dir_b.join("bbb.cstatus"),
            serde_json::to_string(&data_b).unwrap(),
        )
        .unwrap();

        let result = find_cstatus_for_pid(tmp.path(), 111);
        assert!(result.is_some());
        let found: Value =
            serde_json::from_str(&fs::read_to_string(result.unwrap()).unwrap()).unwrap();
        assert_eq!(found["session_id"], "aaa");

        let result = find_cstatus_for_pid(tmp.path(), 222);
        assert!(result.is_some());
        let found: Value =
            serde_json::from_str(&fs::read_to_string(result.unwrap()).unwrap()).unwrap();
        assert_eq!(found["session_id"], "bbb");
    }

    #[test]
    fn preserves_all_existing_fields() {
        let (_tmp, cstatus_path) = make_projects_with_cstatus(12345);

        let contents = fs::read_to_string(&cstatus_path).unwrap();
        let mut data: Value = serde_json::from_str(&contents).unwrap();
        data["session_name"] = Value::String("Test".to_string());
        let mut serialized = serde_json::to_string(&data).unwrap();
        serialized.push('\n');
        write_atomic(&cstatus_path, serialized.as_bytes()).unwrap();

        let result: Value =
            serde_json::from_str(&fs::read_to_string(&cstatus_path).unwrap()).unwrap();
        assert_eq!(result["session_id"], "abc123");
        assert_eq!(result["pid"], 12345);
        assert_eq!(result["ppid"], 1);
        assert_eq!(result["state"], "active");
        assert_eq!(result["activity"], "thinking");
        assert_eq!(result["timestamp"], "2026-03-13T12:00:00Z");
        assert_eq!(result["cwd"], "/tmp/project");
        assert_eq!(result["event"], "UserPromptSubmit");
        assert_eq!(result["session_name"], "Test");
    }

    #[test]
    fn find_cstatus_ignores_non_cstatus_files() {
        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path().join("-test-project");
        fs::create_dir_all(&project_dir).unwrap();

        // Write a .jsonl file with a pid field — should be ignored
        let data = serde_json::json!({"pid": 12345});
        fs::write(
            project_dir.join("session.jsonl"),
            serde_json::to_string(&data).unwrap(),
        )
        .unwrap();

        let result = find_cstatus_for_pid(tmp.path(), 12345);
        assert!(result.is_none());
    }

    #[test]
    fn find_cstatus_ignores_files_in_root() {
        let tmp = TempDir::new().unwrap();

        // Put a .cstatus file directly in the root (not in a subdirectory)
        let data = serde_json::json!({"pid": 12345, "session_id": "root"});
        fs::write(
            tmp.path().join("root.cstatus"),
            serde_json::to_string(&data).unwrap(),
        )
        .unwrap();

        let result = find_cstatus_for_pid(tmp.path(), 12345);
        assert!(result.is_none());
    }
}
