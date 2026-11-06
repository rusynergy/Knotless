//! The installer runs a temporary copy of the NEW executable as a helper.
//! Only the helper elevates, so per-user install paths/registry stay unchanged.
use std::{io, path::{Path,PathBuf}, os::windows::{ffi::OsStrExt,process::CommandExt}, process::Command};
use windows_sys::Win32::{Foundation::*,System::{Threading::*,Diagnostics::ToolHelp::*,RemoteDesktop::ProcessIdToSessionId},UI::{Shell::*,WindowsAndMessaging::SW_HIDE}};

fn wide(s:impl AsRef<std::ffi::OsStr>)->Vec<u16>{s.as_ref().encode_wide().chain(Some(0)).collect()}
struct Handle(HANDLE);
impl Handle{fn wait(&self,ms:u32)->u32{unsafe{WaitForSingleObject(self.0,ms)}}}
impl Drop for Handle{fn drop(&mut self){unsafe{CloseHandle(self.0);}}}
// Handles are owned by a single worker after creation.
unsafe impl Send for Handle{}
fn event_name(pid:u32)->Vec<u16>{wide(format!("Local\\Knotless.UpdateShutdown.{pid}"))}
fn event()->io::Result<Handle>{unsafe{let h=CreateEventW(std::ptr::null(),1,0,event_name(std::process::id()).as_ptr());if h.is_null(){Err(io::Error::last_os_error())}else{Ok(Handle(h))}}}

pub fn start(app:tauri::AppHandle)->io::Result<()>{
    let signal=event()?;
    std::thread::spawn(move||loop{
        let result=signal.wait(1000);
        if result==WAIT_OBJECT_0{crate::tray::quit(&app);break;}
        use tauri::Manager;
        if result==WAIT_FAILED || app.state::<crate::AppState>().closing.load(std::sync::atomic::Ordering::SeqCst){break;}
    });Ok(())
}

fn image(h:HANDLE)->io::Result<PathBuf>{unsafe{
    let mut buf=vec![0u16;32768];let mut size=buf.len() as u32;
    if QueryFullProcessImageNameW(h,0,buf.as_mut_ptr(),&mut size)==0{return Err(io::Error::last_os_error());}
    Ok(PathBuf::from(String::from_utf16_lossy(&buf[..size as usize])))
}}
fn normalized(path:&Path)->io::Result<String>{Ok(path.canonicalize()?.to_string_lossy().to_lowercase())}
fn snapshot()->io::Result<Vec<(u32,u32,String)>>{unsafe{
    let raw=CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS,0);if raw==INVALID_HANDLE_VALUE{return Err(io::Error::last_os_error());}let h=Handle(raw);
    let mut entry:PROCESSENTRY32W=std::mem::zeroed();entry.dwSize=std::mem::size_of_val(&entry) as u32;
    let mut result=Vec::new();let mut ok=Process32FirstW(h.0,&mut entry);
    while ok!=0{let len=entry.szExeFile.iter().position(|c|*c==0).unwrap_or(entry.szExeFile.len());result.push((entry.th32ProcessID,entry.th32ParentProcessID,String::from_utf16_lossy(&entry.szExeFile[..len])));ok=Process32NextW(h.0,&mut entry);}
    Ok(result)
}}
fn open(pid:u32,access:u32)->io::Result<Handle>{unsafe{let h=OpenProcess(access,0,pid);if h.is_null(){Err(io::Error::last_os_error())}else{Ok(Handle(h))}}}
fn exited(h:HANDLE,ms:u32)->io::Result<bool>{match unsafe{WaitForSingleObject(h,ms)}{WAIT_OBJECT_0=>Ok(true),WAIT_TIMEOUT=>Ok(false),_=>Err(io::Error::last_os_error())}}
fn created(h:HANDLE)->io::Result<u64>{unsafe{let mut a=std::mem::zeroed();let mut b=std::mem::zeroed();let mut c=std::mem::zeroed();let mut d=std::mem::zeroed();if GetProcessTimes(h,&mut a,&mut b,&mut c,&mut d)==0{return Err(io::Error::last_os_error());}Ok((a.dwHighDateTime as u64)<<32|a.dwLowDateTime as u64)}}

