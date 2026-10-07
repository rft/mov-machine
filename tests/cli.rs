use std::{
    fs,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Workspace(std::path::PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mov-machine-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mov-machine"))
}
#[test]
fn assemble_run_exec_and_trace() {
    let w = Workspace::new();
    let source = w.file("program.s");
    let image = w.file("program.mov");
    fs::write(&source, "mov r0, -8\nmov r1, 42\nmov [r0], r1").unwrap();
    assert!(cli()
        .arg("assemble")
        .arg(&source)
        .arg(&image)
        .output()
        .unwrap()
        .status
        .success());
    let run = cli()
        .arg("run")
        .arg(&image)
        .args(["--max-steps", "3", "--trace", "--ram-words", "0"])
        .output()
        .unwrap();
    assert!(run.status.success());
    let stdout = String::from_utf8(run.stdout).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 4);
    assert!(lines[2].contains("\"memory_write\":{\"address\":4294967288,\"value\":42}"));
    let exec = cli()
        .arg("exec")
        .arg(&source)
        .args(["--max-steps", "3"])
        .output()
        .unwrap();
    assert!(exec.status.success());
}
#[test]
fn actual_cli_errors_fault_and_budget_status() {
    let w = Workspace::new();
    let source = w.file("program.s");
    fs::write(&source, "mov r0, 1").unwrap();
    let budget = cli()
        .arg("exec")
        .arg(&source)
        .args(["--max-steps", "1"])
        .output()
        .unwrap();
    assert_eq!(budget.status.code(), Some(1));
    let fault = cli()
        .arg("exec")
        .arg(&source)
        .args(["--max-steps", "2"])
        .output()
        .unwrap();
    assert_eq!(fault.status.code(), Some(1));
    assert!(String::from_utf8(fault.stdout)
        .unwrap()
        .contains("\"fault\":true"));
    for options in [
        vec![],
        vec!["--max-steps", "-1"],
        vec!["--max-steps", "1", "--unknown"],
        vec!["--max-steps", "1", "--trace", "--trace"],
    ] {
        let error = cli()
            .arg("exec")
            .arg(&source)
            .args(options)
            .output()
            .unwrap();
        assert_eq!(error.status.code(), Some(2));
    }
    fs::write(&source, "mov_add r0, r1").unwrap();
    assert_eq!(
        cli()
            .arg("exec")
            .arg(&source)
            .args(["--max-steps", "1"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    let image = w.file("bad.mov");
    fs::write(&image, [0; 3]).unwrap();
    assert_eq!(
        cli()
            .arg("run")
            .arg(&image)
            .args(["--max-steps", "1"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
}
