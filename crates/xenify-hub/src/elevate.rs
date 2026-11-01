//! Running a program as the computer's administrator (asks for the password
//! once): pkexec on Linux, the administrator prompt on a Mac, UAC on Windows.

use std::path::Path;

#[cfg(target_os = "linux")]
pub fn run(program: &Path, args: &[String]) -> Result<(), String> {
    let out = match xenify_hub_core::command("pkexec").arg(program).args(args).output() {
        Ok(out) => out,
        // No password prompt for programs here (a desktop without polkit).
        Err(_) => {
            let quote = |s: &str| {
                if s.contains([' ', '\'', '"']) { format!("'{}'", s.replace('\'', r"'\''")) } else { s.to_string() }
            };
            let line = std::iter::once(program.display().to_string())
                .chain(args.iter().cloned())
                .map(|a| quote(&a))
                .collect::<Vec<_>>()
                .join(" ");
            return Err(format!("this computer can't ask for the password here; in a terminal, run: sudo {line}"));
        }
    };
    match out.status.code() {
        Some(0) => Ok(()),
        // Cancelled, or the password was wrong.
        Some(126 | 127) => Err("it needs your computer's password".into()),
        _ => Err(String::from_utf8_lossy(&out.stderr).trim().lines().last().unwrap_or("it didn't work").to_string()),
    }
}

#[cfg(target_os = "macos")]
pub fn run(program: &Path, args: &[String]) -> Result<(), String> {
    let quote = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
    let line = std::iter::once(program.display().to_string())
        .chain(args.iter().cloned())
        .map(|a| quote(&a))
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        line.replace('\\', "\\\\").replace('"', "\\\"")
    );
    let out = xenify_hub_core::command("osascript").args(["-e", &script]).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let e = String::from_utf8_lossy(&out.stderr);
        Err(if e.contains("-128") { "it needs your computer's password".into() } else { e.trim().to_string() })
    }
}

/// Runs `program` as administrator (Windows asks first), hidden, and
/// waits for it. Asked of Windows directly: no PowerShell or command
/// prompt in between to show a window.
#[cfg(windows)]
pub fn run(program: &Path, args: &[String]) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
    use windows_sys::Win32::UI::Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;
    // Each argument as one: in quotes if it has a space (none of the ones
    // passed here holds a quote).
    if args.iter().any(|a| a.contains('"')) {
        return Err("it didn't work".into());
    }
    let params = args
        .iter()
        .map(|a| if a.is_empty() || a.contains(' ') { format!("\"{a}\"") } else { a.clone() })
        .collect::<Vec<_>>()
        .join(" ");
    let wide = |t: &str| t.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (verb, file, params) = (wide("runas"), wide(&program.display().to_string()), wide(&params));
    // SAFETY: the strings outlive the call; the process handle is closed here.
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = SW_HIDE;
        if ShellExecuteExW(&mut info) == 0 {
            // ERROR_CANCELLED: "No" when Windows asked.
            return Err(if GetLastError() == 1223 {
                "it needs administrator permission".into()
            } else {
                "it didn't work".into()
            });
        }
        if info.hProcess.is_null() {
            return Err("it didn't work".into());
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        if code == 0 { Ok(()) } else { Err("it didn't work".into()) }
    }
}

/// Whether a value from the release list is safe to hand to the
/// administrator's step: a sha256 in hex, and a program inside the
/// download (no absolute path, no way out of its folder).
fn plain_values(sha256: &str, run: &str) -> Result<(), String> {
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("the release list's checksum isn't one".into());
    }
    let bad = run.is_empty()
        || run.starts_with(['/', '\\'])
        || run.contains(':')
        || run.split(['/', '\\']).any(|p| p == "..")
        || run.contains(['\'', '"', '\n', '\r']);
    if bad {
        return Err("the release list names an odd program".into());
    }
    Ok(())
}

