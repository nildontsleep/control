//! Les commandes que la page envoie au serveur : les mêmes canaux que l'IPC d'Electron (le noyau les traite tels
//! quels, voir [`Core::dispatch`]), plus quelques canaux « srv: » et « social: » propres au serveur. Chaque appel passe
//! par un contrôle des droits. Repris de `server/rpc.js`.

mod server;

use std::sync::Arc;

use serde_json::{Value, json};

use crate::auth::User;
use crate::hub::{Entry, Hub};
use crate::util::{HttpResult, http_error, inside, relative};
use control_core::core::{Core, int};

const ADMIN_ONLY: &[&str] = &[
    "lib:remove-folder",
    "tools:install",
    "tools:update",
    "tags:write",
    "tags:undo",
    "lyrics:save-file",
    "srv:browse",
    "srv:add-folder",
    "srv:remove-folder",
    "srv:settings",
    "srv:users:list",
    "srv:users:create",
    "srv:users:update",
    "srv:users:password",
    "srv:users:delete",
    "srv:rescan",
    "srv:me:add-folder",
    "srv:users:detail",
    "srv:users:add-folder",
    "srv:users:remove-folder",
    "srv:users:personal-folder",
    "srv:users:set-grants",
    "srv:update:status",
    "srv:update:check",
    "srv:update:download",
    "srv:update:install",
    "srv:backup:status",
    "srv:backup:run",
    "srv:backup:test",
    "srv:subsonic:log",
];
const DOWNLOADS: &[&str] = &[
    "dl:add",
    "dl:info",
    "dl:search",
    "dl:cancel",
    "dl:retry",
    "dl:remove",
    "dl:clear",
    "dl:cookies-status",
    "dl:set-cookies",
    "dl:clear-cookies",
];
/// Des chemins du poste de l'utilisateur ne veulent rien dire pour un serveur.
const DENIED: &[&str] = &["lib:add-paths"];

/// Dans la bibliothèque d'un autre compte (lecture seule) : rien de ce qui la modifierait ou la ferait modifier.
pub fn read_only(channel: &str) -> bool {
    channel.starts_with("pl:")
        || channel.starts_with("tags:")
        || ["lib:remove-tracks", "lib:remove-folder", "lyrics:set", "lyrics:save-file"].contains(&channel)
}

/// La bibliothèque d'un autre compte, ouverte en lecture seule.
pub struct View {
    pub owner: User,
    pub entry: Arc<Entry>,
}

/// Le refus éventuel d'un appel.
pub fn allowed(hub: &Hub, user: &User, channel: &str, args: &[Value]) -> Option<&'static str> {
    if DENIED.contains(&channel) {
        return Some("Indisponible sur la version web.");
    }
    if ADMIN_ONLY.contains(&channel) && !user.is_admin() {
        return Some("Réservé à l’administrateur.");
    }
    if DOWNLOADS.contains(&channel) && !user.is_admin() && !hub.setting_bool("allowUserDownloads") {
        return Some("Les téléchargements sont réservés à l’administrateur.");
    }
    if channel == "lib:remove-tracks" && args.get(1).is_some_and(control_core::core::truthy) && !user.is_admin() {
        return Some("Seul l’administrateur peut supprimer des fichiers.");
    }
    None
}

/// Les playlists qu'un compte a rendues visibles aux autres, avec leur nombre de morceaux.
pub fn public_playlists(entry: &Entry) -> (Vec<Value>, Vec<i64>) {
    let (playlists, items) = entry.core.library.playlists();
    let mut out = Vec::new();
    let mut ids = Vec::new();
    for p in playlists.iter().filter(|p| p.shared) {
        let mut v = serde_json::to_value(p).unwrap_or_default();
        v["count"] = json!(items.iter().filter(|i| i.playlist_id == p.id).count());
        out.push(v);
        ids.push(p.id);
    }
    (out, ids)
}

/// Contexte d'un appel.
pub struct Call<'a> {
    pub hub: &'a Arc<Hub>,
    pub entry: &'a Arc<Entry>,
    pub user: &'a User,
    pub client: &'a str,
}

fn read_only_error(owner: &User) -> crate::util::HttpError {
    http_error(403, "read_only", format!("Bibliothèque de {} : lecture seule.", owner.username))
}