fn stop(target:&Path)->io::Result<()>{
    if !target.exists(){return Ok(());}
    let expected=normalized(target)?;let filename=target.file_name().ok_or_else(||io::Error::from_raw_os_error(87))?.to_string_lossy();
    let mut session=0;unsafe{if ProcessIdToSessionId(std::process::id(),&mut session)==0{return Err(io::Error::last_os_error());}}
    for (pid,_,name) in snapshot()?{
        if pid==std::process::id() || !name.eq_ignore_ascii_case(&filename){continue;}
        let mut other_session=0;unsafe{if ProcessIdToSessionId(pid,&mut other_session)==0 || session!=other_session{continue;}}
        let query=match open(pid,PROCESS_QUERY_LIMITED_INFORMATION|PROCESS_SYNCHRONIZE){Ok(h)=>h,Err(e) if e.raw_os_error()==Some(ERROR_INVALID_PARAMETER as i32)=>continue,Err(e)=>return Err(e)};
        if exited(query.0,0)?{continue;}
        if normalized(&image(query.0)?)?!=expected{continue;}
        // Request terminate rights before touching Xray. Access denied elevates
        // the helper before any partial shutdown of an administrator client.
        let process=open(pid,PROCESS_TERMINATE|PROCESS_QUERY_LIMITED_INFORMATION|PROCESS_SYNCHRONIZE)?;
        if created(query.0)?!=created(process.0)?{return Err(io::Error::from_raw_os_error(1237));}
        let signal=unsafe{OpenEventW(EVENT_MODIFY_STATE,0,event_name(pid).as_ptr())};
        if !signal.is_null(){
            let signal=Handle(signal);if unsafe{SetEvent(signal.0)}==0{return Err(io::Error::last_os_error());}
            if !exited(process.0,30000)?{return Err(io::Error::from_raw_os_error(1460));}
            continue;
        }
        let error=io::Error::last_os_error();
        if error.raw_os_error()!=Some(ERROR_FILE_NOT_FOUND as i32){return Err(error);}
        // Compatibility with <=0.3.4: first let its console Xray children undo
        // TUN routes/DNS, then terminate only this exact application instance.
        for (child,parent,name) in snapshot()?{
            if parent!=pid || !name.eq_ignore_ascii_case("xray.exe"){continue;}
            let h=match open(child,PROCESS_TERMINATE|PROCESS_QUERY_LIMITED_INFORMATION|PROCESS_SYNCHRONIZE){Ok(h)=>h,Err(e) if e.raw_os_error()==Some(ERROR_INVALID_PARAMETER as i32)=>continue,Err(e)=>return Err(e)};
            if exited(h.0,0)? || created(h.0)?<created(process.0)?{continue;}
            let helper_exe=std::env::current_exe()?;
            #[cfg(test)] let helper_exe=std::env::var_os("KNOTLESS_HELPER_EXE").map(PathBuf::from).unwrap_or(helper_exe);
            let mut helper=Command::new(helper_exe).args(["--stop-xray",&child.to_string()]).creation_flags(CREATE_NO_WINDOW).spawn()?;
            let status=helper.wait()?;
            // Older local-proxy children have no console (and no TUN state).
            if status.code()==Some(ERROR_INVALID_HANDLE as i32) && !exited(h.0,0)? {
                if unsafe{TerminateProcess(h.0,0)}==0{return Err(io::Error::last_os_error());}
            }
            // No force-kill if cleanup fails: leave the current installation intact.
            if !exited(h.0,10000)?{return Err(io::Error::from_raw_os_error(1460));}
        }
        if !exited(process.0,0)? && unsafe{TerminateProcess(process.0,0)}==0{return Err(io::Error::last_os_error());}
        if !exited(process.0,10000)?{return Err(io::Error::from_raw_os_error(1460));}
    }Ok(())
}

