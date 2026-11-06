use crate::*;
use tauri::{menu::{Menu,MenuItem,PredefinedMenuItem},tray::{MouseButton,MouseButtonState,TrayIconBuilder,TrayIconEvent}};

pub fn show(app:&tauri::AppHandle){if let Some(w)=app.get_webview_window("main"){let _=w.unminimize();let _=w.show();let _=w.set_focus();}}
pub fn refresh(app:&tauri::AppHandle){
    let state=app.state::<AppState>();
    let language=state.language.lock().map(|l|l.clone()).unwrap_or_default();
    let words=match language.as_str(){"en"=>["Please wait…","Disconnect","Connect","Open Knotless","Exit","Disconnected","Proxy"],"zh"=>["请稍候…","断开连接","连接","打开 Knotless","退出","未连接","代理"],_=>["Подождите…","Отключиться","Подключиться","Открыть Knotless","Выход","Отключён","Прокси"]};
    if let Ok(s)=status(&state){
        let ready=state.profile.lock().map(|p|p.is_some()).unwrap_or(false);
        if let Ok(item)=state.tray_toggle.lock(){if let Some(item)=item.as_ref(){let _=item.set_text(if s.busy{words[0]}else if s.running{words[1]}else{words[2]});let _=item.set_enabled(!s.busy&&(s.running||ready));}}
        if let Ok(item)=state.tray_open.lock(){if let Some(item)=item.as_ref(){let _=item.set_text(words[3]);}}
        if let Ok(item)=state.tray_quit.lock(){if let Some(item)=item.as_ref(){let _=item.set_text(words[4]);}}
        if let Some(tray)=app.tray_by_id("knotless"){let _=tray.set_tooltip(Some(if s.running{format!("Knotless — {} ({})",s.name,if s.mode=="tun"{"TUN"}else{words[6]})}else{format!("Knotless — {}",words[5])}));}
        let _=app.emit("runtime-status",s);
    }
}
pub fn setup(app:&tauri::App)->std::result::Result<(),Box<dyn std::error::Error>>{
    let open=MenuItem::with_id(app,"show","Открыть Knotless",true,None::<&str>)?;
    let toggle=MenuItem::with_id(app,"toggle","Подключиться",false,None::<&str>)?;
    let quit=MenuItem::with_id(app,"quit","Выход",true,None::<&str>)?;
    let separator=PredefinedMenuItem::separator(app)?;
    let menu=Menu::with_items(app,&[&open,&toggle,&separator,&quit])?;
    *app.state::<AppState>().tray_toggle.lock().unwrap()=Some(toggle);
    *app.state::<AppState>().tray_open.lock().unwrap()=Some(open);
    *app.state::<AppState>().tray_quit.lock().unwrap()=Some(quit);
    TrayIconBuilder::with_id("knotless").icon(app.default_window_icon().ok_or("Нет иконки приложения")?.clone()).tooltip("Knotless — отключён").menu(&menu).show_menu_on_left_click(false)
        .on_tray_icon_event(|tray,event|{if matches!(event,TrayIconEvent::Click{button:MouseButton::Left,button_state:MouseButtonState::Up,..}){show(tray.app_handle());}})
        .on_menu_event(|app,event|match event.id.as_ref(){
            "show"=>show(app),
            "toggle"=>{let app=app.clone();tauri::async_runtime::spawn(async move{
                let state=app.state::<AppState>();
                let running=state.runtime.lock().map(|r|r.child.is_some()).unwrap_or(false);
                let profile=if running{None}else{state.profile.lock().ok().and_then(|p|p.clone())};
                if !running&&profile.is_none(){show(&app);return;}
                if let Err(e)=run_connection(app.clone(),profile).await{let _=app.emit("knotless-error",e);show(&app);}
            });},
            "quit"=>{let app=app.clone();app.state::<AppState>().closing.store(true,Ordering::SeqCst);tauri::async_runtime::spawn_blocking(move||{let _=stop_connection(&app);app.exit(0);});},_=>{}
        }).build(app)?;
    let app=app.handle().clone();
    // Native monitoring continues while the webview is hidden or throttled.
    std::thread::spawn(move||loop{
        std::thread::sleep(Duration::from_secs(1));let state=app.state::<AppState>();if state.closing.load(Ordering::SeqCst){break;}
        let mut stopped=false;
        if let Ok(mut r)=state.runtime.try_lock(){if let Some(c)=r.child.as_mut(){if c.child.try_wait().ok().flatten().is_some(){r.child.take();r.version=None;r.mode.clear();r.name.clear();stopped=true;}}}
        if stopped{refresh(&app);let _=app.emit("knotless-error","Xray неожиданно завершился. Соединение отключено.");}
    });Ok(())
}
