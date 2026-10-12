//! Runtime configuration for the four mock servers.
//!
//! Historically every address was a hardcoded `127.0.0.1` constant, which meant
//! the mock could only be reached over loopback. To host it on a real machine we
//! need two independent notions per server:
//!
//! * **bind** — the local interface/port the server listens on. It defaults to
//!   `127.0.0.1`; set it to `0.0.0.0` (all interfaces) for a remote-reachable
//!   deployment.
//! * **public host** — the address handed to the *client* so it knows where to
//!   connect next (login → browser → lobby → match). This must be a host the
//!   client can actually route to (e.g. the VPS's public IP/DNS name), which is
//!   generally **not** the bind address.
//!
//! Everything is read from the environment once, lazily, via [`get`]. Defaults
//! reproduce the old localhost behaviour, so the local dev workflow is unchanged
//! when no env vars are set.
//!
//! ## Environment variables
//!
//! * `SRP_BIND_HOST` / `SRP_PUBLIC_HOST` — bind/public hosts applied to **all**
//!   servers (the common case: one machine hosting everything). Both default to
//!   `127.0.0.1`.
//! * Per-server overrides (take precedence over the shared values):
//!   `SRP_<SERVER>_BIND`, `SRP_<SERVER>_PUBLIC_HOST`, `SRP_<SERVER>_PORT`
//!   where `<SERVER>` is one of `BROWSER`, `LOGIN`, `LOBBY`, `MATCH`.
//!   The stock client always reaches the browser on public port 80, so
//!   `SRP_BROWSER_PORT` is only useful with external port forwarding.
//! * A client on this machine or its local network is told the address it
//!   reached srp at instead of the public host (see
//!   [`ServerConfig::host_for`]), so it plays without the router looping its
//!   traffic back from the public address.
//! * `SRP_LOCAL_PEERS=off` tells every client the public host, even one on
//!   the server's own network; for a server behind a NAT (Docker's bridge)
//!   whose own address means nothing to a client.
//! * The login server's presence pings (UDP): `SRP_LOGIN_UDP_BIND` (defaults
//!   to the login server's bind host) and `SRP_LOGIN_UDP_PORT` (defaults to
//!   the login port). The 0.21d client pings the login server's IP at the
//!   port number it signed in at (0.100b always used 25100), so the port too
//!   is only useful with external port forwarding.
//!
//! Example — host the mock on a VPS whose public DNS is `srp.example.com`:
//! ```text
//! SRP_BIND_HOST=0.0.0.0 SRP_PUBLIC_HOST=srp.example.com cargo run --bin lobby-server
//! ```

use std::net::IpAddr;
use std::sync::OnceLock;

/// Address configuration for a single server.
pub struct ServerConfig {
    /// Local interface to bind/listen on (e.g. `0.0.0.0`).
    pub bind_host: String,
    /// Host advertised to the client so it can connect here next.
    pub public_host: String,
    pub port: u16,
    /// Whether a client on this machine's network is told the address it
    /// reached srp at (see [`ServerConfig::host_for`]). Off behind a NAT such
    /// as Docker's bridge, where that address is the container's own.
    pub local_peers: bool,
}

impl ServerConfig {
    /// `bind_host:port` — pass to `TcpListener::bind` / `UdpSocket::bind`.
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.bind_host, self.port)
    }

    /// `public_host:port` — what we tell the client to connect to.
    pub fn public_addr(&self) -> String {
        format!("{}:{}", self.public_host, self.port)
    }

    /// The host to tell a client at `peer` for this server, when that client
    /// reached the server `via` at `reached`. A client on this machine or its
    /// local network gets `reached`, if this server is advertised at the same
    /// host as `via` (both are on this machine): srp's own player, behind the
    /// router that forwards the public address, would otherwise need the
    /// router to loop its traffic back. Everyone else gets the public host.
    pub fn host_for(&self, via: &ServerConfig, peer: IpAddr, reached: IpAddr) -> String {
        match reached.to_canonical() {
            // The client takes an IPv4 address or a name.
            IpAddr::V4(reached)
                if self.local_peers && is_local(peer) && via.public_host == self.public_host =>
            {
                reached.to_string()
            }
            _ => self.public_host.clone(),
        }
    }
}

/// Whether `ip` is this machine or its local network: loopback, private
/// (10/8, 172.16/12, 192.168/16) or link-local. Carrier-grade NAT addresses
/// (100.64/10) are not: a client there comes through the internet.
fn is_local(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local(),
    }
}