fn elevate(target:&Path)->io::Result<i32>{unsafe{
    let exe=wide(std::env::current_exe()?);let verb=wide("runas");
    let args=wide(format!("--shutdown-for-update-elevated \"{}\"",target.display()));
    let mut info:SHELLEXECUTEINFOW=std::mem::zeroed();info.cbSize=std::mem::size_of_val(&info) as u32;
    info.fMask=SEE_MASK_NOCLOSEPROCESS|SEE_MASK_NOASYNC;info.lpVerb=verb.as_ptr();info.lpFile=exe.as_ptr();info.lpParameters=args.as_ptr();info.nShow=SW_HIDE;
    if ShellExecuteExW(&mut info)==0{return Err(io::Error::last_os_error());}
    if info.hProcess.is_null(){return Err(io::Error::from_raw_os_error(6));}let h=Handle(info.hProcess);
    if !exited(h.0,120000)?{return Err(io::Error::from_raw_os_error(1460));}
    let mut code=0;if GetExitCodeProcess(h.0,&mut code)==0{return Err(io::Error::last_os_error());}Ok(code as i32)
}}
pub fn handle_args(){
    let args:Vec<_>=std::env::args_os().collect();let flag=args.get(1).and_then(|s|s.to_str());
    if !matches!(flag,Some("--shutdown-for-update"|"--shutdown-for-update-elevated")){return;}
    let result=match args.get(2){Some(path)=>{
        let path=Path::new(path);match stop(path){Ok(())=>Ok(0),Err(e) if e.raw_os_error()==Some(ERROR_ACCESS_DENIED as i32)&&flag==Some("--shutdown-for-update")&&!crate::windows_process::is_elevated()=>elevate(path),Err(e)=>Err(e)}
    },None=>Err(io::Error::from_raw_os_error(87))};
    std::process::exit(result.unwrap_or_else(|e|e.raw_os_error().unwrap_or(1)));
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]#[ignore="Child process fixture for shutdown tests"]fn fixture(){
        if std::env::var_os("KNOTLESS_UPDATE_FIXTURE").is_none(){return;}
        let _core=std::env::var_os("KNOTLESS_UPDATE_XRAY").map(|exe|{
            let root=std::env::current_exe().unwrap().parent().unwrap().to_owned();let config=root.join("core.json");
            std::fs::write(&config,r#"{"inbounds":[],"outbounds":[{"protocol":"freedom"}]}"#).unwrap();let log=std::fs::File::create(root.join("core.log")).unwrap();
            let process=crate::windows_process::spawn_console(Path::new(&exe),&config,&log).unwrap();
            std::fs::write(root.join("core.pid"),unsafe{GetProcessId(process.raw_handle())}.to_string()).unwrap();process
        });
        if std::env::var_os("KNOTLESS_UPDATE_EVENT").is_some(){let h=event().unwrap();println!("READY");use std::io::Write;std::io::stdout().flush().unwrap();assert!(exited(h.0,60000).unwrap());}
        else{println!("READY");use std::io::Write;std::io::stdout().flush().unwrap();std::thread::sleep(std::time::Duration::from_secs(60));}
    }
    #[test]fn stops_exact_path_and_supports_graceful_and_legacy_clients(){
        use std::io::{BufRead,BufReader};
        let root=std::env::temp_dir().join(format!("knotless-update-test-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("other")).unwrap();let exe=root.join("knotless.exe");let other=root.join("other/knotless.exe");
        std::fs::copy(std::env::current_exe().unwrap(),&exe).unwrap();std::fs::copy(&exe,&other).unwrap();
        let launch=|path:&Path,graceful:bool|{let mut cmd=Command::new(path);cmd.args(["--exact","update_shutdown::tests::fixture","--ignored","--nocapture"]).env("KNOTLESS_UPDATE_FIXTURE","1").stdout(std::process::Stdio::piped()).creation_flags(CREATE_NO_WINDOW);if graceful{cmd.env("KNOTLESS_UPDATE_EVENT","1");}let mut child=cmd.spawn().unwrap();let mut reader=BufReader::new(child.stdout.take().unwrap());let mut line=String::new();loop{line.clear();assert!(reader.read_line(&mut line).unwrap()>0);if line.contains("READY"){break;}}child.stdout=Some(reader.into_inner());child};
        let mut unrelated=launch(&other,true);
        for graceful in [true,false]{let mut child=launch(&exe,graceful);stop(&exe).unwrap();assert!(child.wait().unwrap().success());assert!(unrelated.try_wait().unwrap().is_none());}
        stop(&other).unwrap();unrelated.wait().unwrap();
        std::fs::remove_file(exe).unwrap();std::fs::remove_file(other).unwrap();std::fs::remove_dir(root.join("other")).unwrap();std::fs::remove_dir(root).unwrap();
    }
    #[test]#[ignore="Needs built helper and real Xray; no TUN/network changes"]fn legacy_console_core_exits_cleanly(){
        use std::io::{BufRead,BufReader};
        let root=std::env::temp_dir().join(format!("knotless-update-core-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();let exe=root.join("knotless.exe");std::fs::copy(std::env::current_exe().unwrap(),&exe).unwrap();
        let xray=std::env::var_os("KNOTLESS_UPDATE_XRAY").expect("Xray fixture required");
        let mut child=Command::new(&exe).args(["--exact","update_shutdown::tests::fixture","--ignored","--nocapture"]).env("KNOTLESS_UPDATE_FIXTURE","1").env("KNOTLESS_UPDATE_XRAY",xray).stdout(std::process::Stdio::piped()).creation_flags(CREATE_NO_WINDOW).spawn().unwrap();
        let mut reader=BufReader::new(child.stdout.take().unwrap());let mut line=String::new();loop{line.clear();assert!(reader.read_line(&mut line).unwrap()>0);if line.contains("READY"){break;}}child.stdout=Some(reader.into_inner());
        let id=std::fs::read_to_string(root.join("core.pid")).unwrap().parse().unwrap();let core=open(id,PROCESS_SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION).unwrap();std::thread::sleep(std::time::Duration::from_secs(1));
        stop(&exe).unwrap();child.wait().unwrap();assert!(exited(core.0,0).unwrap());let mut code=1;unsafe{assert_ne!(GetExitCodeProcess(core.0,&mut code),0);}assert_eq!(code,0,"Xray must clean up normally before the legacy parent is stopped");
        for name in ["knotless.exe","core.json","core.log","core.pid"]{std::fs::remove_file(root.join(name)).unwrap();}std::fs::remove_dir(root).unwrap();
    }
}
