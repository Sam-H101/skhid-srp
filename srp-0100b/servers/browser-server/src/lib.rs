use std::net::SocketAddr;
use std::sync::Arc;

use actix_web::{
    App, HttpRequest, HttpResponse, HttpServer, Responder, get, middleware::Logger, web,
};

use session::SessionStore;
use vostok::config;

#[derive(serde::Deserialize, Debug)]
#[expect(dead_code)]
struct QueryParams {
    unused: Option<String>,
    r#type: Option<u8>,
    local_ip: Option<String>,
    login_ip: Option<String>,
}

/// Both ends of a browser connection, kept for its requests.
#[derive(Clone, Copy)]
struct Ends {
    peer: SocketAddr,
    reached: SocketAddr,
}

#[get("/hello")]
async fn handle_request_lobby_server(
    req: HttpRequest,
    query: web::Query<QueryParams>,
) -> impl Responder {
    log::debug!("Received: {query:#?}");

    // Where the client finds the lobby: see `ServerConfig::host_for`.
    let config = config::get();
    let lobby = &config.lobby_server;
    let host = match req.conn_data::<Ends>() {
        Some(ends) => lobby.host_for(&config.browser_server, ends.peer.ip(), ends.reached.ip()),
        None => lobby.public_host.clone(),
    };
    HttpResponse::Ok().body(format!("{host}:{}", lobby.port))
}

/// Run the browser server to completion on the current thread (it owns a fresh
/// actix runtime), so it can be launched standalone or as one thread of the
/// unified `srp` binary. The session store is unused — the browser server only
/// hands the client the lobby address.
pub fn run(_store: Arc<SessionStore>) -> std::io::Result<()> {
    actix_web::rt::System::new().block_on(serve())
}

async fn serve() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .wrap(Logger::default())
            .service(handle_request_lobby_server)
    })
    .on_connect(|connection, data| {
        if let Some(stream) = connection.downcast_ref::<actix_web::rt::net::TcpStream>()
            && let (Ok(peer), Ok(reached)) = (stream.peer_addr(), stream.local_addr())
        {
            data.insert(Ends { peer, reached });
        }
    })
    .bind(config::get().browser_server.bind_addr())?
    .run()
    .await
}
