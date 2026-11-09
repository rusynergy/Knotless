use std::{fs, io, path::{Path, PathBuf}, process::{Child, Command, ExitStatus, Stdio}, time::{Duration, Instant}};

pub enum Process {
    Plain(Child),
    Tun { helper: Child, marker: PathBuf },
}

impl Process {
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match self { Self::Plain(p) => p.try_wait(), Self::Tun {helper,..} => helper.try_wait() }
    }
    pub fn graceful_stop(&mut self) {
        match self {
            Self::Plain(child) => { let _ = child.kill(); let _ = child.wait(); }
            Self::Tun { helper, marker } => {
                let _ = fs::remove_file(&*marker);
                let deadline = Instant::now() + Duration::from_secs(12);
                while Instant::now() < deadline {
                    if helper.try_wait().ok().flatten().is_some() { return; }
                    std::thread::sleep(Duration::from_millis(100));
                }
                // The supervisor independently checks the marker, including after a late authorization.
                let _ = helper.kill(); let _ = helper.wait();
            }
        }
    }
}
pub fn is_elevated() -> bool { unsafe { libc::geteuid() == 0 } }
pub fn relaunch_elevated() -> io::Result<()> {
    Err(io::Error::other("macOS requests authorization when connecting TUN"))
}
pub fn dns_ready() -> bool {
    Command::new("/usr/sbin/scutil").arg("--dns").output().is_ok_and(|o|
        String::from_utf8_lossy(&o.stdout).lines().any(|line|line.split_whitespace().last()==Some("172.31.255.1")))
}
pub fn spawn_console(exe: &Path, config: &Path, log: &fs::File) -> io::Result<Process> {
    let expected = option_env!("KNOTLESS_MAC_XRAY_HASH").ok_or_else(||io::Error::other("Build with scripts/build-macos.sh for TUN"))?;
    let marker = config.with_extension("active");
    let data: serde_json::Value = serde_json::from_slice(&fs::read(config)?)?;
    let name = data["inbounds"].as_array().and_then(|a|a.iter().find(|v|v["protocol"]=="tun"))
        .and_then(|v|v["settings"]["name"].as_str()).ok_or_else(||io::Error::other("Missing utun"))?;
    fs::File::create_new(&marker)?;
    // Arguments are quoted by AppleScript, never interpolated into shell source.
    let script = r#"on run argv
set cmd to "/bin/sh -c " & quoted form of (item 1 of argv) & " knotless"
repeat with n from 2 to count of argv
set cmd to cmd & " " & quoted form of (item n of argv)
end repeat
do shell script cmd with administrator privileges
end run"#;
    let result = Command::new("/usr/bin/osascript").arg("-e").arg(script)
        .arg(include_str!("mac-tun.sh")).arg(exe).arg(config).arg(&marker)
        .arg(std::process::id().to_string()).arg(expected).arg(name)
        .stdin(Stdio::null()).stdout(Stdio::from(log.try_clone()?)).stderr(Stdio::from(log.try_clone()?)).spawn();
    match result {
        Ok(helper) => Ok(Process::Tun { helper,marker }),
        Err(e) => { let _ = fs::remove_file(marker); Err(e) }
    }
}
