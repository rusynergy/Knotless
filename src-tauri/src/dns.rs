use crate::*;
#[derive(Clone,Serialize,Deserialize,Debug)]
#[serde(rename_all="camelCase")]
pub struct Settings {pub transport:String,pub address:String,pub doh_url:String,#[serde(default)]pub ipv6:bool}
impl Default for Settings {fn default()->Self{Self{transport:"tcp".into(),address:"9.9.9.9".into(),doh_url:"https://dns.quad9.net/dns-query".into(),ipv6:false}}}
impl Settings {
    pub fn validate(&self)->Result<()>{
        if !["tcp","doh"].contains(&self.transport.as_str()){return Err("DNS_INVALID".into());}
        let ip=self.address.parse::<std::net::IpAddr>().map_err(|_|"DNS_INVALID")?;
        if ip.is_unspecified()||ip.is_multicast()||ip.is_loopback(){return Err("DNS_INVALID".into());}
        if self.transport=="doh"{let url=reqwest::Url::parse(&self.doh_url).map_err(|_|"DNS_INVALID")?;if url.scheme()!="https"||url.host_str().is_none()||!url.username().is_empty()||url.password().is_some()||url.fragment().is_some(){return Err("DNS_INVALID".into());}}
        Ok(())
    }
}
pub fn configure(config:&mut Value,settings:&Settings)->Result<()>{
    settings.validate()?;
    let mut rules=if settings.ipv6{vec![]}else{vec![json!({"qType":"28,65","action":"return","rCode":0})]};
    let outbound=if settings.transport=="doh"{
        config["dns"]=json!({"servers":[settings.doh_url],"tag":"knotless-dns-upstream","queryStrategy":if settings.ipv6{"UseIP"}else{"UseIPv4"},"disableFallback":true});
        config["routing"]["rules"].as_array_mut().ok_or("No routing rules")?.insert(0,json!({"type":"field","inboundTag":["knotless-dns-upstream"],"outboundTag":"proxy"}));
        // Default DNS-outbound rules hijack A/AAAA and return empty answers for
        // other types. No plaintext fallback when encrypted DNS is selected.
        json!({"tag":"knotless-dns","protocol":"dns","settings":{"rules":rules}})
    }else{rules.push(json!({"action":"direct"}));json!({"tag":"knotless-dns","protocol":"dns","settings":{"rewriteNetwork":"tcp","rewriteAddress":settings.address,"rewritePort":53,"rules":rules},"streamSettings":{"sockopt":{"dialerProxy":"proxy"}}})};
    config["outbounds"].as_array_mut().ok_or("No outbounds")?.push(outbound);
    config["routing"]["rules"].as_array_mut().ok_or("No routing rules")?.insert(0,json!({"type":"field","inboundTag":["knotless-tun"],"port":"53","outboundTag":"knotless-dns"}));
    Ok(())
}
// Resolve the VPN endpoint BEFORE TUN replaces system DNS. Otherwise connecting
// to a domain-named VPN server can depend on that same not-yet-connected tunnel.
pub fn pin_endpoint(config:&mut Value)->Result<()>{
    use std::net::ToSocketAddrs;
    let outbound=config["outbounds"].as_array_mut().and_then(|v|v.iter_mut().find(|v|v["tag"]=="proxy")).ok_or("No proxy")?;
    let host=outbound["settings"]["vnext"][0]["address"].as_str().ok_or("No VLESS endpoint")?.to_owned();
    if host.parse::<std::net::IpAddr>().is_ok(){return Ok(());}
    let port=outbound["settings"]["vnext"][0]["port"].as_u64().ok_or("No port")? as u16;
    let mut addresses=(host.as_str(),port).to_socket_addrs().map_err(|_|"VPN_ENDPOINT_DNS_FAILED")?.collect::<Vec<_>>();addresses.sort_by_key(|a|a.is_ipv6());
    let ip=addresses.first().ok_or("VPN_ENDPOINT_DNS_FAILED")?.ip();
    outbound["settings"]["vnext"][0]["address"]=json!(ip.to_string());
    let stream=&mut outbound["streamSettings"];
    if stream["security"]=="tls"&&stream["tlsSettings"]["serverName"].as_str().unwrap_or("").is_empty(){stream["tlsSettings"]["serverName"]=json!(host);}
    if stream["network"]=="ws"&&stream["wsSettings"]["headers"]["Host"].is_null(){stream["wsSettings"]["headers"]["Host"]=json!(host);}
    if stream["network"]=="xhttp"&&stream["xhttpSettings"]["host"].as_str().unwrap_or("").is_empty(){stream["xhttpSettings"]["host"]=json!(host);}
    Ok(())
}
#[cfg(test)]mod tests{use super::*;
 #[test]fn pins_endpoint_preserving_tls_and_websocket_host(){let mut config=json!({"outbounds":[{"tag":"proxy","settings":{"vnext":[{"address":"localhost","port":443}]},"streamSettings":{"network":"ws","security":"tls"}}]});pin_endpoint(&mut config).unwrap();let outbound=&config["outbounds"][0];assert!(outbound["settings"]["vnext"][0]["address"].as_str().unwrap().parse::<std::net::IpAddr>().is_ok());assert_eq!(outbound["streamSettings"]["tlsSettings"]["serverName"],"localhost");assert_eq!(outbound["streamSettings"]["wsSettings"]["headers"]["Host"],"localhost");}
 #[test] #[ignore="Real DNS and DoH requests through a local VLESS server"]
 fn real_dns_providers(){
  let root=PathBuf::from(std::env::var("KNOTLESS_TEST_ROOT").unwrap());let exe=core_path(&root,"v26.10.31").unwrap();
  let listener=TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();drop(listener);
  let id="11111111-1111-4111-8111-111111111111";
  let server_config=json!({"inbounds":[{"listen":"127.0.0.1","port":port,"protocol":"vless","settings":{"clients":[{"id":id}],"decryption":"none"}}],"outbounds":[{"protocol":"freedom"}]});
  let mut server=spawn_managed(&exe,&server_config,&root).unwrap();wait_ready(&mut server,port).unwrap();
  let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
  rt.block_on(async {
   let mut failures=vec![];
   for (name,address,url) in [("Quad9","9.9.9.9","https://dns.quad9.net/dns-query"),("Google","8.8.8.8","https://dns.google/dns-query"),("Cloudflare","1.1.1.1","https://1.1.1.1/dns-query"),("OpenDNS","208.67.222.222","https://doh.opendns.com/dns-query"),("Yandex","77.88.8.8","https://common.dot.dns.yandex.net/dns-query")]{
    for transport in ["tcp","doh"]{
     let settings=Settings{transport:transport.into(),address:address.into(),doh_url:url.into(),ipv6:false};
     let tcp=TcpListener::bind("127.0.0.1:0").unwrap();let socks_port=tcp.local_addr().unwrap().port();
     let udp=std::net::UdpSocket::bind("127.0.0.1:0").unwrap();let dns_port=udp.local_addr().unwrap().port();
     let mut config=prepare_config(json!({"routing":{"rules":[]},"outbounds":[{"tag":"proxy","protocol":"vless","settings":{"vnext":[{"address":"127.0.0.1","port":port,"users":[{"id":id,"encryption":"none"}]}]}}]}),socks_port,None).unwrap();
     tun::add_with_dns(&mut config,&settings).unwrap();config["inbounds"].as_array_mut().unwrap().retain(|v|v["protocol"]!="tun");
     config["inbounds"].as_array_mut().unwrap().push(json!({"tag":"knotless-tun","listen":"127.0.0.1","port":dns_port,"protocol":"dokodemo-door","settings":{"address":"192.168.1.1","port":53,"network":"udp"}}));
     preflight(&exe,&config,&root).unwrap();drop(tcp);drop(udp);let mut child=spawn_managed(&exe,&config,&root).unwrap();wait_ready(&mut child,socks_port).unwrap();
     let socket=tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();let packet=b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x07example\x03com\x00\x00\x01\x00\x01";
     socket.send_to(packet,("127.0.0.1",dns_port)).await.unwrap();let mut answer=[0u8;4096];
     let result=tokio::time::timeout(Duration::from_secs(12),socket.recv_from(&mut answer)).await;
     let ok=matches!(result,Ok(Ok((n,_))) if n>12)&&answer[0..2]==packet[0..2]&&answer[3]&15==0&&u16::from_be_bytes([answer[6],answer[7]])>0;
     println!("{name} {transport}: {}",if ok{"DNS answer received"}else{"FAILED"});if !ok{failures.push(format!("{name} {transport}"));}
     for qtype in [28u8,65u8]{let mut query=packet.to_vec();let len=query.len();query[len-3]=qtype;socket.send_to(&query,("127.0.0.1",dns_port)).await.unwrap();let(n,_)=tokio::time::timeout(Duration::from_secs(3),socket.recv_from(&mut answer)).await.unwrap().unwrap();assert!(n>=12);assert_eq!(answer[3]&15,0);assert_eq!(u16::from_be_bytes([answer[6],answer[7]]),0,"IPv6 and HTTPS hints must be suppressed in compatibility mode");}
    }
   }assert!(failures.is_empty(),"Failed providers: {failures:?}");
  });
 }
 #[test]fn validates_dns(){assert!(Settings::default().validate().is_ok());for address in ["hostname","127.0.0.1","0.0.0.0","224.0.0.1"]{assert!(Settings{address:address.into(),..Default::default()}.validate().is_err());}for url in ["http://example.com/dns-query","https://u:p@example.com/dns-query","https://example.com/#x"]{assert!(Settings{transport:"doh".into(),doh_url:url.into(),..Default::default()}.validate().is_err());}}
 #[test]fn doh_cannot_fall_back_to_plaintext(){let mut config=json!({"outbounds":[],"routing":{"rules":[]}});configure(&mut config,&Settings{transport:"doh".into(),..Default::default()}).unwrap();assert_eq!(config["dns"]["servers"],json!(["https://dns.quad9.net/dns-query"]));assert_eq!(config["routing"]["rules"][1]["outboundTag"],"proxy");assert!(config["outbounds"][0]["settings"]["rewriteAddress"].is_null());}
}
