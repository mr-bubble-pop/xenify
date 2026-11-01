//! Apps start the checker program themselves and it exits when they're gone.

#[cfg(unix)]
#[test]
fn apps_start_one_checker_that_exits_after_them() {
    use std::os::unix::fs::DirBuilderExt;
    use std::time::{Duration, Instant};
    let run = std::env::temp_dir().join(format!("xenify-checker-start-{}", std::process::id()));
    std::fs::DirBuilder::new().mode(0o700).create(&run).unwrap();
    // Safety: this test is alone in its process (an integration test file).
    unsafe {
        std::env::set_var("XDG_RUNTIME_DIR", &run);
        std::env::set_var("XENIFY_CHECKER", env!("CARGO_BIN_EXE_xenify-checker"));
        std::env::set_var("XENIFY_CHECKER_GRACE", "1");
    }
    let socket = xenify_checker::socket_path().unwrap();
    let a = xenify_checker::Client::connect().unwrap();
    let b = xenify_checker::Client::connect().unwrap();
    assert!(socket.exists());
    if let Some(dir) = std::env::var_os("XENIFY_SPOTTER").map(std::path::PathBuf::from) {
        assert!(a.load(&dir).unwrap() > 0);
        let text = "She go to school every days.";
        assert_eq!(a.spot(&dir, text, 0.5, 3, &[]).unwrap(), b.spot(&dir, text, 0.5, 3, &[]).unwrap());
    }
    drop(a);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(socket.exists(), "one app still uses it");
    drop(b);
    let until = Instant::now() + Duration::from_secs(5);
    while socket.exists() {
        assert!(Instant::now() < until, "the checker didn't exit");
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = std::fs::remove_dir_all(&run);
}