/// What runs as root (Linux and Mac): the download is copied into a folder
/// only root can change, checked there against the signed release list's
/// checksum, unpacked there, and only that copy is run. The person's own
/// copy is never run as root (whatever they run could have changed it).
#[cfg(unix)]
const VERIFIED_SH: &str = r#"set -eu
archive=$1 sha=$2 run=$3
shift 3
if [ "$(uname -s)" = Darwin ]; then base="/Library/Application Support/Xenify/Installer"; else base=/usr/local/lib/xenify/installer; fi
umask 022
mkdir -p "$base"
if [ -n "$(find "$base" "$(dirname "$base")" -maxdepth 0 \( ! -user 0 -o -perm -0020 -o -perm -0002 \) -print)" ]; then
    echo "$base can be changed by others than root" >&2
    exit 1
fi
work=$(mktemp -d "$base/run.XXXXXX")
trap 'rm -rf "$work"' EXIT
cp "$archive" "$work/download"
if command -v sha256sum >/dev/null 2>&1; then got=$(sha256sum "$work/download"); else got=$(shasum -a 256 "$work/download"); fi
if [ "${got%% *}" != "$sha" ]; then
    echo "the download doesn't match the release list" >&2
    exit 1
fi
mkdir "$work/app"
tar -xzf "$work/download" -C "$work/app" --no-same-owner
chmod 755 "$work"
"$work/app/$run" "$@"
"#;

/// Runs `run` from the downloaded `archive` as the administrator, after
/// checking (as the administrator, on its own copy) that it's the file
/// the signed release list names by `sha256`.
#[cfg(unix)]
pub fn run_verified(archive: &Path, sha256: &str, run: &str, args: &[String]) -> Result<(), String> {
    plain_values(sha256, run)?;
    let mut all = vec![
        "-c".to_string(),
        VERIFIED_SH.to_string(),
        "xenify-install".into(),
        archive.display().to_string(),
        sha256.to_ascii_lowercase(),
        run.to_string(),
    ];
    all.extend(args.iter().cloned());
    if cfg!(target_os = "linux") && xenify_hub_core::command("pkexec").arg("--version").output().is_err() {
        return Err("this computer can't ask for the password here (it has no polkit)".into());
    }
    self::run(Path::new("/bin/sh"), &all)
}

/// Stops starting the server with the computer: only the system's own
/// tools, run as the administrator.
#[cfg(target_os = "linux")]
pub fn remove_service() -> Result<(), String> {
    let script = "systemctl disable --now xenify-server 2>/dev/null || true; rm -f /etc/systemd/system/xenify-server.service; systemctl daemon-reload";
    run(Path::new("/bin/sh"), &["-c".into(), script.into()])
}

#[cfg(target_os = "macos")]
pub fn remove_service() -> Result<(), String> {
    let script = "launchctl bootout system/net.xenify.server 2>/dev/null || true; rm -f /Library/LaunchDaemons/net.xenify.server.plist";
    run(Path::new("/bin/sh"), &["-c".into(), script.into()])
}

