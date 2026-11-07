#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod windows_process;
mod update_shutdown;
mod tun;
mod dns;
mod tray;
mod monitor;
mod tcp_probe;
mod bundled_core;
mod subscriptions;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, io::{Cursor, Read}, net::TcpListener, path::{Path, PathBuf}, process::{Command, Stdio}, sync::{Mutex,atomic::{AtomicBool,Ordering}}, time::{Duration, Instant}};
use tauri::{Emitter, Manager, State};

type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String { e.to_string() }
const MAX_ARCHIVE: usize = 120 * 1024 * 1024;
#[cfg(target_arch="aarch64")] const CORE_DIRECTORY:&str="cores-arm64";
#[cfg(not(target_arch="aarch64"))] const CORE_DIRECTORY:&str="cores";
#[cfg(target_arch="aarch64")] const XRAY_ARCHIVE:&str="Xray-windows-arm64-v8a.zip";
#[cfg(not(target_arch="aarch64"))] const XRAY_ARCHIVE:&str="Xray-windows-64.zip";

struct ManagedChild { child: windows_process::Process, config_path: PathBuf, log_path: PathBuf, #[cfg(windows)] job: isize }
impl Drop for ManagedChild {
    fn drop(&mut self) {
        self.child.graceful_stop();
        #[cfg(windows)] unsafe { if self.job != 0 { windows_sys::Win32::Foundation::CloseHandle(self.job as _); } }
        let _ = fs::remove_file(&self.config_path); let _ = fs::remove_file(&self.log_path);
    }
}
#[derive(Default)] struct Runtime { child: Option<ManagedChild>, version: Option<String>, mode: String, name: String, session:String, metrics_port:u16,health_port:u16,active_profile:Option<ConnectionProfile> }
#[derive(Clone,Deserialize)] #[serde(rename_all="camelCase")] struct ConnectionProfile { version:String, config:Value, mode:String, name:String, #[serde(default)] dns:dns::Settings }
struct AppState { root: PathBuf, bundled_core:Option<String>, runtime: Mutex<Runtime>, profile:Mutex<Option<ConnectionProfile>>, tray_toggle:Mutex<Option<tauri::menu::MenuItem<tauri::Wry>>>, tray_open:Mutex<Option<tauri::menu::MenuItem<tauri::Wry>>>,tray_quit:Mutex<Option<tauri::menu::MenuItem<tauri::Wry>>>,language:Mutex<String>,subscriptions:Mutex<subscriptions::Store>,telemetry:Mutex<monitor::Telemetry>,auto_reconnect:AtomicBool,busy:AtomicBool, closing:AtomicBool, minimize_to_tray:AtomicBool, probes: tokio::sync::Semaphore, installs: tokio::sync::Mutex<()> }
#[derive(Serialize)] struct Core { version: String, path: String, bundled:bool }
#[derive(Clone,Serialize)] #[serde(rename_all="camelCase")] struct Status { running: bool, core: Option<String>, socks_port: u16, http_port: u16, mode:String, name:String, busy:bool, elevated:bool }
#[derive(Serialize)] struct Probe { ms: u64, status: u16, url: String }

fn valid_version(version: &str) -> Result<()> {
    if version.len() > 40 || !version.starts_with('v') || !version[1..].chars().all(|c| c.is_ascii_digit() || c == '.') || version[1..].split('.').any(|s| s.is_empty()) { return Err("Некорректная версия ядра.".into()); } Ok(())
}
fn core_path(root: &Path, version: &str) -> Result<PathBuf> { valid_version(version)?; let p = root.join(CORE_DIRECTORY).join(version).join("xray.exe"); if !p.is_file() { return Err("Выбранное ядро не установлено.".into()); } Ok(p) }
fn client() -> Result<reqwest::Client> { reqwest::Client::builder().no_proxy().user_agent("Knotless/0.1").timeout(Duration::from_secs(120)).build().map_err(err) }
#[tauri::command]
fn list_cores(state: State<AppState>) -> Result<Vec<Core>> {
    let mut result = vec![];
    for entry in fs::read_dir(state.root.join(CORE_DIRECTORY)).map_err(err)? {
        let e = entry.map_err(err)?; let name = e.file_name().to_string_lossy().to_string();
        if valid_version(&name).is_ok() && e.path().join("xray.exe").is_file() { result.push(Core { bundled:state.bundled_core.as_deref()==Some(name.as_str()), version: name, path: e.path().to_string_lossy().to_string() }); }
    }
    result.sort_by(|a,b| b.version.cmp(&a.version)); Ok(result)
}
#[tauri::command]
async fn list_releases() -> Result<Vec<String>> {
    let data: Vec<Value> = client()?.get("https://api.github.com/repos/XTLS/Xray-core/releases?per_page=30").send().await.map_err(err)?.error_for_status().map_err(err)?.json().await.map_err(err)?;
    Ok(data.iter().filter(|r| r["draft"] != true && r["prerelease"] != true).filter_map(|r| r["tag_name"].as_str()).filter(|s| valid_version(s).is_ok()).map(str::to_owned).collect())
}
async fn download(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>> {
    let mut response = client.get(url).send().await.map_err(err)?.error_for_status().map_err(err)?;
    if response.content_length().unwrap_or(0) > limit as u64 { return Err("Файл превышает допустимый размер.".into()); }
    let mut bytes = Vec::new(); while let Some(chunk) = response.chunk().await.map_err(err)? { if bytes.len() + chunk.len() > limit { return Err("Файл превышает допустимый размер.".into()); } bytes.extend_from_slice(&chunk); } Ok(bytes)
}
#[tauri::command]
async fn install_core(version: String, state: State<'_, AppState>) -> Result<()> {
    valid_version(&version)?; let _lock = state.installs.lock().await;
    install_package(&version, &state.root).await
}
async fn install_package(version: &str, root: &Path) -> Result<()> {
    valid_version(version)?;
    let target = root.join(CORE_DIRECTORY).join(version);
    if target.join("xray.exe").exists() { return Ok(()); }
    let http = client()?; let base = format!("https://github.com/XTLS/Xray-core/releases/download/{version}/{XRAY_ARCHIVE}");
    let bytes = download(&http, &base, MAX_ARCHIVE).await?;
    let checksum = String::from_utf8(download(&http, &format!("{base}.dgst"), 16384).await?).map_err(err)?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if !checksum.split(|c: char| !c.is_ascii_hexdigit()).any(|s| s.len() == 64 && s.eq_ignore_ascii_case(&digest)) { return Err("SHA-256 архива не совпадает с опубликованной контрольной суммой.".into()); }
    let staging = root.join(CORE_DIRECTORY).join(format!(".download-{}", uuid::Uuid::new_v4())); fs::create_dir_all(&staging).map_err(err)?;
    let result = (|| -> Result<()> {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(err)?;
        for name in ["xray.exe", "geoip.dat", "geosite.dat", "LICENSE"] {
            match archive.by_name(name) {
                Ok(file) => { if file.size() > MAX_ARCHIVE as u64 { return Err("Слишком большой файл в архиве.".into()); } let mut data = Vec::new(); file.take(MAX_ARCHIVE as u64 + 1).read_to_end(&mut data).map_err(err)?; if data.len() > MAX_ARCHIVE { return Err("Размер файла превышен.".into()); } fs::write(staging.join(name), data).map_err(err)?; },
                Err(_) if name == "xray.exe" => return Err("В архиве нет xray.exe.".into()), Err(_) => {},
            }
        }
        fs::rename(&staging, &target).map_err(err)?; Ok(())
    })();
    if staging.exists() { let _ = fs::remove_dir_all(&staging); } result
}
fn hidden_command(exe: &Path) -> Command {
    let mut c = Command::new(exe);
    #[cfg(windows)] { use std::os::windows::process::CommandExt; c.creation_flags(0x08000000); }
    c
}
fn spawn_managed(exe: &Path, config: &Value, root: &Path) -> Result<ManagedChild> {
    let id = uuid::Uuid::new_v4(); let config_path = root.join("runs").join(format!("{id}.json")); let log_path = root.join("runs").join(format!("{id}.log"));
    fs::write(&config_path, serde_json::to_vec(config).map_err(err)?).map_err(err)?;
    let log = fs::File::create(&log_path).map_err(err)?;
    let is_tun=config["inbounds"].as_array().is_some_and(|a|a.iter().any(|i|i["protocol"]=="tun"));
    let spawned = if is_tun { windows_process::spawn_console(exe,&config_path,&log) } else {
        hidden_command(exe).args(["run", "-c"]).arg(&config_path).current_dir(exe.parent().unwrap()).env("XRAY_LOCATION_ASSET", exe.parent().unwrap()).stdin(Stdio::null()).stdout(Stdio::from(log.try_clone().map_err(err)?)).stderr(Stdio::from(log)).spawn().map(windows_process::Process::Plain)
    };
    let child = match spawned { Ok(c) => c, Err(e) => { let _ = fs::remove_file(&config_path); let _ = fs::remove_file(&log_path); return Err(err(e)); } };
    let mut managed = ManagedChild { child, config_path, log_path, #[cfg(windows)] job: 0 };
    #[cfg(windows)] unsafe {
        use windows_sys::Win32::System::JobObjects::*;
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() { return Err("Не удалось создать Windows Job Object для Xray.".into()); }
        managed.job = job as isize;
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed(); info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as _, std::mem::size_of_val(&info) as u32) == 0 || AssignProcessToJobObject(job, managed.child.raw_handle()) == 0 { return Err("Не удалось привязать Xray к процессу приложения.".into()); }
    }
    Ok(managed)
}
fn prepare_config(mut config: Value, socks: u16, http: Option<u16>) -> Result<Value> {
    if !config.is_object() || !config["outbounds"].is_array() { return Err("Некорректная конфигурация Xray.".into()); }
    let mut inbounds = vec![json!({"tag":"socks-in", "listen":"127.0.0.1", "port":socks, "protocol":"socks", "settings":{"auth":"noauth", "udp":true}, "sniffing":{"enabled":true,"destOverride":["http","tls"],"routeOnly":true}})];
    if let Some(p) = http { inbounds.push(json!({"tag":"http-in", "listen":"127.0.0.1", "port":p, "protocol":"http", "settings":{}})); }
    config["inbounds"] = json!(inbounds); config["log"] = json!({"loglevel":"warning"}); Ok(config)
}
fn preflight(exe: &Path, config: &Value, root: &Path) -> Result<()> {
    let path = root.join("runs").join(format!("check-{}.json", uuid::Uuid::new_v4())); fs::write(&path, serde_json::to_vec(config).map_err(err)?).map_err(err)?;
    let output = hidden_command(exe).args(["run", "-test", "-c"]).arg(&path).current_dir(exe.parent().unwrap()).env("XRAY_LOCATION_ASSET", exe.parent().unwrap()).stdin(Stdio::null()).stdout(if cfg!(test){Stdio::inherit()}else{Stdio::null()}).stderr(if cfg!(test){Stdio::inherit()}else{Stdio::null()}).spawn();
    let result = match output {
        Ok(mut child) => { let start = Instant::now(); loop { match child.try_wait() { Ok(Some(s)) => break if s.success() { Ok(()) } else { Err("Выбранное ядро отклонило конфигурацию. Проверьте параметры сервера, правила и наличие geodata.".into()) }, Ok(None) if start.elapsed() < Duration::from_secs(10) => std::thread::sleep(Duration::from_millis(50)), _ => { let _ = child.kill(); let _ = child.wait(); break Err("Проверка конфигурации не завершилась.".into()); } } } },
        Err(e) => Err(err(e))
    }; let _ = fs::remove_file(path); result
}
fn wait_ready(child: &mut ManagedChild, port: u16) -> Result<()> {
    let start = Instant::now(); while start.elapsed() < Duration::from_secs(25) {
        if child.child.try_wait().map_err(err)?.is_some() { return Err("Xray завершился при запуске. Проверьте совместимость ядра и конфигурации.".into()); }
        if std::net::TcpStream::connect_timeout(&format!("127.0.0.1:{port}").parse().unwrap(), Duration::from_millis(100)).is_ok() { std::thread::sleep(Duration::from_millis(100)); if child.child.try_wait().map_err(err)?.is_none() { return Ok(()); } }
        std::thread::sleep(Duration::from_millis(50));
    } Err("Xray не открыл локальный порт за отведённое время.".into())
}
fn wait_tun_ready(child:&mut ManagedChild)->Result<()> {
    let start=Instant::now();
    while start.elapsed()<Duration::from_secs(30){
        if child.child.try_wait().map_err(err)?.is_some(){return Err("Xray не смог включить TUN. Проверьте права администратора, ядро и Wintun.".into());}
        if fs::read_to_string(&child.log_path).unwrap_or_default().contains("Knotless up"){return Ok(());}
        std::thread::sleep(Duration::from_millis(100));
    }Err("Тайм-аут подготовки TUN-интерфейса. Подключение отменено.".into())
}
struct BusyGuard<'a>(&'a AtomicBool);
impl Drop for BusyGuard<'_> {fn drop(&mut self){self.0.store(false,Ordering::SeqCst);}}
fn check_fragment_core(version:&str,config:&Value)->Result<()> {
    if !tun::supported(version) && config["outbounds"].as_array().is_some_and(|outs|outs.iter().any(|o|!o["streamSettings"]["finalmask"].is_null())) {return Err("FRAGMENT_CORE".into());} Ok(())
}
fn connect_profile(profile:ConnectionProfile,app:&tauri::AppHandle)->Result<()> {
        let active_profile=profile.clone();
        let ConnectionProfile{version,config,mode,name,dns}=profile;
        let state = app.state::<AppState>();
        if state.closing.load(Ordering::SeqCst) {return Err("Приложение завершает работу.".into());}
        if state.runtime.lock().map_err(err)?.child.is_some() { return Err("Сначала отключите текущее соединение.".into()); }
        if mode!="proxy"&&mode!="tun" {return Err("Неизвестный режим подключения.".into());}
        if mode=="tun" {
            if !tun::supported(&version) {return Err(format!("Для TUN нужно Xray {} или новее. Выберите новое ядро.",tun::MIN_VERSION));}
            if !windows_process::is_elevated(){return Err("Для TUN запустите Knotless от имени администратора — кнопка в настройках подключения.".into());}
        }
        check_fragment_core(&version,&config)?;
        let exe = core_path(&state.root, &version)?;
        let socks = TcpListener::bind("127.0.0.1:10808").map_err(|_| "Порт 10808 занят другим приложением.")?;
        let http = TcpListener::bind("127.0.0.1:10809").map_err(|_| "Порт 10809 занят другим приложением.")?;
        let mut config = prepare_config(config, 10808, Some(10809))?;
        if mode=="tun" { tun::ensure_driver(&app.path().resource_dir().map_err(err)?,&exe)?; dns::pin_endpoint(&mut config)?; tun::add_with_dns(&mut config,&dns)?; }
        let metrics_socket=TcpListener::bind("127.0.0.1:0").map_err(err)?;let metrics_port=metrics_socket.local_addr().map_err(err)?.port();
        let health_socket=TcpListener::bind("127.0.0.1:0").map_err(err)?;let health_port=health_socket.local_addr().map_err(err)?.port();
        monitor::configure(&mut config,metrics_port,health_port)?;
        preflight(&exe, &config, &state.root)?; drop(socks); drop(http);drop(metrics_socket);drop(health_socket);
        let mut child = spawn_managed(&exe, &config, &state.root)?; wait_ready(&mut child, 10808)?; wait_ready(&mut child, 10809)?;
        if mode=="tun" {wait_tun_ready(&mut child)?;}
        if state.closing.load(Ordering::SeqCst){return Err("Приложение завершает работу.".into());}
        let mut runtime=state.runtime.lock().map_err(err)?;
        let session=uuid::Uuid::new_v4().to_string();
        runtime.child = Some(child); runtime.version = Some(version); runtime.mode=mode; runtime.name=name;runtime.session=session.clone();runtime.metrics_port=metrics_port;runtime.health_port=health_port;runtime.active_profile=Some(active_profile);
        *state.telemetry.lock().map_err(err)?=monitor::Telemetry{session,health:"checking".into(),..Default::default()};Ok(())
}
#[tauri::command]
async fn connect(version:String,config:Value,mode:String,name:String,dns:Option<dns::Settings>,app:tauri::AppHandle)->Result<()> {
    run_connection(app,Some(ConnectionProfile{version,config,mode,name,dns:dns.unwrap_or_default()})).await
}
async fn run_connection(app:tauri::AppHandle,profile:Option<ConnectionProfile>)->Result<()> {
    let state=app.state::<AppState>();
    if state.busy.swap(true,Ordering::SeqCst){return Err("Операция подключения уже выполняется.".into());}
    let _guard=BusyGuard(&state.busy); tray::refresh(&app);
    let handle=app.clone();
    let result=tauri::async_runtime::spawn_blocking(move|| match profile {Some(p)=>connect_profile(p,&handle),None=>stop_connection(&handle)}).await.map_err(err).and_then(|r|r);
    drop(_guard); tray::refresh(&app); result
}
fn stop_connection(app:&tauri::AppHandle)->Result<()> {let state=app.state::<AppState>();let mut r=state.runtime.lock().map_err(err)?;r.child.take();r.version=None;r.name.clear();r.mode.clear();r.session.clear();r.active_profile=None;*state.telemetry.lock().map_err(err)?=monitor::Telemetry::default();let _=app.emit("telemetry",monitor::Telemetry::default());Ok(())}
#[tauri::command]
async fn disconnect(app:tauri::AppHandle)->Result<()> {run_connection(app,None).await}
#[tauri::command]
fn set_connection_profile(profile:Option<ConnectionProfile>,app:tauri::AppHandle)->Result<()> { *app.state::<AppState>().profile.lock().map_err(err)?=profile;tray::refresh(&app);Ok(()) }
#[tauri::command]
fn set_window_behavior(minimize_to_tray:bool,state:State<AppState>){state.minimize_to_tray.store(minimize_to_tray,Ordering::SeqCst);}
#[tauri::command]fn set_auto_reconnect(enabled:bool,state:State<AppState>){state.auto_reconnect.store(enabled,Ordering::SeqCst);}
#[tauri::command]fn set_language(language:String,app:tauri::AppHandle)->Result<()>{if !["ru","en","zh"].contains(&language.as_str()){return Err("LANGUAGE_INVALID".into());}*app.state::<AppState>().language.lock().map_err(err)?=language;tray::refresh(&app);Ok(())}
#[tauri::command]
fn restart_as_admin(app:tauri::AppHandle)->Result<()> {
    let state=app.state::<AppState>();if state.busy.load(Ordering::SeqCst)||state.runtime.lock().map_err(err)?.child.is_some(){return Err("Перед перезапуском отключите соединение.".into());}
    windows_process::relaunch_elevated().map_err(err)?;state.closing.store(true,Ordering::SeqCst);app.exit(0);Ok(())
}
fn status(state:&AppState)->Result<Status>{
    let r=state.runtime.lock().map_err(err)?;
    Ok(Status{running:r.child.is_some(),core:r.version.clone(),socks_port:10808,http_port:10809,mode:r.mode.clone(),name:r.name.clone(),busy:state.busy.load(Ordering::SeqCst),elevated:windows_process::is_elevated()})
}
#[tauri::command]
fn runtime_status(state: State<AppState>) -> Result<Status> {
    status(&state)
}
#[tauri::command]
async fn tcp_probe(host:String,port:u16)->Result<f64>{tcp_probe::probe(host,port).await}
#[tauri::command]
async fn https_probe(version: String, mut config: Value, url: String, expected: u16, app: tauri::AppHandle) -> Result<Probe> {
    check_fragment_core(&version,&config)?;
    let parsed = reqwest::Url::parse(&url).map_err(err)?;
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() || !(200..300).contains(&expected) { return Err("Для проверки нужен HTTPS URL и статус 200–299.".into()); }
    let state = app.state::<AppState>(); let _permit = state.probes.acquire().await.map_err(err)?;
    let proxy = config["outbounds"].as_array().and_then(|a| a.iter().find(|o| o["tag"] == "proxy" && o["protocol"] == "vless")).cloned().ok_or("Нет VLESS outbound для проверки.")?;
    // Never trust user routing for a health check: only the selected proxy can egress.
    config = json!({"outbounds":[proxy], "routing":{"domainStrategy":"AsIs","rules":[]}});
    let exe = core_path(&state.root, &version)?; let root = state.root.clone();
    let (managed, port) = tauri::async_runtime::spawn_blocking(move || -> Result<(ManagedChild,u16)> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(err)?; let port = listener.local_addr().map_err(err)?.port();
        let config = prepare_config(config, port, None)?; preflight(&exe, &config, &root)?; drop(listener);
        let mut managed = spawn_managed(&exe, &config, &root)?; wait_ready(&mut managed, port)?; Ok((managed, port))
    }).await.map_err(err)??;
    let proxy = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{port}")).map_err(err)?;
    let http = reqwest::Client::builder().no_proxy().proxy(proxy).redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(12)).build().map_err(err)?;
    let start = Instant::now(); let mut response = http.get(&url).header("Cache-Control", "no-cache").send().await.map_err(|e| format!("HTTPS через сервер: {e}"))?;
    let status = response.status().as_u16(); if status != expected { return Err(format!("Ожидался HTTP {expected}, получен {status}.")); }
    let mut received = 0; while let Some(chunk) = response.chunk().await.map_err(err)? { received += chunk.len(); if received > 1024 * 1024 { return Err("Ответ проверки превышает 1 МБ.".into()); } }
    let result = Probe { ms: start.elapsed().as_millis() as u64, status, url }; drop(managed); Ok(result)
}
fn main() {
    update_shutdown::handle_args();
    if windows_process::handle_helper_args(){return;}
    // Elevate before creating the GUI, WebView, tray or single-instance mutex.
    // Installer and console helpers above retain their own privilege handling.
    if !windows_process::ensure_gui_elevation(){return;}
    #[cfg(windows)] unsafe {
        use windows_sys::Win32::{System::Threading::CreateMutexW, Foundation::{GetLastError, ERROR_ALREADY_EXISTS}};
        let name: Vec<u16> = "Local\\Knotless.Desktop.0.1\0".encode_utf16().collect();
        let mutex = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if mutex.is_null() || GetLastError() == ERROR_ALREADY_EXISTS { return; }
        // The OS releases this process-lifetime mutex when Knotless exits.
    }
    tauri::Builder::default().setup(|app| {
        let root = app.path().app_local_data_dir()?;
        fs::create_dir_all(root.join(CORE_DIRECTORY))?; fs::create_dir_all(root.join("runs"))?;
        // Run files can contain credentials. Remove stale files belonging to Knotless on startup.
        for entry in fs::read_dir(root.join("runs"))?.flatten() { if entry.file_type()?.is_file() { let _ = fs::remove_file(entry.path()); } }
        let bundled_core=bundled_core::install(&root,&app.path().resource_dir()?)?;
        let subscriptions=subscriptions::load(&root);
        app.manage(AppState { root, bundled_core, runtime:Mutex::new(Runtime::default()), profile:Mutex::new(None),tray_toggle:Mutex::new(None),tray_open:Mutex::new(None),tray_quit:Mutex::new(None),language:Mutex::new("ru".into()),subscriptions:Mutex::new(subscriptions),telemetry:Mutex::new(monitor::Telemetry::default()),auto_reconnect:AtomicBool::new(true),busy:AtomicBool::new(false),closing:AtomicBool::new(false),minimize_to_tray:AtomicBool::new(true),probes:tokio::sync::Semaphore::new(3), installs:tokio::sync::Mutex::new(()) });
        tray::setup(app)?;update_shutdown::start(app.handle().clone())?;monitor::start(app.handle().clone());subscriptions::start_scheduler(app.handle().clone()); Ok(())
    }).invoke_handler(tauri::generate_handler![list_cores,list_releases,install_core,connect,disconnect,runtime_status,tcp_probe,https_probe,set_connection_profile,set_window_behavior,restart_as_admin,set_language,set_auto_reconnect,monitor::telemetry,subscriptions::get_hwid,subscriptions::list_subscriptions,subscriptions::add_subscription,subscriptions::configure_subscription,subscriptions::refresh_subscription,subscriptions::remove_subscription])
    .on_window_event(|window,event| {
        let state=window.state::<AppState>();
        if !state.closing.load(Ordering::SeqCst)&&state.minimize_to_tray.load(Ordering::SeqCst){match event {
            tauri::WindowEvent::CloseRequested{api,..}=>{if window.hide().is_ok(){api.prevent_close();}},
            tauri::WindowEvent::Resized(_)=>{if window.is_minimized().unwrap_or(false){let _=window.hide();}},_=>{}
        }}
        if let tauri::WindowEvent::Destroyed=event {let _=stop_connection(window.app_handle());}
    })
    .run(tauri::generate_context!()).expect("Could not start Knotless");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires downloaded Xray and the built Knotless helper executable"]
    fn console_shutdown_and_tun_preflight() {
        let root=PathBuf::from(std::env::var("KNOTLESS_TEST_ROOT").unwrap());
        let exe=core_path(&root,"v26.10.31").unwrap();
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();drop(listener);
        let base=json!({"outbounds":[{"tag":"proxy","protocol":"freedom"}],"routing":{"rules":[]}});
        let config=prepare_config(base,port,None).unwrap();
        let mut tun_config=config.clone();tun::add_inbound(&mut tun_config).unwrap();preflight(&exe,&tun_config,&root).unwrap();
        let id=uuid::Uuid::new_v4();let path=root.join("runs").join(format!("{id}.json"));let logpath=root.join("runs").join(format!("{id}.log"));
        fs::write(&path,serde_json::to_vec(&config).unwrap()).unwrap();let log=fs::File::create(&logpath).unwrap();
        let process=windows_process::spawn_console(&exe,&path,&log).unwrap();
        let mut managed=ManagedChild{child:process,config_path:path,log_path:logpath,job:0};
        wait_ready(&mut managed,port).unwrap();managed.child.graceful_stop();
        let exit=managed.child.try_wait().unwrap().unwrap();assert!(exit.success(),"Xray must exit normally, not via TerminateProcess: {exit}");
        assert!(std::net::TcpStream::connect(("127.0.0.1",port)).is_err());
    }
    #[test] fn rejects_path_traversal_versions() { for v in ["../v1", "v1/../../", "v", "v1..2", "v1.exe"] { assert!(valid_version(v).is_err()); } assert!(valid_version("v26.9.11").is_ok()); }
    #[test] fn always_binds_loopback() { let c = prepare_config(json!({"outbounds":[],"inbounds":[{"listen":"0.0.0.0"}]}), 1234, Some(1235)).unwrap(); assert_eq!(c["inbounds"][0]["listen"], "127.0.0.1"); assert_eq!(c["inbounds"][1]["port"], 1235); }
    #[test]
    #[ignore = "Downloads official Xray and makes an HTTPS request through a local VLESS test server"]
    fn real_xray_roundtrip() {
        let root = PathBuf::from(std::env::var("KNOTLESS_TEST_ROOT").expect("set KNOTLESS_TEST_ROOT"));
        fs::create_dir_all(root.join(CORE_DIRECTORY)).unwrap(); fs::create_dir_all(root.join("runs")).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let version = "v26.10.31".to_string();
            if core_path(&root,&version).is_err(){install_package(&version, &root).await.unwrap();} let exe = core_path(&root, &version).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap(); let server_port = listener.local_addr().unwrap().port(); drop(listener);
            let id = "4ce82b94-cf44-4415-879c-7a8c29925475";
            let server_config = json!({"log":{"loglevel":"warning"},"inbounds":[{"listen":"127.0.0.1","port":server_port,"protocol":"vless","settings":{"clients":[{"id":id}],"decryption":"none"}}],"outbounds":[{"protocol":"freedom"}]});
            preflight(&exe,&server_config,&root).unwrap(); let mut server = spawn_managed(&exe,&server_config,&root).unwrap(); wait_ready(&mut server,server_port).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap(); let proxy_port = listener.local_addr().unwrap().port(); drop(listener);
            let mut client_config = prepare_config(json!({"routing":{"rules":[]},"outbounds":[{"tag":"proxy","protocol":"vless","settings":{"vnext":[{"address":"127.0.0.1","port":server_port,"users":[{"id":id,"encryption":"none"}]}]},"streamSettings":{"network":"tcp","security":"none"}}]}),proxy_port,None).unwrap();
            let m=TcpListener::bind("127.0.0.1:0").unwrap();let metrics_port=m.local_addr().unwrap().port();
            let h=TcpListener::bind("127.0.0.1:0").unwrap();let health_port=h.local_addr().unwrap().port();
            let d=std::net::UdpSocket::bind("127.0.0.1:0").unwrap();let dns_port=d.local_addr().unwrap().port();
            tun::add_inbound(&mut client_config).unwrap();
            // Exercise the exact TUN DNS routing without modifying Windows routes.
            let inbounds=client_config["inbounds"].as_array_mut().unwrap();inbounds.retain(|v|v["protocol"]!="tun");
            inbounds.push(json!({"tag":"knotless-tun","listen":"127.0.0.1","port":dns_port,"protocol":"dokodemo-door","settings":{"address":"192.168.1.1","port":53,"network":"udp"}}));
            monitor::configure(&mut client_config,metrics_port,health_port).unwrap();drop(m);drop(h);drop(d);
            preflight(&exe,&client_config,&root).unwrap(); let mut client = spawn_managed(&exe,&client_config,&root).unwrap(); wait_ready(&mut client,proxy_port).unwrap();
            let http = reqwest::Client::builder().no_proxy().proxy(reqwest::Proxy::all(format!("socks5h://127.0.0.1:{proxy_port}")).unwrap()).redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(12)).build().unwrap();
            let response = http.get("https://www.gstatic.com/generate_204").send().await.unwrap(); assert_eq!(response.status(),204);
            for round in 0..7{
                assert!(monitor::check_path(Some(health_port)).await,"health failed at round {round}");
                let socket=tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
                let packet=b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x07example\x03com\x00\x00\x01\x00\x01";
                socket.send_to(packet,("127.0.0.1",dns_port)).await.unwrap();let mut answer=[0u8;4096];
                let(n,_)=tokio::time::timeout(Duration::from_secs(10),socket.recv_from(&mut answer)).await.expect("DNS rewrite timed out").unwrap();assert!(n>12);assert_eq!(&answer[..2],&packet[..2]);assert_eq!(answer[3]&15,0,"DNS returned an error");assert!(u16::from_be_bytes([answer[6],answer[7]])>0);
                let metrics=reqwest::Client::builder().no_proxy().build().unwrap().get(format!("http://127.0.0.1:{metrics_port}/debug/vars")).send().await.unwrap().json::<Value>().await.unwrap();
                let(down,up)=monitor::counters(&metrics).expect("real metrics structure");assert!(down>0&&up>0,"traffic counters must increase");
                println!("Round {round}: HTTPS healthy, DNS rewritten through VLESS, traffic {down}/{up}");
                if round<6{tokio::time::sleep(Duration::from_secs(15)).await;}
            }
            drop(server);
            assert!(!monitor::check_path(Some(health_port)).await,"health must detect upstream failure");
            // TCP to the client's SOCKS listener still succeeds, but the end-to-end request must fail.
            assert!(std::net::TcpStream::connect(("127.0.0.1", proxy_port)).is_ok());
            let fresh = reqwest::Client::builder().no_proxy().proxy(reqwest::Proxy::all(format!("socks5h://127.0.0.1:{proxy_port}")).unwrap()).timeout(Duration::from_secs(3)).build().unwrap();
            assert!(fresh.get("https://www.gstatic.com/generate_204").send().await.is_err());
            let path = client.config_path.clone(); drop(client); assert!(!path.exists());
            assert!(std::net::TcpStream::connect(("127.0.0.1", proxy_port)).is_err());
            println!("Verified {version}: real HTTPS 204 over VLESS, upstream failure, process and config cleanup.");
        });
    }
}
