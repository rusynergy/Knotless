//! TCP connect latency must not measure the local TUN stack's SYN-ACK.
use crate::*;
use std::{net::SocketAddr,os::windows::io::AsRawSocket};
use windows_sys::Win32::{NetworkManagement::IpHelper::*,Networking::WinSock::*};

fn default_interface(family:ADDRESS_FAMILY)->Result<u32>{
    unsafe {
        let mut table=std::ptr::null_mut();
        let code=GetIpForwardTable2(family,&mut table);
        if code!=0{return Err("TCP_INTERFACE_UNAVAILABLE".into());}
        struct Table(*mut MIB_IPFORWARD_TABLE2);
        impl Drop for Table{fn drop(&mut self){unsafe{FreeMibTable(self.0.cast());}}}
        let table=Table(table);
        let rows=std::slice::from_raw_parts((*table.0).Table.as_ptr(),(*table.0).NumEntries as usize);
        let mut candidates=Vec::new();
        for route in rows {
            // Knotless installs /1 routes; the original /0 still identifies
            // the uplink. Never fall back to an unbound socket if it is absent.
            if route.DestinationPrefix.PrefixLength!=0||route.Loopback!=0||route.ValidLifetime==0{continue;}
            let mut interface:MIB_IPINTERFACE_ROW=std::mem::zeroed();interface.Family=family;interface.InterfaceIndex=route.InterfaceIndex;
            if GetIpInterfaceEntry(&mut interface)!=0||interface.Connected==0{continue;}
            // Windows interface binding can re-enter TUN with weak-host send
            // or forwarding enabled. Refuse misleading measurements in that case.
            if interface.WeakHostSend!=0||interface.ForwardingEnabled!=0{continue;}
            candidates.push((u64::from(route.Metric)+u64::from(interface.Metric),route.InterfaceIndex));
        }
        candidates.sort_unstable();candidates.first().map(|c|c.1).ok_or_else(||"TCP_INTERFACE_UNAVAILABLE".into())
    }
}
async fn connect_direct(address:SocketAddr)->Result<f64>{
    let socket=if address.is_ipv4(){tokio::net::TcpSocket::new_v4()}else{tokio::net::TcpSocket::new_v6()}.map_err(err)?;
    if !address.ip().is_loopback(){
        let index=default_interface(if address.is_ipv4(){AF_INET}else{AF_INET6})?;
        let (level,option,value)=if address.is_ipv4(){(IPPROTO_IP,IP_UNICAST_IF,index.to_be())}else{(IPPROTO_IPV6,IPV6_UNICAST_IF,index)};
        let code=unsafe{setsockopt(socket.as_raw_socket() as _,level,option,(&value as *const u32).cast(),4)};
        if code==SOCKET_ERROR{return Err("TCP_INTERFACE_UNAVAILABLE".into());}
    }
    let start=Instant::now();
    let stream=tokio::time::timeout(Duration::from_millis(1500),socket.connect(address)).await.map_err(|_|"TCP_TIMEOUT")?.map_err(|_|"TCP_CONNECT_FAILED")?;
    let ms=(start.elapsed().as_secs_f64()*10_000.0).round()/10.0;drop(stream);Ok(ms)
}
pub async fn probe(host:String,port:u16)->Result<f64>{
    if host.is_empty()||port==0{return Err("TCP_ADDRESS_INVALID".into());}
    tokio::time::timeout(Duration::from_secs(5),async{
        // DNS and failed address attempts are not part of TCP connect latency.
        let mut addresses=tokio::net::lookup_host((host.as_str(),port)).await.map_err(|_|"TCP_DNS_FAILED")?.collect::<Vec<_>>();
        addresses.sort_by_key(|a|a.is_ipv6());addresses.dedup();let mut last="TCP_CONNECT_FAILED".to_string();
        for address in addresses.into_iter().take(4){match connect_direct(address).await{Ok(ms)=>return Ok(ms),Err(e)=>last=e,}}
        Err(last)
    }).await.map_err(|_|"TCP_TIMEOUT")?
}
#[cfg(test)]mod tests{use super::*;
 #[test]fn accepts_listener_and_rejects_closed_port(){let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();rt.block_on(async{let listener=TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();assert!(probe("127.0.0.1".into(),port).await.is_ok());drop(listener);assert!(probe("127.0.0.1".into(),port).await.is_err());assert!(probe("".into(),443).await.is_err());});}
 #[test]#[ignore="Real direct TCP probes, optionally while Knotless TUN is active"]
 fn direct_network(){let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();rt.block_on(async{let host=std::env::var("KNOTLESS_TCP_HOST").unwrap_or("1.1.1.1".into());let port=std::env::var("KNOTLESS_TCP_PORT").ok().and_then(|p|p.parse().ok()).unwrap_or(443);let start=Instant::now();let ordinary=tokio::time::timeout(Duration::from_secs(5),tokio::net::TcpStream::connect((host.as_str(),port))).await;println!("Unbound connect accepted: {}, elapsed: {:.1} ms",matches!(ordinary,Ok(Ok(_))),start.elapsed().as_secs_f64()*1000.0);let ms=probe(host,port).await.unwrap();println!("Direct TCP to test endpoint: {ms} ms");let unbound=tokio::time::timeout(Duration::from_secs(2),tokio::net::TcpStream::connect("203.0.113.1:9")).await;println!("Unbound TEST-NET connect accepted: {}",matches!(unbound,Ok(Ok(_))));assert!(probe("203.0.113.1".into(),9).await.is_err(),"unreachable endpoint must not report local TUN latency");});}
}
