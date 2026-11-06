use serde_json::{json,Value};
use std::{fs,path::Path};
use crate::{err,Result};

pub const MIN_VERSION: &str="v26.10.31";
pub fn supported(version:&str)->bool {
    let parts:Option<Vec<u32>>=version.trim_start_matches('v').split('.').map(|s|s.parse().ok()).collect();
    parts.is_some_and(|p|p.len()==3&&p.as_slice()>=[26,10,31].as_slice())
}
#[cfg(test)] pub fn add_inbound(config:&mut Value) -> Result<()> { add_with_dns(config, &crate::dns::Settings::default()) }
pub fn add_with_dns(config:&mut Value,dns:&crate::dns::Settings) -> Result<()> {
    dns.validate()?;
    config["log"]=json!({"loglevel":"info"});
    config["inbounds"].as_array_mut().ok_or("Нет списка inbound.")?.push(json!({
        "tag":"knotless-tun", "protocol":"tun",
        "settings":{ "name":"Knotless", "desc":"Knotless", "mtu":1500,
            "gateway":["172.31.255.1/30","fdfe:dcba:9876::1/126"],
            // A private virtual resolver keeps automatic browser DoH upgrades
            // from bypassing the selected DNS policy (including AAAA filtering).
            "dns":["172.31.255.2"],
            "autoSystemRoutingTable":["0.0.0.0/1","128.0.0.0/1","::/1","8000::/1"],
            "autoOutboundsInterface":"auto", "autoSystemWfpBlockLeak":if dns.leak_protection{vec!["dns","misconfigtun"]}else{vec!["misconfigtun"]} },
        "sniffing":{"enabled":true,"destOverride":["http","tls","quic"],"routeOnly":true}
    }));
    crate::dns::configure(config,dns)?;
    Ok(())
}
pub fn ensure_driver(resource:&Path,exe:&Path)->Result<()> {
    let source=resource.join("resources").join("wintun.dll");
    let target=exe.parent().ok_or("Нет каталога ядра.")?.join("wintun.dll");
    if !source.is_file() { return Err("В сборке отсутствует wintun.dll. Переустановите Knotless.".into()); }
    let data=fs::read(&source).map_err(err)?;
    if fs::read(&target).ok().as_deref()!=Some(data.as_slice()) { fs::write(target,data).map_err(err)?; }
    Ok(())
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn version_gate(){assert!(supported("v26.10.31"));assert!(supported("v26.11.1"));assert!(!supported("v26.9.9"));assert!(!supported("v1.8.24"));}
    #[test] fn dns_protection_requires_explicit_opt_out(){
        let legacy:crate::dns::Settings=serde_json::from_value(json!({"transport":"tcp","address":"9.9.9.9","dohUrl":"https://dns.quad9.net/dns-query"})).unwrap();assert!(legacy.leak_protection);
        let mut protected=json!({"inbounds":[],"outbounds":[],"routing":{"rules":[]}});add_with_dns(&mut protected,&legacy).unwrap();
        assert_eq!(protected["inbounds"][0]["settings"]["autoSystemWfpBlockLeak"],json!(["dns","misconfigtun"]));
        let mut compatible=json!({"inbounds":[],"outbounds":[],"routing":{"rules":[]}});add_with_dns(&mut compatible,&crate::dns::Settings{leak_protection:false,..legacy}).unwrap();
        assert_eq!(compatible["inbounds"][0]["settings"]["autoSystemWfpBlockLeak"],json!(["misconfigtun"]));
        compatible["inbounds"][0]["settings"]["autoSystemWfpBlockLeak"]=json!(["dns","misconfigtun"]);assert_eq!(protected,compatible,"DNS forwarding, routes and outbound policy must remain unchanged");
    }
    #[test] fn covers_both_ip_families_and_preserves_user_rules(){
        let rule=json!({"type":"field","ip":["geoip:private"],"outboundTag":"direct"});
        let mut c=json!({"inbounds":[],"outbounds":[],"routing":{"rules":[rule.clone()]}}); add_inbound(&mut c).unwrap();
        assert_eq!(c["inbounds"][0]["settings"]["autoSystemRoutingTable"],json!(["0.0.0.0/1","128.0.0.0/1","::/1","8000::/1"]));
        assert_eq!(c["inbounds"][0]["settings"]["autoOutboundsInterface"],"auto");
        assert_eq!(c["inbounds"][0]["settings"]["dns"],json!(["172.31.255.2"]));
        assert_eq!(c["routing"]["rules"][0]["port"],"53");assert_eq!(c["routing"]["rules"][1],rule);
        assert_eq!(c["outbounds"][0]["settings"]["rewriteNetwork"],"tcp");assert_eq!(c["outbounds"][0]["streamSettings"]["sockopt"]["dialerProxy"],"proxy");
    }
}
