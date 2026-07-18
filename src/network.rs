use std::io;
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Debug)]
enum ProxyForwardError {
    Connect(io::Error),
    Transfer(io::Error),
    WorkerPanic,
}

impl std::fmt::Display for ProxyForwardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(error) => write!(formatter, "upstream connect failed: {error}"),
            Self::Transfer(error) => write!(formatter, "stream transfer ended: {error}"),
            Self::WorkerPanic => formatter.write_str("proxy upload worker panicked"),
        }
    }
}

impl ProxyForwardError {
    fn should_report(&self) -> bool {
        match self {
            Self::Connect(_) | Self::WorkerPanic => true,
            Self::Transfer(error) => {
                !matches!(
                    error.kind(),
                    io::ErrorKind::BrokenPipe
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::NotConnected
                        | io::ErrorKind::UnexpectedEof
                ) && error.raw_os_error() != Some(libc::EADDRNOTAVAIL)
            }
        }
    }
}

const PROXY_ENV_KEYS: &[&str] = &[
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "ALL_PROXY",
    "https_proxy",
    "http_proxy",
    "all_proxy",
];

#[derive(Clone)]
struct ProxyTarget {
    scheme: String,
    credentials: Option<String>,
    target: SocketAddr,
}

#[derive(Clone, Copy)]
struct Ipv4Subnet {
    network: u32,
    mask: u32,
}

impl Ipv4Subnet {
    fn parse(value: &str) -> Option<Self> {
        let (address, prefix) = value.split_once('/')?;
        let address = address.parse::<Ipv4Addr>().ok()?;
        let prefix = prefix.parse::<u32>().ok()?;
        if prefix > 30 {
            return None;
        }
        let mask = if prefix == 0 {
            0
        } else {
            u32::MAX << (32 - prefix)
        };
        Some(Self {
            network: u32::from(address) & mask,
            mask,
        })
    }

    fn gateway(self) -> Ipv4Addr {
        Ipv4Addr::from(self.network + 1)
    }

    fn contains(self, address: Ipv4Addr) -> bool {
        u32::from(address) & self.mask == self.network
    }
}

pub struct HostProxyBridge {
    url: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl HostProxyBridge {
    pub fn start(container: &Path, child_path: &str) -> Result<Option<Self>, String> {
        let Some(proxy) = loopback_proxy_from_environment() else {
            return Ok(None);
        };
        let subnet = apple_default_subnet(container, child_path)?;
        let listener = bind_apple_listener(subnet.gateway())?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let port = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        let credentials = proxy
            .credentials
            .as_deref()
            .map(|value| format!("{}@", value))
            .unwrap_or_default();
        let url = format!(
            "{}://{}{}:{}",
            proxy.scheme,
            credentials,
            subnet.gateway(),
            port
        );
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let target = proxy.target;
        let worker = thread::Builder::new()
            .name("cocoa-way-proxy".into())
            .spawn(move || accept_connections(listener, subnet, target, thread_stop))
            .map_err(|error| error.to_string())?;
        Ok(Some(Self {
            url,
            stop,
            worker: Some(worker),
        }))
    }

    pub fn container_environment(&self) -> Vec<String> {
        ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"]
            .into_iter()
            .map(|key| format!("{}={}", key, self.url))
            .collect()
    }
}

fn bind_apple_listener(gateway: Ipv4Addr) -> Result<TcpListener, String> {
    let mut last_error = None;
    for _ in 0..10 {
        match TcpListener::bind((gateway, 0)) {
            Ok(listener) => return Ok(listener),
            Err(error) => last_error = Some(error),
        }
        thread::sleep(Duration::from_millis(50));
    }
    TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|fallback| {
        format!(
            "could not bind the Apple Container proxy bridge on {} ({}) or a source-filtered fallback ({})",
            gateway,
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "unknown error".into()),
            fallback
        )
    })
}

impl Drop for HostProxyBridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub fn has_proxy_environment(environment: &[String]) -> bool {
    environment.iter().any(|entry| {
        entry
            .split_once('=')
            .map(|(key, value)| PROXY_ENV_KEYS.contains(&key) && !value.trim().is_empty())
            .unwrap_or(false)
    })
}

fn loopback_proxy_from_environment() -> Option<ProxyTarget> {
    PROXY_ENV_KEYS.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .and_then(|value| parse_loopback_proxy(&value))
    })
}