/// PowerShell's single-quoted text.
#[cfg(windows)]
fn ps(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A PowerShell script as -EncodedCommand takes it (UTF-16LE, base64).
#[cfg(windows)]
fn encoded(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    const B: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for k in 0..4 {
            out.push(if k <= c.len() { B[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// Windows: the same as on Linux and Mac, in a folder under Program Files
/// (only administrators can change it).
#[cfg(windows)]
pub fn run_verified(archive: &Path, sha256: &str, run: &str, args: &[String]) -> Result<(), String> {
    plain_values(sha256, run)?;
    let list = args.iter().map(|a| ps(&format!("\"{a}\""))).collect::<Vec<_>>().join(", ");
    let script = format!(
        "$ErrorActionPreference = 'Stop'\n\
         $base = Join-Path $env:ProgramFiles 'Xenify\\Installer'\n\
         New-Item -ItemType Directory -Force -Path $base | Out-Null\n\
         $work = Join-Path $base ([guid]::NewGuid().ToString('N'))\n\
         New-Item -ItemType Directory -Path $work | Out-Null\n\
         try {{\n\
           $zip = Join-Path $work 'download.zip'\n\
           Copy-Item -LiteralPath {archive} -Destination $zip\n\
           if ((Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLower() -ne {sha}) {{ exit 3 }}\n\
           Expand-Archive -LiteralPath $zip -DestinationPath (Join-Path $work 'app')\n\
           $p = Start-Process -FilePath (Join-Path (Join-Path $work 'app') {run}) -ArgumentList @({list}) -Wait -PassThru -WindowStyle Hidden\n\
           exit $p.ExitCode\n\
         }} finally {{ Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue }}\n",
        archive = ps(&archive.display().to_string()),
        sha = ps(&sha256.to_ascii_lowercase()),
        run = ps(run),
    );
    self::run(
        Path::new("powershell"),
        &["-NoProfile".into(), "-NonInteractive".into(), "-WindowStyle".into(), "Hidden".into(), "-ExecutionPolicy".into(), "Bypass".into(), "-EncodedCommand".into(), encoded(&script)],
    )
}

#[cfg(windows)]
pub fn remove_service() -> Result<(), String> {
    let script = "sc.exe stop XenifyServer | Out-Null; sc.exe delete XenifyServer | Out-Null; exit 0";
    run(
        Path::new("powershell"),
        &["-NoProfile".into(), "-NonInteractive".into(), "-WindowStyle".into(), "Hidden".into(), "-EncodedCommand".into(), encoded(script)],
    )
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn only_plain_values_reach_the_administrator() {
        let sha = "9363804f33987941e1b3dcc1a493da04ea9da06dc21c26087c7c3b55b05640b3";
        assert!(super::plain_values(sha, "xenify-server/xenify-server").is_ok());
        assert!(super::plain_values("abc", "xenify-server").is_err());
        assert!(super::plain_values(sha, "/bin/sh").is_err());
        assert!(super::plain_values(sha, "../../bin/sh").is_err());
        assert!(super::plain_values(sha, "a/../../x").is_err());
    }

    /// The root step's checks, run as this user in a folder of its own.
    #[test]
    fn the_copy_is_checked_before_it_runs() {
        let base = std::env::temp_dir().join(format!("xenify-elevate-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("pkg/app")).unwrap();
        std::fs::write(base.join("pkg/app/run.sh"), "#!/bin/sh\necho \"ran $*\" > \"$OUT\"\n").unwrap();
        xenify_hub_core::command("chmod").arg("755").arg(base.join("pkg/app/run.sh")).status().unwrap();
        let archive = base.join("app.tar.gz");
        assert!(
            xenify_hub_core::command("tar").arg("-czf").arg(&archive).arg("-C").arg(base.join("pkg")).arg("app").status().unwrap().success()
        );
        let sha = xenify_hub_core::manifest::sha256_hex(&std::fs::read(&archive).unwrap());
        // The script with its folder moved somewhere this test may write.
        let script = super::VERIFIED_SH
            .replace("base=/usr/local/lib/xenify/installer", &format!("base={}/installer", base.display()))
            .replace("! -user 0 -o ", "");
        let go = |sha: &str| {
            xenify_hub_core::command("/bin/sh")
                .args(["-c", &script, "xenify-install"])
                .arg(&archive)
                .args([sha, "app/run.sh", "service", "install"])
                .env("OUT", base.join("out"))
                .output()
                .unwrap()
        };
        let bad = go(&"0".repeat(64));
        assert!(!bad.status.success());
        assert!(String::from_utf8_lossy(&bad.stderr).contains("doesn't match the release list"));
        assert!(!base.join("out").exists(), "nothing ran");
        let good = go(&sha);
        assert!(good.status.success(), "{}", String::from_utf8_lossy(&good.stderr));
        assert_eq!(std::fs::read_to_string(base.join("out")).unwrap(), "ran service install\n");
        // Its working copy is gone afterwards.
        assert_eq!(std::fs::read_dir(base.join("installer")).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&base);
    }
}
