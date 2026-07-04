use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

fn example_script(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples").join(name)
}

fn fixture_script(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
}

fn run_trapdoor(args: &[&str], script: &Path, commands: &[&str]) -> (String, ExitStatus) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_trapdoor"))
        .arg("--no-color")
        .args(args)
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn trapdoor test binary");

    {
        let stdin = child.stdin.as_mut().expect("trapdoor stdin should be piped");
        for command in commands {
            writeln!(stdin, "{command}").expect("failed to write REPL command");
        }
    }

    let output = child.wait_with_output().expect("failed waiting for trapdoor");
    let mut captured = String::from_utf8_lossy(&output.stdout).into_owned();
    captured.push_str(&String::from_utf8_lossy(&output.stderr));

    if captured.contains("cannot launch bash") {
        panic!("bash not found: trapdoor could not launch bash from PATH\n{captured}");
    }

    (captured, output.status)
}

#[test]
fn step_and_print() {
    let (output, status) = run_trapdoor(
        &[],
        &example_script("demo.sh"),
        &["s", "s", "s", "p count", "c"],
    );

    assert!(status.success(), "{output}");
    assert!(output.contains("declare -- count=\"0\""), "{output}");
}

#[test]
fn conditional_breakpoint() {
    let (output, status) = run_trapdoor(
        &["-r", "-b", "demo.sh:13 if (( count == 1 ))"],
        &example_script("demo.sh"),
        &["p count", "c"],
    );

    assert!(status.success(), "{output}");
    assert!(output.contains("declare -- count=\"1\""), "{output}");
    // The `●` announce tag only appears when the breakpoint actually fires;
    // the bare "breakpoint #1" string is already printed at registration.
    assert!(output.contains("● breakpoint #1"), "{output}");
}

#[test]
fn watch_and_until() {
    let (output, status) = run_trapdoor(
        &[],
        &example_script("demo.sh"),
        &["w $count", "u 17", "c"],
    );

    assert!(status.success(), "{output}");
    assert!(output.contains("watch #1: $count = 3"), "{output}");
}

#[test]
fn live_mutation() {
    let (output, status) = run_trapdoor(
        &["-r", "-b", "36"],
        &example_script("pipeline.sh"),
        &["!requests=11", "c"],
    );

    assert!(status.success(), "{output}");
    assert!(
        output.lines().any(|line| line.contains("requests=11")),
        "{output}"
    );
}

#[test]
fn exit_status() {
    let (output, status) = run_trapdoor(&["-r"], &fixture_script("exit3.sh"), &[]);

    assert_eq!(status.code(), Some(3), "{output}");
}
