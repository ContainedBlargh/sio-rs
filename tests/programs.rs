use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn resources_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/resources")
}

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_sio-rs")
}

/// Run the binary with the given .sio files and stdin input, killing it after `timeout`.
/// Returns captured stdout as a String.
fn run_with(files: &[&str], stdin_input: &str, timeout: Duration) -> String {
    let mut cmd = Command::new(bin());
    for f in files {
        cmd.arg(resources_dir().join(f));
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");

    if !stdin_input.is_empty() {
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(stdin_input.as_bytes())
            .unwrap();
    }
    drop(child.stdin.take());

    let start = Instant::now();
    loop {
        match child.try_wait().unwrap() {
            Some(_) => break,
            None => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    let output = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn test_no_comments_counts_down() {
    let out = run_with(
        &["test_no_comments.sio"],
        "",
        Duration::from_secs(10),
    );
    let lines: Vec<&str> = out.lines().collect();
    // 25 down to 1, then "done"
    assert_eq!(lines.first().copied(), Some("25"));
    assert!(lines.contains(&"1"));
    assert!(lines.iter().any(|l| l.trim() == "done"));
    // first 25 lines are decreasing integers from 25 down to 1
    for (i, n) in (0..25).zip((1..=25).rev()) {
        assert_eq!(lines[i], n.to_string(), "line {}: got {:?}", i, lines[i]);
    }
}

#[test]
fn test_radix() {
    let out = run_with(&["radix-test.sio"], "", Duration::from_secs(5));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["38", "42"]);
}

#[test]
fn test_array() {
    let out = run_with(&["array-test.sio"], "", Duration::from_secs(5));
    // The program writes arr[i]=i for i=0..9, then also arr[10%10=0]=10 on the
    // final iteration when acc reaches 10 but the test passes. So arr[0]=10 and
    // arr[1..=9]=1..=9. It then prints the walk downward from arr[10]=arr[0]=10
    // through arr[9..=1], followed by arr[0]=10 as the "last one": arr[-1]=arr[9]=9.
    assert!(out.contains("[10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 10]"));
    assert!(out.contains("and the last one:"));
    assert!(out.trim_end().ends_with("9"));
}

#[test]
fn test_text_spongebob() {
    let out = run_with(&["text.sio"], "", Duration::from_secs(5));
    assert!(out.trim_end().ends_with("spongebob squarepants"));
    assert!(out.contains("spongebobspongebob"));
}

#[test]
fn test_more_text_cross_product() {
    let out = run_with(&["more-text.sio"], "", Duration::from_secs(5));
    assert_eq!(out.trim_end(), "abababab");
}

#[test]
fn test_jump_countdown() {
    let out = run_with(&["jump.sio"], "", Duration::from_secs(30));
    assert!(out.contains("Counting down from 100"));
    assert!(out.contains("I'm done counting :)"));
    for n in 1..=99 {
        assert!(
            out.contains(&format!("\n{}\n", n)),
            "missing number {}",
            n
        );
    }
}

#[test]
fn test_fib_5() {
    let out = run_with(&["fib.sio"], "5\n", Duration::from_secs(10));
    // fib(5) = 5. The printed result follows the "> " prompt (stdin is piped,
    // so the user's "5\n" is not echoed). The first numeric token printed by
    // the program should be the answer.
    let first_num = out
        .lines()
        .filter_map(|l| l.trim_start_matches("> ").trim().parse::<i64>().ok())
        .next()
        .unwrap_or_else(|| panic!("expected a numeric line, got: {:?}", out));
    assert_eq!(first_num, 5);
}

#[test]
fn test_fact_5() {
    // fact.sio never terminates once stdin is exhausted — it loops printing
    // `a` (reset to 1 each main iteration). So we just check that the first
    // printed number is 120, the result of fact(5).
    let out = run_with(&["fact.sio"], "5\n", Duration::from_secs(3));
    let first_num = out
        .lines()
        .find_map(|l| l.trim_start_matches("> ").trim().parse::<i64>().ok())
        .expect("expected at least one numeric line");
    assert_eq!(first_num, 120);
}

#[test]
fn test_xbus_sender_receiver() {
    let out = run_with(
        &["xbus-sender.sio", "xbus-receiver.sio"],
        "",
        Duration::from_secs(20),
    );
    // Sender sends 10,9,...,0,-1; receiver counts non-negative packets.
    assert!(
        out.contains("received 11 packages, package sum: 55"),
        "unexpected output: {:?}",
        out
    );
}

#[test]
fn test_aoc1_day1_part1() {
    let input = std::fs::read_to_string(resources_dir().join("aoc1.test")).unwrap();
    let out = run_with(&["aoc1.sio"], &input, Duration::from_secs(30));
    assert!(
        out.contains("total: 7"),
        "expected 'total: 7' in output, got: {:?}",
        out
    );
}

#[test]
fn test_fio_binary_roundtrip() {
    let out = run_with(&["fio_roundtrip.sio"], "", Duration::from_secs(5));
    assert_eq!(out.trim_end(), "10\n20\n30");
}

#[test]
fn test_stacked_tests_or_semantics() {
    let out = run_with(&["stacked-tests.sio"], "", Duration::from_secs(5));
    // "a" and "A" each match the stacked teq; "b" does not.
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["match", "match", "done"]);
}

#[test]
fn test_fio_text_roundtrip() {
    /* 
        TODO: Fix this test, it's incorrect. 
        Claude corrected the source code for the registers, rather than his program text.
        Silly Claude.
    */
    let out = run_with(&["fio_text_roundtrip.sio"], "", Duration::from_secs(5));
    assert_eq!(out.trim_end(), "hello\nworld");
}
