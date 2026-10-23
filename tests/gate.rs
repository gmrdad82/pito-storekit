use std::path::PathBuf;
use std::process::{Command, Output};

fn gate(args: &[&str]) -> Output {
    Command::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bin/gate"))
        .args(args)
        .env_remove("NO_COLOR")
        .output()
        .unwrap()
}

fn refusal(message: &str) -> String {
    format!("error: {message}\n\nUsage: bin/gate [--fast]\n\nFor more information, try '--help'.\n")
}

#[test]
fn an_unknown_flag_is_refused_on_stderr_with_exit_2_before_any_step_runs() {
    let output = gate(&["--nope"]);

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        refusal("unexpected argument '--nope' found")
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn a_stray_word_is_refused() {
    let output = gate(&["extra"]);

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        refusal("unexpected argument 'extra' found")
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn fast_given_twice_is_refused() {
    let output = gate(&["--fast", "--fast"]);

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        refusal("the argument '--fast' cannot be used multiple times")
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn help_prints_usage_on_stdout_and_runs_nothing() {
    for flag in ["--help", "-h"] {
        let output = gate(&[flag]);

        assert_eq!(output.status.code(), Some(0), "{flag}");
        assert!(output.stderr.is_empty(), "{flag}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            stdout.contains("Usage: bin/gate [--fast]"),
            "{flag}: {stdout}"
        );
        assert!(stdout.contains("--fast"), "{flag}: {stdout}");
        assert!(stdout.starts_with("Run the gate"), "{flag}: {stdout}");
        assert!(
            !stdout.trim_end().ends_with("fast gate: green"),
            "{flag}: {stdout}"
        );
    }
}

#[cfg(target_os = "linux")]
fn gate_in_a_terminal(no_color: Option<&str>) -> Option<(Option<i32>, String)> {
    if Command::new("script").arg("--version").output().is_err() {
        return None;
    }
    let mut command = Command::new("script");
    command
        .args(["-qec", "bin/gate --nope", "/dev/null"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env_remove("NO_COLOR");
    if let Some(value) = no_color {
        command.env("NO_COLOR", value);
    }
    let output = command.output().unwrap();
    Some((
        output.status.code(),
        String::from_utf8(output.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
    ))
}

#[cfg(target_os = "linux")]
#[test]
fn a_refusal_in_a_terminal_is_coloured_as_clap_colours_it() {
    let Some((code, shown)) = gate_in_a_terminal(None) else {
        return;
    };

    assert_eq!(code, Some(2));
    assert_eq!(
        shown,
        "\x1b[1;31merror:\x1b[0m unexpected argument '\x1b[33m--nope\x1b[0m' found\n\n\
         \x1b[1;4mUsage:\x1b[0m \x1b[1mbin/gate\x1b[0m [--fast]\n\n\
         For more information, try '\x1b[1m--help\x1b[0m'.\n"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_refusal_in_a_terminal_with_no_color_is_plain() {
    let Some((code, shown)) = gate_in_a_terminal(Some("1")) else {
        return;
    };

    assert_eq!(code, Some(2));
    assert_eq!(shown, refusal("unexpected argument '--nope' found"));
}
