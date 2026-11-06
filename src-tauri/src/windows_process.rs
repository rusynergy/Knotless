//! TUN needs a graceful console interrupt so Xray can undo its network setup.
//! The console is hidden at creation; signaling occurs in a short-lived helper
//! process so attaching a console cannot interfere with the GUI or test runner.
use std::{io, os::windows::{io::AsRawHandle, process::{CommandExt, ExitStatusExt}}, path::Path, process::{Child, Command, ExitStatus}, time::{Duration, Instant}};
use windows_sys::Win32::{Foundation::*, System::{Console::*, Threading::*}, UI::WindowsAndMessaging::*};

fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_ref().encode_wide().chain(Some(0)).collect()
}
pub enum Process { Plain(Child), Console(ConsoleProcess) }
pub struct ConsoleProcess { handle: isize, id: u32 }
impl Drop for ConsoleProcess { fn drop(&mut self) { unsafe { CloseHandle(self.handle as _); } } }
impl Process {
    pub fn raw_handle(&self) -> HANDLE { match self { Self::Plain(p) => p.as_raw_handle() as _, Self::Console(p) => p.handle as _ } }
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match self {
            Self::Plain(p) => p.try_wait(),
            Self::Console(p) => unsafe { if WaitForSingleObject(p.handle as _,0) == WAIT_TIMEOUT { return Ok(None); } let mut code=0; if GetExitCodeProcess(p.handle as _,&mut code)==0 { return Err(io::Error::last_os_error()); } Ok(Some(ExitStatus::from_raw(code))) }
        }
    }
    pub fn kill(&mut self) -> io::Result<()> { match self { Self::Plain(p)=>p.kill(), Self::Console(p)=>unsafe { if TerminateProcess(p.handle as _,1)==0 { Err(io::Error::last_os_error()) } else { Ok(()) } } } }
    pub fn wait(&mut self) -> io::Result<()> { match self { Self::Plain(p)=>{p.wait()?;},Self::Console(p)=>unsafe { WaitForSingleObject(p.handle as _,5000); } }; Ok(()) }
    pub fn graceful_stop(&mut self) {
        if self.try_wait().ok().flatten().is_some() { return; }
        if let Self::Console(p)=self {
            if let Ok(exe)=helper_exe() {
                let _ = Command::new(exe).args(["--stop-xray",&p.id.to_string()]).creation_flags(CREATE_NO_WINDOW).status();
            }
            let start=Instant::now();
            while start.elapsed()<Duration::from_secs(8) { if self.try_wait().ok().flatten().is_some() { return; } std::thread::sleep(Duration::from_millis(50)); }
        }
        let _=self.kill(); let _=self.wait();
    }
}
fn helper_exe()->io::Result<std::path::PathBuf>{
    #[cfg(test)] if let Some(path)=std::env::var_os("KNOTLESS_HELPER_EXE"){return Ok(path.into());}
    std::env::current_exe()
}
pub fn spawn_console(exe:&Path, config:&Path, log:&std::fs::File)->io::Result<Process> {
    let application=wide(exe); let directory=wide(exe.parent().unwrap());
    let mut command=wide(format!("\"{}\" run -c \"{}\"",exe.display(),config.display()));
    unsafe {
        let mut startup:STARTUPINFOW=std::mem::zeroed(); startup.cb=std::mem::size_of_val(&startup) as u32;
        startup.dwFlags=STARTF_USESHOWWINDOW|STARTF_USESTDHANDLES; startup.wShowWindow=SW_HIDE as u16;
        startup.hStdOutput=log.as_raw_handle() as _; startup.hStdError=startup.hStdOutput;
        if SetHandleInformation(startup.hStdOutput,HANDLE_FLAG_INHERIT,HANDLE_FLAG_INHERIT)==0 { return Err(io::Error::last_os_error()); }
        let mut process:PROCESS_INFORMATION=std::mem::zeroed();
        let ok=CreateProcessW(application.as_ptr(),command.as_mut_ptr(),std::ptr::null(),std::ptr::null(),1,CREATE_NEW_CONSOLE|CREATE_NEW_PROCESS_GROUP,std::ptr::null(),directory.as_ptr(),&startup,&mut process);
        SetHandleInformation(startup.hStdOutput,HANDLE_FLAG_INHERIT,0);
        if ok==0 { return Err(io::Error::last_os_error()); }
        CloseHandle(process.hThread);
        Ok(Process::Console(ConsoleProcess{handle:process.hProcess as isize,id:process.dwProcessId}))
    }
}
pub fn handle_helper_args()->bool {
    let args:Vec<String>=std::env::args().collect();
    if args.get(1).map(String::as_str)==Some("--stop-xray") {
        let mut code=87;
        if let Some(id)=args.get(2).and_then(|s|s.parse::<u32>().ok()) { unsafe {
            if AttachConsole(id)!=0 { SetConsoleCtrlHandler(None,1);code=if GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT,id)!=0{0}else{GetLastError()};FreeConsole(); }
            else{code=GetLastError();}
        } }
        std::process::exit(code as i32);
    }
    if args.get(1).map(String::as_str)==Some("--wait-for-parent") {
        if let Some(id)=args.get(2).and_then(|s|s.parse::<u32>().ok()) { unsafe { let h=OpenProcess(PROCESS_SYNCHRONIZE,0,id); if !h.is_null() { WaitForSingleObject(h,15000); CloseHandle(h); } } }
    }
    false
}
pub fn is_elevated()->bool {
    use windows_sys::Win32::Security::*;
    unsafe { let mut token=std::ptr::null_mut(); if OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&mut token)==0 { return false; } let mut elevation:TOKEN_ELEVATION=std::mem::zeroed(); let mut returned=0; let ok=GetTokenInformation(token,TokenElevation,&mut elevation as *mut _ as _,std::mem::size_of_val(&elevation) as u32,&mut returned); CloseHandle(token); ok!=0&&elevation.TokenIsElevated!=0 }
}
pub fn relaunch_elevated()->io::Result<()> {
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW,SHELLEXECUTEINFOW,SEE_MASK_NOASYNC,SEE_MASK_FLAG_NO_UI};
    let exe=wide(std::env::current_exe()?); let verb=wide("runas"); let args=wide(format!("--wait-for-parent {}",std::process::id()));
    unsafe {
        let mut info:SHELLEXECUTEINFOW=std::mem::zeroed();info.cbSize=std::mem::size_of_val(&info) as u32;
        info.fMask=SEE_MASK_NOASYNC|SEE_MASK_FLAG_NO_UI;info.lpVerb=verb.as_ptr();info.lpFile=exe.as_ptr();info.lpParameters=args.as_ptr();info.nShow=SW_SHOWNORMAL;
        if ShellExecuteExW(&mut info)==0{Err(io::Error::last_os_error())}else{Ok(())}
    }
}
/// Every GUI launch requests elevation, including after a Windows reboot.
/// Never show a non-elevated window that needs a second manual restart for TUN.
pub fn ensure_gui_elevation()->bool {
    if is_elevated(){return true;}
    // A rejected/ineffective elevation must not start an endless relaunch loop.
    let result=if std::env::args().nth(1).as_deref()==Some("--wait-for-parent"){
        Err(io::Error::from_raw_os_error(ERROR_ELEVATION_REQUIRED as i32))
    }else{relaunch_elevated()};
    if let Err(error)=result{
        if error.raw_os_error()!=Some(ERROR_CANCELLED as i32){
            let message=wide(format!("Knotless could not obtain administrator rights.\n\n{error}"));
            unsafe{MessageBoxW(std::ptr::null_mut(),message.as_ptr(),wide("Knotless").as_ptr(),MB_OK|MB_ICONERROR);}
        }
    }
    false
}
