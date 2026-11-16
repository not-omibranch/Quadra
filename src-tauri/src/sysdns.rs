//! A tiny DNS server on 127.0.0.1 that answers with the system's own lookups (getaddrinfo).
//!
//! Android has no /etc/resolv.conf, and that file is all sing-box's "local" DNS server reads
//! when it runs as a plain program (the official app gets the system's resolver through its
//! Java side instead). Left alone, every name sing-box had to resolve itself (what goes
//! direct: the Russia preset, exclusions by domain, names a provider config resolves locally)
//! would be asked of 127.0.0.1:53, where nobody answers. So on Android the "local" servers of
//! a sing-box config are pointed here, and this asks Android's resolver: the current network,
//! Private DNS and all. The app is outside its own VPN, so the answers come from the real
//! network, as they should for traffic that does not go through the server.

use hickory_proto::op::{Message, ResponseCode};
use hickory_proto::rr::rdata::{A, AAAA};
use hickory_proto::rr::{RData, Record, RecordType};
use serde_json::{json, Value};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;

/// How long the answers may be kept by whoever asked.
const TTL: u32 = 60;

static PORT: tokio::sync::OnceCell<u16> = tokio::sync::OnceCell::const_new();

/// The port the server listens on; it is started on first use and then runs for as long as
/// the app does.
pub async fn port() -> Result<u16, String> {
    PORT.get_or_try_init(|| async {
        let sock = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.map_err(|e| format!("системный DNS: {e}"))?;
        let port = sock.local_addr().map_err(|e| format!("системный DNS: {e}"))?.port();
        tokio::spawn(serve(Arc::new(sock)));
        Ok(port)
    })
    .await
    .copied()
}

/// Points every "local" DNS server of a sing-box config at this server.
pub fn redirect_local(cfg: &mut Value, port: u16) -> usize {
    let mut n = 0;
    for s in cfg["dns"]["servers"].as_array_mut().into_iter().flatten() {
        if s["type"] == "local" {
            *s = json!({"type": "udp", "tag": s["tag"].clone(), "server": "127.0.0.1", "server_port": port});
            n += 1;
        }
    }
    n
}

async fn serve(sock: Arc<UdpSocket>) {
    let mut buf = vec![0u8; 4096];
    loop {
        let (n, from) = match sock.recv_from(&mut buf).await {
            Ok(r) => r,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let query = buf[..n].to_vec();
        let sock = sock.clone();
        tokio::spawn(async move {
            if let Some(reply) = answer(&query).await {
                let _ = sock.send_to(&reply, from).await;
            }
        });
    }
}

/// The reply to one DNS message: A and AAAA questions are looked up by the system, anything
/// else gets an empty answer.
pub async fn answer(query: &[u8]) -> Option<Vec<u8>> {
    let request = Message::from_vec(query).ok()?;
    let mut reply = Message::response(request.metadata.id, request.metadata.op_code);
    reply.metadata.recursion_desired = request.metadata.recursion_desired;
    reply.metadata.recursion_available = true;
    let Some(q) = request.queries.first().cloned() else {
        reply.metadata.response_code = ResponseCode::FormErr;
        return reply.to_vec().ok();
    };
    reply.add_query(q.clone());
    let wanted = q.query_type();
    if matches!(wanted, RecordType::A | RecordType::AAAA) {
        let host = q.name().to_ascii();
        let host = host.trim_end_matches('.');
        let found = tokio::time::timeout(Duration::from_secs(8), tokio::net::lookup_host((host, 0))).await;
        match found {
            Ok(Ok(addrs)) => {
                let mut seen: Vec<IpAddr> = vec![];
                for ip in addrs.map(|a| a.ip()) {
                    if seen.contains(&ip) {
                        continue;
                    }
                    seen.push(ip);
                    let data = match (ip, wanted) {
                        (IpAddr::V4(v4), RecordType::A) => RData::A(A(v4)),
                        (IpAddr::V6(v6), RecordType::AAAA) => RData::AAAA(AAAA(v6)),
                        _ => continue,
                    };
                    reply.add_answer(Record::from_rdata(q.name().clone(), TTL, data));
                }
            }
            _ => reply.metadata.response_code = ResponseCode::ServFail,
        }
    }
    reply.to_vec().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::op::Query;
    use hickory_proto::rr::Name;

    fn ask(name: &str, kind: RecordType) -> Vec<u8> {
        let mut m = Message::query();
        m.metadata.recursion_desired = true;
        m.add_query(Query::query(Name::from_ascii(name).unwrap(), kind));
        m.to_vec().unwrap()
    }

    #[tokio::test]
    async fn the_system_answers_through_the_server() {
        let port = port().await.unwrap();
        let sock = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let query = ask("localhost.", RecordType::A);
        sock.send_to(&query, (Ipv4Addr::LOCALHOST, port)).await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = tokio::time::timeout(Duration::from_secs(10), sock.recv(&mut buf)).await.unwrap().unwrap();
        let reply = Message::from_vec(&buf[..n]).unwrap();
        let sent = Message::from_vec(&query).unwrap();
        assert_eq!(reply.metadata.id, sent.metadata.id);
        assert_eq!(reply.metadata.response_code, ResponseCode::NoError);
        assert_eq!(reply.queries.len(), 1);
        assert!(reply.answers.iter().any(|r| matches!(r.data, RData::A(A(ip)) if ip == Ipv4Addr::LOCALHOST)), "{reply:?}");
    }

    #[tokio::test]
    async fn other_questions_get_an_empty_answer() {
        let reply = Message::from_vec(&answer(&ask("localhost.", RecordType::MX)).await.unwrap()).unwrap();
        assert_eq!(reply.metadata.response_code, ResponseCode::NoError);
        assert!(reply.answers.is_empty());
        assert!(answer(b"\x00\x01").await.is_none());
    }

    #[test]
    fn only_local_servers_are_redirected() {
        let mut cfg = json!({"dns": {"servers": [
            {"type": "udp", "tag": "dns-0", "server": "1.1.1.1", "detour": "proxy"},
            {"type": "local", "tag": "dns-local"}]}});
        assert_eq!(redirect_local(&mut cfg, 5353), 1);
        assert_eq!(cfg["dns"]["servers"][0]["server"], "1.1.1.1");
        assert_eq!(cfg["dns"]["servers"][1], json!({"type": "udp", "tag": "dns-local", "server": "127.0.0.1", "server_port": 5353}));
    }
}
