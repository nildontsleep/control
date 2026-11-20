//! control, version web : `control-server` sert la bibliothèque et l'application à des navigateurs ;
//! `control-server --launcher` le fait tourner sous un lanceur qui permet la mise à jour automatique.
//! Configuration : variables d'environnement (README, section « Version web »).

use std::sync::Arc;

use control_server::config::Config;
use control_server::hub::{Hub, HubOptions, version};
use control_server::launcher::{self, Supervisor};

/// Journal minimal : avertissements et erreurs du noyau sur la sortie d'erreur.
struct Logger;

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.level() <= log::Level::Warn
    }

    fn log(&self, r: &log::Record<'_>) {
        if self.enabled(r.metadata()) {
            eprintln!("[{}] {}", r.level(), r.args());
        }
    }

    fn flush(&self) {}
}

static LOGGER: Logger = Logger;

async fn shutdown(stop: Arc<tokio::sync::Notify>) {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let term = async {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut s) => {
                    s.recv().await;
                }
                Err(_) => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = ctrl_c => {}
            _ = term => {}
            _ = stop.notified() => {}
        }
    }
    #[cfg(not(unix))]
    tokio::select! {
        _ = ctrl_c => {}
        _ = stop.notified() => {}
    }
}

async fn run() -> anyhow::Result<()> {
    let config = Config::from_env();
    // Lancé par le lanceur : il peut demander l'arrêt, et c'est lui qui redémarre sur une nouvelle version.
    let stop = Arc::new(tokio::sync::Notify::new());
    let on_stop = stop.clone();
    let supervisor = Supervisor::connect(move || on_stop.notify_one());
    let restart = supervisor.clone().map(|s| Arc::new(move || s.restart()) as Arc<dyn Fn() + Send + Sync>);
    let hub = Hub::new(config.clone(), HubOptions { supervisor: restart, ..HubOptions::default() })?;
    let listener = tokio::net::TcpListener::bind((config.host.as_str(), config.port)).await?;
    hub.start();
    let shown = if config.host == "0.0.0.0" || config.host == "::" { "localhost" } else { config.host.as_str() };
    println!("control (web) écoute sur http://{shown}:{}", config.port);
    println!("Données : {}", config.data_dir.display());
    if let Some(s) = &supervisor {
        s.ready(&version());
    }
    if hub.accounts.count() == 0 {
        println!(
            "Aucun compte pour l'instant : ouvrez http://{shown}:{}/setup pour créer l'administrateur.",
            config.port
        );
        if config.setup_token.is_empty() {
            println!(
                "Astuce : définissez CONTROL_SETUP_TOKEN pour que seul vous puissiez créer ce premier compte tant que le serveur est exposé."
            );
        }
    }
    let closing = hub.clone();
    control_server::serve(listener, hub, async move {
        shutdown(stop).await;
        println!("Arrêt…");
        // Ferme les flux d'événements : sans ça, l'arrêt attendrait que chaque page se déconnecte.
        closing.close();
    })
    .await?;
    Ok(())
}

fn main() {
    let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(log::LevelFilter::Warn));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(err) => {
            eprintln!("control : {err}");
            std::process::exit(1);
        }
    };
    if args.first().map(String::as_str) == Some("--launcher") {
        let code = runtime.block_on(launcher::run(args[1..].to_vec()));
        std::process::exit(code);
    }
    if let Err(err) = runtime.block_on(run()) {
        eprintln!("control : {err:#}");
        std::process::exit(1);
    }
    runtime.shutdown_timeout(std::time::Duration::from_millis(300));
}
