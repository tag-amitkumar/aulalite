use std::process::Command;

#[test]
fn help_lists_admin_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_aulalite-admin"))
        .arg("--help")
        .output()
        .expect("run aulalite-admin --help");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("help output is utf-8");
    assert!(stdout.contains("create-tenant"));
    assert!(stdout.contains("promote-platform-admin"));
}