pub struct Config {
    pub browser_server: ServerConfig,
    pub login_server: ServerConfig,
    /// Where the login server hears presence pings. Its public host is the
    /// login server's: the client pings the IP it signed in at.
    pub login_pings: ServerConfig,
    pub lobby_server: ServerConfig,
    pub match_server: ServerConfig,
}

/// Default public host when `SRP_PUBLIC_HOST` is unset — preserves the original
/// loopback-only behaviour for local development.
const DEFAULT_PUBLIC_HOST: &str = "127.0.0.1";
/// Default bind host — preserves the original local-only behaviour. Remote
/// hosting must be opted into explicitly.
const DEFAULT_BIND_HOST: &str = "127.0.0.1";

impl Config {
    /// Build the config from the environment (see module docs). Called once by
    /// [`get`]; reads `std::env` so it must run after the process has its env.
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> Result<Self, String> {
        let shared_bind_host =
            lookup("SRP_BIND_HOST").unwrap_or_else(|| DEFAULT_BIND_HOST.to_string());
        let shared_public_host =
            lookup("SRP_PUBLIC_HOST").unwrap_or_else(|| DEFAULT_PUBLIC_HOST.to_string());

        // `SRP_LOCAL_PEERS=off`: everyone is told the public host.
        let local_peers_on = !lookup("SRP_LOCAL_PEERS")
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("off"));
        let mut server = |name: &str, default_port: u16| {
            let port = match lookup(&format!("SRP_{name}_PORT")) {
                Some(port) => port.parse().map_err(|_| {
                    format!("SRP_{name}_PORT is `{port}`; it must be a port number (0-65535)")
                })?,
                None => default_port,
            };
            Ok::<_, String>(ServerConfig {
                bind_host: lookup(&format!("SRP_{name}_BIND"))
                    .unwrap_or_else(|| shared_bind_host.clone()),
                public_host: lookup(&format!("SRP_{name}_PUBLIC_HOST"))
                    .unwrap_or_else(|| shared_public_host.clone()),
                port,
                local_peers: local_peers_on,
            })
        };

        let browser_server = server("BROWSER", 80)?;
        let login_server = server("LOGIN", 1234)?;
        let lobby_server = server("LOBBY", 1235)?;
        let match_server = server("MATCH", 1236)?;
        let login_pings = server("LOGIN_UDP", login_server.port)?;
        let login_pings = ServerConfig {
            bind_host: lookup("SRP_LOGIN_UDP_BIND")
                .unwrap_or_else(|| login_server.bind_host.clone()),
            public_host: login_server.public_host.clone(),
            port: login_pings.port,
            local_peers: login_pings.local_peers,
        };
        Ok(Self {
            browser_server,
            login_server,
            login_pings,
            lobby_server,
            match_server,
        })
    }
}

static CONFIG: OnceLock<Config> = OnceLock::new();

/// Read the configuration from the environment, or say what is wrong with
/// it. A binary calls this first, so a bad value stops it at start.
pub fn load() -> Result<&'static Config, String> {
    if let Some(config) = CONFIG.get() {
        return Ok(config);
    }
    let config = Config::from_env()?;
    Ok(CONFIG.get_or_init(|| config))
}