fn parse_loopback_proxy(value: &str) -> Option<ProxyTarget> {
    let (scheme, remainder) = value.trim().split_once("://")?;
    let authority = remainder.split('/').next()?;
    let (credentials, host_port) = match authority.rsplit_once('@') {
        Some((credentials, host_port)) => (Some(credentials.to_string()), host_port),
        None => (None, authority),
    };
    let (host, port) = if let Some(host) = host_port.strip_prefix('[') {
        let (host, port) = host.split_once("]:")?;
        (host, port)
    } else {
        host_port.rsplit_once(':')?
    };
    let port = port.parse::<u16>().ok()?;
    let target = (host, port)
        .to_socket_addrs()
        .ok()?
        .find(|address| address.ip().is_loopback())?;
    Some(ProxyTarget {
        scheme: scheme.to_string(),
        credentials,
        target,
    })
}

fn apple_default_subnet(container: &Path, child_path: &str) -> Result<Ipv4Subnet, String> {
    let output = Command::new(container)
        .env("PATH", child_path)
        .args(["network", "list"])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "`container network list` exited with {}",
            output.status
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter(|line| line.split_whitespace().next() == Some("default"))
        .flat_map(str::split_whitespace)
        .find_map(Ipv4Subnet::parse)
        .ok_or_else(|| "Apple Container's default IPv4 subnet was not found".into())
}

fn accept_connections(
    listener: TcpListener,
    subnet: Ipv4Subnet,
    target: SocketAddr,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((client, peer)) => {
                let allowed = matches!(peer.ip(), IpAddr::V4(address) if subnet.contains(address));
                if !allowed {
                    let _ = client.shutdown(Shutdown::Both);
                    continue;
                }
                if let Err(error) = client.set_nonblocking(false) {
                    eprintln!("cocoa-way proxy bridge stream setup failed: {}", error);
                    continue;
                }
                let _ = thread::Builder::new()
                    .name("cocoa-way-proxy-stream".into())
                    .spawn(move || {
                        if let Err(error) = forward_connection(client, target)
                            && error.should_report()
                        {
                            eprintln!("cocoa-way proxy bridge stream failed: {}", error);
                        }
                    });
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                eprintln!("cocoa-way proxy bridge stopped: {}", error);
                break;
            }
        }
    }
}

fn forward_connection(mut client: TcpStream, target: SocketAddr) -> Result<(), ProxyForwardError> {
    let mut upstream = TcpStream::connect_timeout(&target, Duration::from_secs(3))
        .map_err(ProxyForwardError::Connect)?;
    let mut client_reader = client.try_clone().map_err(ProxyForwardError::Transfer)?;
    let mut upstream_writer = upstream.try_clone().map_err(ProxyForwardError::Transfer)?;
    let upload = thread::spawn(move || {
        let result = io::copy(&mut client_reader, &mut upstream_writer);
        let _ = upstream_writer.shutdown(Shutdown::Write);
        result
    });
    let download = io::copy(&mut upstream, &mut client);
    let _ = client.shutdown(Shutdown::Write);
    let upload = upload.join().map_err(|_| ProxyForwardError::WorkerPanic)?;
    upload.map_err(ProxyForwardError::Transfer)?;
    download.map_err(ProxyForwardError::Transfer)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_loopback_http_proxy() {
        let proxy = parse_loopback_proxy("http://127.0.0.1:7897").unwrap();
        assert_eq!(proxy.scheme, "http");
        assert_eq!(proxy.target, "127.0.0.1:7897".parse().unwrap());
    }

    #[test]
    fn rejects_non_loopback_proxy() {
        assert!(parse_loopback_proxy("http://192.168.1.2:7897").is_none());
    }

    #[test]
    fn computes_gateway_and_source_range() {
        let subnet = Ipv4Subnet::parse("192.168.64.0/24").unwrap();
        assert_eq!(subnet.gateway(), Ipv4Addr::new(192, 168, 64, 1));
        assert!(subnet.contains(Ipv4Addr::new(192, 168, 64, 203)));
        assert!(!subnet.contains(Ipv4Addr::new(192, 168, 1, 87)));
    }

    #[test]
    fn normal_stream_teardown_is_not_reported_as_a_proxy_failure() {
        for kind in [
            io::ErrorKind::BrokenPipe,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::NotConnected,
            io::ErrorKind::UnexpectedEof,
        ] {
            assert!(!ProxyForwardError::Transfer(io::Error::from(kind)).should_report());
        }
        assert!(
            ProxyForwardError::Connect(io::Error::from(io::ErrorKind::ConnectionRefused))
                .should_report()
        );
    }
}