/// La bibliothèque d'un autre compte, vue en lecture seule : ses morceaux (sans les dossiers communs, avec des chemins
/// relatifs), ses playlists publiques et les paroles. Les écoutes ne sont pas comptées, rien ne peut être modifié.
fn foreign(hub: &Hub, view: &View, channel: &str, args: &[Value]) -> HttpResult<Option<Value>> {
    let roots = hub.owner_roots(&view.owner);
    match channel {
        "lib:tracks" => {
            let tracks: Vec<Value> = view
                .entry
                .core
                .library
                .tracks()
                .into_iter()
                .filter(|t| inside(&roots, &t.path))
                .map(|mut t| {
                    t.path = relative(&roots, &t.path);
                    t.play_count = 0;
                    t.last_played = None;
                    t.source_url = None;
                    serde_json::to_value(t).unwrap_or_default()
                })
                .collect();
            Ok(Some(Value::Array(tracks)))
        }
        "lib:playlists" => {
            let (playlists, ids) = public_playlists(&view.entry);
            let items: Vec<Value> = view
                .entry
                .core
                .library
                .playlists()
                .1
                .into_iter()
                .filter(|i| ids.contains(&i.playlist_id))
                .map(|i| serde_json::to_value(i).unwrap_or_default())
                .collect();
            Ok(Some(json!({ "playlists": playlists, "items": items })))
        }
        "lyrics:get" => view.entry.core.dispatch("lyrics:get", args).map(Some),
        "lib:play-start" => Ok(Some(json!(0))),
        c if read_only(c) => Err(read_only_error(&view.owner)),
        _ => Ok(None),
    }
}

/// Une commande « invoke » : `/api/rpc/<canal>`.
pub fn invoke(call: &Call<'_>, channel: &str, args: &[Value], view: Option<&View>) -> HttpResult<Value> {
    if let Some(view) = view
        && let Some(v) = foreign(call.hub, view, channel, args)?
    {
        return Ok(v);
    }
    if !server::handles(channel) && !Core::has_channel(channel, false) {
        return Err(http_error(404, "unknown_channel", format!("Commande inconnue : {channel}")));
    }
    if let Some(refusal) = allowed(call.hub, call.user, channel, args) {
        return Err(http_error(403, "forbidden", refusal));
    }
    match server::invoke(call, channel, args)? {
        Some(v) => Ok(v),
        None => call.entry.core.dispatch_from(call.client, channel, args),
    }
}

/// Une commande « send » (sans réponse) : `/api/send/<canal>`.
pub fn send(call: &Call<'_>, channel: &str, args: &[Value], view: Option<&View>) -> HttpResult<()> {
    if let Some(view) = view {
        if channel == "lib:play-end" {
            return Ok(()); // pas d'écoute comptée dans la bibliothèque d'un autre
        }
        if read_only(channel) {
            return Err(read_only_error(&view.owner));
        }
    }
    if !Core::has_channel(channel, true) {
        return Err(http_error(404, "unknown_channel", format!("Commande inconnue : {channel}")));
    }
    if let Some(refusal) = allowed(call.hub, call.user, channel, args) {
        return Err(http_error(403, "forbidden", refusal));
    }
    match channel {
        "settings:set" => {
            let Some(patch) = args.first().and_then(Value::as_object) else { return Ok(()) };
            // Réglages qui décrivent le serveur ou les dossiers de ce compte : ils se règlent par les commandes du
            // serveur, pas par les réglages.
            let mut clean = patch.clone();
            for key in ["libraryFolders", "commonFolders", "downloadDir"] {
                clean.remove(key);
            }
            if let Some(dl) = clean.get_mut("dl").and_then(Value::as_object_mut) {
                dl.remove("cookiesFile"); // chemin du poste de l'utilisateur
            }
            if !clean.is_empty() {
                call.entry.core.dispatch_from(call.client, "settings:set", &[Value::Object(clean)])?;
            }
        }
        "lib:play-end" => {
            call.entry.core.dispatch_from(call.client, channel, args)?;
            call.hub.presence.end(call.hub, &call.entry.id, args.first().and_then(int));
        }
        _ => {
            call.entry.core.dispatch_from(call.client, channel, args)?;
        }
    }
    Ok(())
}
