use crate::*;

#[derive(Clone,Default,Serialize)] #[serde(rename_all="camelCase")]
pub struct Telemetry{pub session:String,pub down_rate:f64,pub up_rate:f64,pub downloaded:u64,pub uploaded:u64,pub available:bool,pub health:String,pub detail:String,pub checked_at:u64}
pub fn configure(config:&mut Value,metrics_port:u16,health_port:u16)->Result<()>{
    config["stats"]=json!({});config["policy"]["system"]=json!({"statsInboundUplink":true,"statsInboundDownlink":true});
    config["metrics"]=json!({"tag":"knotless-metrics","listen":format!("127.0.0.1:{metrics_port}")});
    config["inbounds"].as_array_mut().ok_or("No inbounds")?.push(json!({"tag":"knotless-health","listen":"127.0.0.1","port":health_port,"protocol":"socks","settings":{"auth":"noauth","udp":false}}));
    config["routing"]["rules"].as_array_mut().ok_or("No routing rules")?.insert(0,json!({"type":"field","inboundTag":["knotless-health"],"outboundTag":"proxy"}));Ok(())
}
pub fn counters(data:&Value)->Option<(u64,u64)>{
    let inbound=data["stats"]["inbound"].as_object()?;
    let mut down=0;let mut up=0;
    for tag in ["socks-in","http-in","knotless-tun"]{if let Some(s)=inbound.get(tag){down+=s["downlink"].as_u64().unwrap_or(0);up+=s["uplink"].as_u64().unwrap_or(0);}}
    Some((down,up))
}
pub fn rate(old:u64,new:u64,seconds:f64)->f64{if seconds<=0.0{return 0.0;}new.saturating_sub(old)as f64/seconds}
#[tauri::command]pub fn telemetry(state:State<AppState>)->Result<Telemetry>{Ok(state.telemetry.lock().map_err(err)?.clone())}
fn epoch()->u64{std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()}
fn record_health(root:&Path,mode:&str,health:&str,detail:&str){
    use std::io::Write;
    let path=root.join("connection-health.jsonl");
    if fs::metadata(&path).map(|m|m.len()>128*1024).unwrap_or(false){let _=fs::rename(&path,root.join("connection-health.previous.jsonl"));}
    if let Ok(mut file)=fs::OpenOptions::new().create(true).append(true).open(path){let _=writeln!(file,"{}",json!({"at":epoch(),"mode":mode,"health":health,"detail":detail}));}
}
pub async fn check_path(port:Option<u16>)->bool{
    let mut builder=reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(7));
    if let Some(port)=port{let Ok(proxy)=reqwest::Proxy::all(format!("socks5h://127.0.0.1:{port}"))else{return false;};builder=builder.proxy(proxy);}
    let Ok(http)=builder.build()else{return false;};
    for url in ["https://www.gstatic.com/generate_204","https://cp.cloudflare.com/generate_204"]{
        if let Ok(r)=http.get(url).header("Cache-Control","no-cache").send().await{if r.status().as_u16()==204{return true;}}
    }false
}
fn snapshot(app:&tauri::AppHandle)->Option<(String,u16,u16,String)>{let state=app.state::<AppState>();let r=state.runtime.lock().ok()?;r.child.as_ref()?;Some((r.session.clone(),r.metrics_port,r.health_port,r.mode.clone()))}
pub fn start(app:tauri::AppHandle){
    let metrics_app=app.clone();tauri::async_runtime::spawn(async move{
        let Ok(http)=reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(2)).build()else{return;};
        let mut previous:Option<(String,u64,u64,Instant)>=None;
        loop{tokio::time::sleep(Duration::from_secs(1)).await;let state=metrics_app.state::<AppState>();if state.closing.load(Ordering::SeqCst){break;}
            let Some((session,port,_,_))=snapshot(&metrics_app)else{previous=None;continue;};
            let data=match http.get(format!("http://127.0.0.1:{port}/debug/vars")).send().await{Ok(r)=>r.json::<Value>().await.ok(),Err(_)=>None};
            let mut t=match state.telemetry.lock(){Ok(t)=>t,Err(_)=>continue};if t.session!=session{continue;}
            if let Some((down,up))=data.as_ref().and_then(counters){let now=Instant::now();let rates=previous.as_ref().filter(|(s,_,_,_)|s==&session).map(|(_,d,u,time)|(rate(*d,down,time.elapsed().as_secs_f64()),rate(*u,up,time.elapsed().as_secs_f64()))).unwrap_or((0.0,0.0));t.down_rate=rates.0;t.up_rate=rates.1;t.downloaded=down;t.uploaded=up;t.available=true;previous=Some((session,down,up,now));}else{t.available=false;t.down_rate=0.0;t.up_rate=0.0;previous=None;}
            let _=metrics_app.emit("telemetry",t.clone());
        }
    });
    tauri::async_runtime::spawn(async move{
        let mut failures=0;let mut last_session=String::new();let mut recoveries=0;let mut last_recovery=Instant::now()-Duration::from_secs(120);
        loop{tokio::time::sleep(Duration::from_secs(15)).await;let state=app.state::<AppState>();if state.closing.load(Ordering::SeqCst){break;}
            let Some((session,_,port,mode))=snapshot(&app)else{failures=0;recoveries=0;last_session.clear();continue;};
            if session!=last_session{failures=0;last_session=session.clone();}
            let proxy_ok=check_path(Some(port)).await;
            let system_ok=if mode=="tun"&&proxy_ok{check_path(None).await}else{proxy_ok};
            let ok=proxy_ok&&system_ok;
            if snapshot(&app).as_ref().map(|s|&s.0)!=Some(&session){continue;}
            failures=if ok{0}else{failures+1};
            {let mut t=state.telemetry.lock().unwrap();if t.session!=session{continue;}let health=if ok{"healthy"}else{"degraded"};let detail=if ok{""}else if proxy_ok{"SYSTEM_PATH_FAILED"}else{"PROXY_PATH_FAILED"};if t.health!=health||t.detail!=detail{record_health(&state.root,&mode,health,detail);let _=app.emit("connection-health",if ok{"connected"}else if proxy_ok{"systemPathFailed"}else{"proxyPathFailed"});}t.health=health.into();t.detail=detail.into();t.checked_at=epoch();let _=app.emit("telemetry",t.clone());}
            if !ok&&failures>=2&&mode=="tun"&&recoveries<3&&last_recovery.elapsed()>=Duration::from_secs(60)&&state.auto_reconnect.load(Ordering::SeqCst){
                if state.busy.compare_exchange(false,true,Ordering::SeqCst,Ordering::SeqCst).is_err(){continue;}
                let guard=BusyGuard(&state.busy);
                let profile={let r=state.runtime.lock().unwrap();if r.session==session{r.active_profile.clone()}else{None}};
                if let Some(profile)=profile{
                    recoveries+=1;last_recovery=Instant::now();
                    record_health(&state.root,&mode,"recovering","");let _=app.emit("connection-health","recovering");
                    {let mut t=state.telemetry.lock().unwrap();t.health="recovering".into();let _=app.emit("telemetry",t.clone());}
                    let handle=app.clone();let outcome=tauri::async_runtime::spawn_blocking(move||{stop_connection(&handle)?;connect_profile(profile,&handle)}).await;
                    if !matches!(outcome,Ok(Ok(()))){let _=app.emit("knotless-error","RECOVERY_FAILED");}
                }drop(guard);tray::refresh(&app);
            }
        }
    });
}
#[cfg(test)]mod tests{use super::*;
 #[test]fn excludes_health_and_metrics_traffic(){assert_eq!(counters(&json!({"stats":{"inbound":{"socks-in":{"downlink":100,"uplink":20},"knotless-tun":{"downlink":200,"uplink":30},"knotless-health":{"downlink":999,"uplink":999}}}})),Some((300,50)));}
 #[test]fn handles_counter_reset_and_elapsed_time(){assert_eq!(rate(100,300,2.0),100.0);assert_eq!(rate(300,100,1.0),0.0);assert_eq!(rate(0,100,0.0),0.0);}
}
