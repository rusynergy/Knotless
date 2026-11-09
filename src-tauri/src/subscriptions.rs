use crate::*;
use base64::{Engine,engine::general_purpose::{STANDARD,STANDARD_NO_PAD,URL_SAFE,URL_SAFE_NO_PAD}};
use std::{collections::HashSet,time::{SystemTime,UNIX_EPOCH}};
#[cfg(windows)] use winreg::{RegKey,enums::{HKEY_LOCAL_MACHINE,KEY_READ,KEY_WOW64_64KEY}};

#[derive(Clone,Serialize,Deserialize)] #[serde(rename_all="camelCase")]
pub struct Subscription {
    pub id:String,pub name:String,pub url:String,pub send_hwid:bool,pub auto_update:bool,pub interval_hours:u32,
    #[serde(default)] pub content:String,#[serde(default)]pub updated_at:u64,#[serde(default)]pub attempted_at:u64,#[serde(default)]pub error:Option<String>,
}
#[derive(Default)] pub struct Store {pub items:Vec<Subscription>,busy:HashSet<String>}
fn now()->u64{SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()}
pub fn load(root:&Path)->Store{Store{items:fs::read(root.join("subscriptions.json")).ok().and_then(|b|serde_json::from_slice(&b).ok()).unwrap_or_default(),busy:HashSet::new()}}
fn save(root:&Path,store:&Store)->Result<()>{
    let path=root.join("subscriptions.json");let temp=root.join("subscriptions.tmp");
    fs::write(&temp,serde_json::to_vec(&store.items).map_err(err)?).map_err(err)?;
    fs::rename(&temp,&path).map_err(err)?;Ok(())
}
#[tauri::command] pub fn get_hwid()->Result<String>{
    #[cfg(windows)] let key=RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags("SOFTWARE\\Microsoft\\Cryptography",KEY_READ|KEY_WOW64_64KEY).map_err(|_|"HWID_UNAVAILABLE")?;
    #[cfg(windows)] let guid:String=key.get_value("MachineGuid").map_err(|_|"HWID_UNAVAILABLE")?;
    #[cfg(target_os="macos")]
    let guid = {
        let output=Command::new("/usr/sbin/ioreg").args(["-rd1","-c","IOPlatformExpertDevice"]).output().map_err(err)?;
        String::from_utf8_lossy(&output.stdout).lines().find_map(|s|s.split_once("\"IOPlatformUUID\" = ")).map(|(_,s)|s.trim().trim_matches('"').to_owned()).ok_or("HWID_UNAVAILABLE")?
    };
    Ok(format!("{:x}",Sha256::digest(format!("Knotless:device:v1:{}",guid.to_lowercase()))))
}
fn validate_url(url:&str)->Result<reqwest::Url>{
    let url=reqwest::Url::parse(url).map_err(|_|"SUB_URL_INVALID")?;
    if url.scheme()!="https"||url.host_str().is_none()||!url.username().is_empty()||url.password().is_some()||url.fragment().is_some(){return Err("SUB_HTTPS_REQUIRED".into());}Ok(url)
}
pub fn decode_content(text:&str)->Result<String>{
    let text=text.trim().trim_start_matches('\u{feff}');
    let decoded=if text.contains("://"){text.to_string()}else{
        let compact:String=text.chars().filter(|c|!c.is_whitespace()).collect();
        let bytes=[&STANDARD,&STANDARD_NO_PAD,&URL_SAFE,&URL_SAFE_NO_PAD].iter().find_map(|e|e.decode(&compact).ok()).ok_or("SUB_FORMAT")?;
        String::from_utf8(bytes).map_err(|_|"SUB_FORMAT")?
    };
    let lines:Vec<&str>=decoded.lines().map(str::trim).filter(|s|!s.is_empty()&&!s.starts_with('#')).collect();
    if lines.len()>5000{return Err("SUB_TOO_LARGE".into());}
    if !lines.iter().any(|s|supported_link(s)){return Err("SUB_NO_VLESS".into());}
    Ok(lines.join("\n"))
}
fn supported_link(text:&str)->bool{
    let Ok(url)=reqwest::Url::parse(text)else{return false;};
    if url.scheme()!="vless"||url.host_str().is_none()||uuid::Uuid::parse_str(url.username()).is_err()||url.port()==Some(0){return false;}
    let params:std::collections::HashMap<_,_>=url.query_pairs().into_owned().collect();let get=|key:&str,default:&str|params.get(key).map(String::as_str).unwrap_or(default).to_owned();
    let transport=get("type","tcp");let security=get("security","none");
    if !["tcp","raw","ws","grpc","xhttp","httpupgrade"].contains(&transport.as_str())||!["none","tls","reality"].contains(&security.as_str()){return false;}
    if get("allowInsecure","")=="1"||get("insecure","")=="1"||get("encryption","none")!="none"{return false;}
    if security=="reality"&&(get("pbk","").is_empty()||get("sni","").is_empty()){return false;}
    if transport=="xhttp" {
        let extra = match params.get("extra") {
            Some(raw) => match serde_json::from_str::<serde_json::Map<String, Value>>(raw) {
                Ok(fields) if fields.keys().all(|k| matches!(k.as_str(),"mode"|"xPaddingBytes")) => fields,
                _ => return false,
            },
            None => Default::default(),
        };
        let mode=params.get("mode").map(String::as_str).filter(|s|!s.is_empty()).or_else(||extra.get("mode").and_then(Value::as_str)).unwrap_or("auto");
        if !matches!(mode,"auto"|"packet-up"|"stream-up"|"stream-one"){return false;}
        let padding=match extra.get("xPaddingBytes") {
            Some(Value::String(s))=>Some(s), None|Some(Value::Null)=>params.get("x_padding_bytes"), _=>return false,
        };
        if let Some(s)=padding {
            let parts:Vec<_>=s.split('-').collect();
            if parts.len()>2||parts.iter().any(|p|p.is_empty()||!p.bytes().all(|b|b.is_ascii_digit())){return false;}
        }
    }
    if ["tcp","raw"].contains(&transport.as_str())&&get("headerType","none")!="none"{return false;}true
}
async fn fetch(sub:&Subscription)->Result<String>{
    let url=validate_url(&sub.url)?;
    // Do not send a device identifier or secret subscription token across redirects.
    let client=reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(25)).user_agent("Knotless/0.3.0").build().map_err(err)?;
    let mut request=client.get(url).header("Accept","text/plain");
    if sub.send_hwid{request=request.header("x-hwid",get_hwid()?).header("x-device-os",if cfg!(windows){"Windows"}else{"macOS"}).header("x-device-model",if cfg!(windows){"Windows PC"}else{"Mac"});}
    let mut response=request.send().await.map_err(|_|"SUB_NETWORK")?;
    if response.headers().get("x-hwid-not-supported").is_some(){return Err("SUB_HWID_REQUIRED".into());}
    if response.status().is_redirection(){return Err("SUB_REDIRECT".into());}
    if !response.status().is_success(){return Err(format!("SUB_HTTP:{}",response.status().as_u16()));}
    let mut bytes=Vec::new();while let Some(chunk)=response.chunk().await.map_err(|_|"SUB_NETWORK")?{if bytes.len()+chunk.len()>4*1024*1024{return Err("SUB_TOO_LARGE".into());}bytes.extend_from_slice(&chunk);}
    decode_content(&String::from_utf8(bytes).map_err(|_|"SUB_FORMAT")?)
}
#[tauri::command] pub fn list_subscriptions(state:State<AppState>)->Result<Vec<Subscription>>{Ok(state.subscriptions.lock().map_err(err)?.items.clone())}
#[tauri::command] pub async fn add_subscription(name:String,url:String,send_hwid:bool,auto_update:bool,interval_hours:u32,app:tauri::AppHandle)->Result<Subscription>{
    validate_url(&url)?;if name.trim().is_empty()||name.len()>120||!(1..=168).contains(&interval_hours){return Err("SUB_SETTINGS".into());}
    let mut item=Subscription{id:uuid::Uuid::new_v4().to_string(),name:name.trim().into(),url,send_hwid,auto_update,interval_hours,content:String::new(),updated_at:0,attempted_at:0,error:None};
    item.content=fetch(&item).await?;item.updated_at=now();item.attempted_at=now();
    let state=app.state::<AppState>();let mut store=state.subscriptions.lock().map_err(err)?;
    if store.items.iter().any(|s|s.url==item.url){return Err("SUB_DUPLICATE".into());}
    let old=store.items.clone();store.items.push(item.clone());if let Err(e)=save(&state.root,&store){store.items=old;return Err(e);}
    drop(store);let _=app.emit("subscriptions-changed",());Ok(item)
}
#[tauri::command]pub fn configure_subscription(id:String,auto_update:bool,interval_hours:u32,send_hwid:bool,app:tauri::AppHandle)->Result<()>{
    if !(1..=168).contains(&interval_hours){return Err("SUB_SETTINGS".into());}
    let state=app.state::<AppState>();let mut store=state.subscriptions.lock().map_err(err)?;if store.busy.contains(&id){return Err("SUB_BUSY".into());}
    let old=store.items.clone();let item=store.items.iter_mut().find(|s|s.id==id).ok_or("SUB_NOT_FOUND")?;item.auto_update=auto_update;item.interval_hours=interval_hours;item.send_hwid=send_hwid;
    if let Err(e)=save(&state.root,&store){store.items=old;return Err(e);}drop(store);let _=app.emit("subscriptions-changed",());Ok(())
}
#[tauri::command]pub fn remove_subscription(id:String,app:tauri::AppHandle)->Result<()>{
    let state=app.state::<AppState>();let mut store=state.subscriptions.lock().map_err(err)?;if store.busy.contains(&id){return Err("SUB_BUSY".into());}
    let old=store.items.clone();store.items.retain(|s|s.id!=id);if let Err(e)=save(&state.root,&store){store.items=old;return Err(e);}drop(store);let _=app.emit("subscriptions-changed",());Ok(())
}
#[tauri::command]pub async fn refresh_subscription(id:String,app:tauri::AppHandle)->Result<()>{
    let state=app.state::<AppState>();
    let item={let mut store=state.subscriptions.lock().map_err(err)?;if store.busy.contains(&id){return Err("SUB_BUSY".into());}let item=store.items.iter().find(|s|s.id==id).cloned().ok_or("SUB_NOT_FOUND")?;store.busy.insert(id.clone());item};
    let result=fetch(&item).await;
    let mut store=state.subscriptions.lock().map_err(err)?;store.busy.remove(&id);let old=store.items.clone();
    if let Some(item)=store.items.iter_mut().find(|s|s.id==id){item.attempted_at=now();match &result{Ok(content)=>{item.content=content.clone();item.updated_at=now();item.error=None;},Err(e)=>item.error=Some(e.clone())}}
    if let Err(e)=save(&state.root,&store){store.items=old;return Err(e);}drop(store);let _=app.emit("subscriptions-changed",());result.map(|_|())
}
pub fn start_scheduler(app:tauri::AppHandle){tauri::async_runtime::spawn(async move{loop{
    tokio::time::sleep(Duration::from_secs(30)).await;let state=app.state::<AppState>();if state.closing.load(Ordering::SeqCst){break;}
    let due=state.subscriptions.lock().map(|s|s.items.iter().filter(|i|i.auto_update&&now().saturating_sub(i.attempted_at)>=u64::from(i.interval_hours)*3600&&!s.busy.contains(&i.id)).map(|i|i.id.clone()).collect::<Vec<_>>()).unwrap_or_default();
    for id in due{let _=refresh_subscription(id,app.clone()).await;}
}});}
#[cfg(test)]mod tests{use super::*;
 #[test]fn saves_and_replaces_subscription_store(){let root=std::env::temp_dir().join(format!("knotless-sub-test-{}",uuid::Uuid::new_v4()));fs::create_dir(&root).unwrap();let mut store=Store::default();save(&root,&store).unwrap();store.items.push(Subscription{id:"one".into(),name:"Test".into(),url:"https://example.com/sub".into(),send_hwid:true,auto_update:true,interval_hours:24,content:"test".into(),updated_at:0,attempted_at:0,error:None});save(&root,&store).unwrap();assert_eq!(load(&root).items[0].name,"Test");fs::remove_file(root.join("subscriptions.json")).unwrap();fs::remove_dir(root).unwrap();}
 #[test]fn plaintext_and_base64(){let input="vless://11111111-1111-4111-8111-111111111111@example.com:443#One\nvless://22222222-2222-4222-8222-222222222222@example.com:443#Two";assert_eq!(decode_content(input).unwrap(),input);assert_eq!(decode_content(&STANDARD.encode(input)).unwrap(),input);assert_eq!(decode_content(&URL_SAFE_NO_PAD.encode(input)).unwrap(),input);}
 #[test]fn invalid_refresh_cannot_replace_good_content(){for input in ["vless://invalid@example.com:443","vless://11111111-1111-4111-8111-111111111111@example.com?security=reality","vless://11111111-1111-4111-8111-111111111111@example.com?type=unsupported"]{assert_eq!(decode_content(input).unwrap_err(),"SUB_NO_VLESS");}}
 #[test]fn rejects_html_empty_and_unsafe_urls(){assert!(decode_content("<html>login</html>").is_err());assert!(decode_content("").is_err());assert!(validate_url("http://example.com/sub").is_err());assert!(validate_url("https://user:secret@example.com/sub").is_err());assert!(validate_url("https://example.com/sub#token").is_err());}
}
