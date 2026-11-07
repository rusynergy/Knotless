use crate::*;
use std::{ffi::CString, os::fd::AsRawFd};

pub async fn probe(host: String, port: u16) -> Result<f64> {
    if host.is_empty() || port == 0 { return Err("TCP_ADDRESS_INVALID".into()); }
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut addresses=tokio::net::lookup_host((host.as_str(),port)).await.map_err(|_|"TCP_DNS_FAILED")?.collect::<Vec<_>>();
        addresses.sort_by_key(|a|a.is_ipv6());
        for address in addresses {
            let v4=address.is_ipv4();
            let socket=if v4 {tokio::net::TcpSocket::new_v4()} else {tokio::net::TcpSocket::new_v6()}.map_err(err)?;
            if !address.ip().is_loopback() {
                let route=Command::new("/sbin/route").args(["-n","get",if v4{"-inet"}else{"-inet6"},"default"]).output().map_err(err)?;
                let output=String::from_utf8_lossy(&route.stdout);
                let Some(name)=output.lines().find_map(|l|l.trim().strip_prefix("interface: ")).map(str::trim) else {continue;};
                if name.starts_with("utun"){continue;}
                let name=CString::new(name).map_err(err)?;
                let index=unsafe{libc::if_nametoindex(name.as_ptr())};
                let (level,opt)=if v4{(libc::IPPROTO_IP,libc::IP_BOUND_IF)}else{(libc::IPPROTO_IPV6,libc::IPV6_BOUND_IF)};
                if index==0 || unsafe{libc::setsockopt(socket.as_raw_fd(),level,opt,(&index as *const u32).cast(),4)}!=0 {continue;}
            }
            let start=Instant::now();
            if matches!(tokio::time::timeout(Duration::from_millis(1500),socket.connect(address)).await,Ok(Ok(_))){
                return Ok((start.elapsed().as_secs_f64()*10000.0).round()/10.0);
            }
        }
        Err("TCP_CONNECT_FAILED".into())
    }).await.map_err(|_|"TCP_TIMEOUT")?
}
