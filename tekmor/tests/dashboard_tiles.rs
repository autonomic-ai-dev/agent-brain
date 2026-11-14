use std::process::Command;

#[test]
fn every_dashboard_tile_opens_to_a_detail_panel() {
    let home = tempfile::tempdir().expect("home");
    let output = Command::new(env!("CARGO_BIN_EXE_tekmor"))
        .arg("dashboard")
        .env("TEKMOR_HOME", home.path())
        .output()
        .expect("run tekmor dashboard");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let html = std::fs::read_to_string(home.path().join("metrics/dashboard.html"))
        .expect("dashboard html");
    let tiles = html.matches("<details class=\"card tile\">").count();
    let details = html.matches("<p class=\"detail\">").count();
    assert_eq!(tiles, 12, "one details tile per card");
    assert_eq!(details, tiles, "every tile has a detail panel");
    assert!(!html.contains("<script"), "dashboard must not run scripts");
    assert!(html.contains("Click any tile"));
}
