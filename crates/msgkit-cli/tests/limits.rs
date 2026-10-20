//! The command line's limit flags: the defaults apply, a flag changes one limit, `--no-limits` is the only way to
//! switch them off, and 0 is refused.

use std::path::PathBuf;
use std::process::{Command, Output};

const CHAT: &str = "01/02/2024, 10:00 - Alice: PRIVATE-VALUE-MUST-NOT-APPEAR\n01/02/2024, 10:01 - Bob: hi\n";

fn chat_file(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("msgkit-cli-limits-{}-{name}.txt", std::process::id()));
    std::fs::write(&path, CHAT).unwrap();
    path
}

fn run(args: &[&str], file: &PathBuf) -> Output {
    Command::new(env!("CARGO_BIN_EXE_msgkit")).arg("whatsapp-export").arg(file).args(args).output().unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn the_defaults_accept_a_normal_export() {
    let file = chat_file("default");
    let out = run(&[], &file);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout).lines().count(), 2);
}

#[test]
fn a_flag_lowers_one_limit_and_the_error_never_repeats_the_input() {
    let file = chat_file("lower");
    for args in [["--max-messages", "1"], ["--max-line-bytes", "10"], ["--max-input-bytes", "20"]] {
        let out = run(&args, &file);
        assert!(!out.status.success(), "{args:?}");
        let err = stderr(&out);
        assert!(
            err.contains("limit exceeded")
                && err.contains(args[0].trim_start_matches("--").replace('-', "_").as_str()),
            "{err}"
        );
        assert!(!err.contains("PRIVATE-VALUE") && !err.contains("Alice"), "{err}");
        assert_eq!(out.stdout, b"", "nothing is imported");
    }
}

#[test]
fn zero_is_refused_by_the_parser_and_no_limits_is_the_explicit_opt_out() {
    let file = chat_file("zero");
    for flag in ["--max-messages", "--max-input-bytes", "--max-line-bytes", "--max-json-depth"] {
        let out = run(&[flag, "0"], &file);
        assert!(!out.status.success(), "{flag} 0 must be refused");
        assert_eq!(out.stdout, b"");
    }
    let out = run(&["--max-messages", "1", "--no-limits"], &file);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout).lines().count(), 2);
}

#[test]
fn the_device_command_reports_a_limit_as_an_error_not_a_warning() {
    let path = std::env::temp_dir().join(format!("msgkit-cli-limits-{}-device.json", std::process::id()));
    std::fs::write(
        &path,
        r#"{"sms":[{"address":"+94700000000","body":"a","date_ms":1,"type":1},{"address":"+94700000000","body":"b","date_ms":2,"type":1}]}"#,
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_msgkit"))
        .arg("device")
        .arg(&path)
        .args(["--max-messages", "1"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("limit exceeded"), "{}", stderr(&out));
}
