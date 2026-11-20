//! Serveur web de control : il héberge la bibliothèque et l'application, accessible depuis un navigateur. Même API
//! HTTP, mêmes cookies, mêmes formats et même dossier de données que la version Node (`server/*.js`).

pub mod app;
pub mod appstate;
pub mod auth;
pub mod backup;
pub mod cli;
pub mod compare;
pub mod config;
pub mod http;
pub mod hub;
pub mod launcher;
pub mod presence;
pub mod rpc;
pub mod secrets;
pub mod serverdb;
pub mod sign;
pub mod social;
pub mod subsonic;
pub mod tar;
pub mod updater;
pub mod upload;
pub mod util;

use std::net::SocketAddr;
use std::sync::Arc;

use config::Config;
use hub::{Hub, HubOptions};

/// Le routeur de l'application (un seul gestionnaire, voir [`app::handle`]).
pub fn router(hub: Arc<Hub>) -> axum::Router {
    axum::Router::new().fallback(app::handle).with_state(hub)
}

/// Sert `hub` sur `listener` jusqu'à ce que `stop` se termine.
pub async fn serve(
    listener: tokio::net::TcpListener,
    hub: Arc<Hub>,
    stop: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let app = router(hub).into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, app).with_graceful_shutdown(stop).await
}

/// Un serveur lancé sur un fil à lui (essais, outils) : arrêté quand on le libère.
pub struct Background {
    pub addr: SocketAddr,
    pub hub: Arc<Hub>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Background {
    /// Lance le serveur sur `config.host:config.port` (port 0 : un port libre).
    pub fn start(config: Config, options: HubOptions) -> anyhow::Result<Background> {
        let hub = Hub::new(config.clone(), options)?;
        hub.start();
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build()?;
        let listener = runtime.block_on(tokio::net::TcpListener::bind((config.host.as_str(), config.port)))?;
        let addr = listener.local_addr()?;
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let served = hub.clone();
        let thread = std::thread::Builder::new().name("control-http".into()).spawn(move || {
            runtime.block_on(async move {
                let _ = serve(listener, served, async move {
                    let _ = rx.await;
                })
                .await;
            });
            runtime.shutdown_timeout(std::time::Duration::from_millis(200));
        })?;
        Ok(Background { addr, hub, stop: Some(tx), thread: Some(thread) })
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for Background {
    fn drop(&mut self) {
        self.hub.close();
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