/// Process-wide configuration, initialised from the environment on first use.
pub fn get() -> &'static Config {
    load().unwrap_or_else(|error| panic!("{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn local_peers_can_be_switched_off() {
        let on = Config::from_lookup(|_| None).unwrap();
        assert!(on.lobby_server.local_peers && on.login_pings.local_peers);
        let off =
            Config::from_lookup(|name| (name == "SRP_LOCAL_PEERS").then(|| "off".into())).unwrap();
        assert!(!off.lobby_server.local_peers && !off.login_pings.local_peers);
        let ip = |text: &str| text.parse().unwrap();
        let mut off = off;
        off.lobby_server.public_host = "203.0.113.9".into();
        assert_eq!(
            off.lobby_server
                .host_for(&off.lobby_server, ip("172.18.0.1"), ip("172.18.0.2")),
            "203.0.113.9"
        );
    }

    #[test]
    fn defaults_remain_local_only() {
        let config = Config::from_lookup(|_| None).unwrap();
        assert_eq!(config.login_server.bind_addr(), "127.0.0.1:1234");
        assert_eq!(config.login_server.public_addr(), "127.0.0.1:1234");
        assert_eq!(config.login_pings.bind_addr(), "127.0.0.1:1234");
    }

    #[test]
    fn presence_pings_are_heard_where_the_login_server_is() {
        let values = HashMap::from([("SRP_LOGIN_BIND", "127.0.0.3")]);
        let config = Config::from_lookup(|name| values.get(name).map(ToString::to_string)).unwrap();
        assert_eq!(config.login_pings.bind_addr(), "127.0.0.3:1234");

        let values = HashMap::from([
            ("SRP_LOGIN_UDP_BIND", "127.0.0.4"),
            ("SRP_LOGIN_UDP_PORT", "25101"),
        ]);
        let config = Config::from_lookup(|name| values.get(name).map(ToString::to_string)).unwrap();
        assert_eq!(config.login_pings.bind_addr(), "127.0.0.4:25101");
    }

    #[test]
    fn shared_hosts_and_per_server_overrides_are_applied() {
        let values = HashMap::from([
            ("SRP_BIND_HOST", "0.0.0.0"),
            ("SRP_PUBLIC_HOST", "srp.example.com"),
            ("SRP_MATCH_BIND", "127.0.0.2"),
            ("SRP_MATCH_PORT", "4321"),
        ]);
        let config = Config::from_lookup(|name| values.get(name).map(ToString::to_string)).unwrap();

        assert_eq!(config.login_server.bind_addr(), "0.0.0.0:1234");
        assert_eq!(config.login_server.public_addr(), "srp.example.com:1234");
        assert_eq!(config.match_server.bind_addr(), "127.0.0.2:4321");
        assert_eq!(config.match_server.public_addr(), "srp.example.com:4321");
    }

    #[test]
    fn local_clients_are_told_the_address_they_reached() {
        let values = HashMap::from([("SRP_PUBLIC_HOST", "203.0.113.67")]);
        let config = Config::from_lookup(|name| values.get(name).map(ToString::to_string)).unwrap();
        let (lobby, browser) = (&config.lobby_server, &config.browser_server);
        let ip = |text: &str| text.parse::<IpAddr>().unwrap();

        // srp's own machine, and another one behind the same router.
        assert_eq!(
            lobby.host_for(browser, ip("127.0.0.1"), ip("127.0.0.1")),
            "127.0.0.1"
        );
        assert_eq!(
            lobby.host_for(browser, ip("192.168.50.20"), ip("192.168.50.134")),
            "192.168.50.134"
        );
        assert_eq!(
            lobby.host_for(browser, ip("10.0.0.5"), ip("10.0.0.2")),
            "10.0.0.2"
        );
        assert_eq!(
            lobby.host_for(browser, ip("172.20.1.1"), ip("172.20.1.2")),
            "172.20.1.2"
        );
        assert_eq!(
            lobby.host_for(
                browser,
                ip("::ffff:192.168.50.20"),
                ip("::ffff:192.168.50.134")
            ),
            "192.168.50.134"
        );
        // Through the internet, carrier-grade NAT included.
        assert_eq!(
            lobby.host_for(browser, ip("198.51.100.47"), ip("192.168.50.134")),
            "203.0.113.67"
        );
        assert_eq!(
            lobby.host_for(browser, ip("100.64.0.1"), ip("192.168.50.134")),
            "203.0.113.67"
        );
        // An address the client can't take.
        assert_eq!(lobby.host_for(browser, ip("::1"), ip("::1")), "203.0.113.67");
    }

    #[test]
    fn a_server_advertised_elsewhere_keeps_its_public_host() {
        let values = HashMap::from([
            ("SRP_PUBLIC_HOST", "203.0.113.67"),
            ("SRP_MATCH_PUBLIC_HOST", "match.example.com"),
        ]);
        let config = Config::from_lookup(|name| values.get(name).map(ToString::to_string)).unwrap();
        let local = "127.0.0.1".parse::<IpAddr>().unwrap();
        assert_eq!(
            config
                .match_server
                .host_for(&config.lobby_server, local, local),
            "match.example.com"
        );
    }

    #[test]
    fn a_bad_port_is_an_error_naming_it() {
        let error = Config::from_lookup(|name| (name == "SRP_LOBBY_PORT").then(|| "12a".into()))
            .err()
            .unwrap();
        assert!(error.contains("SRP_LOBBY_PORT"), "{error}");
    }
}
