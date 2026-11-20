//! Canaux propres au serveur (`srv:*`, `social:*`, cookies), et ceux du noyau dont le sens change sur un serveur
//! partagé (« en écoute », dossiers attribués par l'administrateur, sources de pochettes).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{Call, public_playlists};
use crate::hub::Hub;
use crate::social::Social;
use crate::util::{HttpResult, http_error, https_host, inside};
use control_core::core::{int, text, truthy};
use control_core::downloads::cookies;

const CHANNELS: &[&str] = &[
    "srv:me:add-folder",
    "dl:cookies-status",
    "dl:set-cookies",
    "dl:clear-cookies",
    "srv:info",
    "srv:browse",
    "srv:add-folder",
    "srv:remove-folder",
    "srv:rescan",
    "srv:settings",
    "srv:settings:get",
    "srv:backup:status",
    "srv:backup:run",
    "srv:backup:test",
    "srv:update:status",
    "srv:update:check",
    "srv:update:download",
    "srv:update:install",
    "srv:users:list",
    "srv:users:create",
    "srv:users:detail",
    "srv:users:add-folder",
    "srv:users:remove-folder",
    "srv:users:personal-folder",
    "srv:users:set-grants",
    "srv:users:update",
    "srv:users:password",
    "srv:users:delete",
    "social:me",
    "social:set-profile",
    "social:set-grants",
    "social:set-listening",
    "social:others",
    "social:profile",
    "social:compare",
    "srv:me:sessions",
    "srv:me:revoke-session",
    "srv:me:revoke-others",
    "srv:me:subsonic",
    "srv:me:subsonic-reveal",
    "srv:me:subsonic-renew",
    "srv:subsonic:log",
    "srv:me:subsonic-revoke",
    "srv:me:password",
];

/// Hôtes de pochettes que le serveur accepte de télécharger (résultats de la recherche de pochettes).
const COVER_HOSTS: &[&str] = &["mzstatic.com", "dzcdn.net", "deezer.com", "coverartarchive.org", "archive.org"];

pub fn handles(channel: &str) -> bool {
    CHANNELS.contains(&channel)
}

fn to<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or_default()
}

fn arg(args: &[Value], i: usize) -> &Value {
    static NULL: Value = Value::Null;
    args.get(i).unwrap_or(&NULL)
}

fn owner_of(hub: &Hub, id: &Value) -> HttpResult<crate::auth::User> {
    hub.accounts
        .by_id(&text(id))
        .filter(|u| !u.disabled)
        .ok_or_else(|| http_error(404, "not_found", "Compte introuvable."))
}

/// Le mot de passe du compte, pour voir ou renouveler le mot de passe d'application.
fn confirm_password(call: &Call<'_>, password: &Value) -> HttpResult<()> {
    let key = format!("subsonic:{}", call.user.id);
    if call.hub.login_throttle.wait(&key) > 0 {
        return Err(http_error(429, "throttled", "Trop d’essais : patientez quelques minutes."));
    }
    if call.hub.accounts.check(&call.user.username, &text(password)).is_none() {
        call.hub.login_throttle.fail(&key);
        return Err(http_error(403, "wrong_password", "Mot de passe incorrect."));
    }
    Ok(())
}

fn stats_of(entry: &crate::hub::Entry, from: &str, to: &str, limit: usize) -> HttpResult<Value> {
    let db = entry.core.db.lock();
    let stats = control_core::stats::get(&db, from, to, limit, crate::util::now_ms())
        .map_err(|e| http_error(500, "error", e.to_string()))?;
    Ok(serde_json::to_value(stats).unwrap_or_default())
}

/// Traite un canal du serveur ; `None` : c'est un canal du noyau, à lui de répondre.
pub fn invoke(call: &Call<'_>, channel: &str, args: &[Value]) -> HttpResult<Option<Value>> {
    let (hub, user, entry) = (call.hub, call.user, call.entry);
    let a = |i| arg(args, i);
    let cookies_file = || entry.core.dir.join("cookies.txt");
    Ok(Some(match channel {
        // ─── Canaux du noyau corrigés ──────────────────────────────────────
        "lib:play-start" => {
            let play = entry.core.dispatch_from(call.client, channel, args)?;
            // « En écoute » : le serveur sait ce que ce compte joue et le montre à ceux qu'il a choisis.
            if let (Some(id), Some(play_id)) = (int(a(0)), play.as_i64())
                && let Some(t) = entry.core.library.track(id)
            {
                let track = json!({
                    "id": t.id, "kind": t.kind, "title": t.title, "artist": t.artist, "album": t.album,
                    "cover": t.cover, "duration": t.duration,
                });
                hub.presence.start(hub, &entry.id, play_id, track);
            }
            play
        }
        // Les dossiers sont attribués par l'administrateur : un compte ne règle que les siens via ces commandes.
        "lib:remove-folder" => hub.remove_own_folder(&user.id, &text(a(0))),
        "srv:me:add-folder" => hub.add_own_folder(&user.id, &text(a(0)))?,
        "tags:stage-cover" => {
            let src = a(0);
            if let Some(data) = src["data"].as_str().filter(|s| !s.is_empty()) {
                return entry.core.dispatch_from(call.client, channel, &[json!({ "data": data })]).map(Some);
            }
            let host = https_host(&text(&src["url"])).to_lowercase();
            if host.is_empty() || !COVER_HOSTS.iter().any(|h| host == *h || host.ends_with(&format!(".{h}"))) {
                return Ok(Some(json!({ "ok": false, "message": "Source de pochette refusée." })));
            }
            return entry.core.dispatch_from(call.client, channel, &[json!({ "url": src["url"] })]).map(Some);
        }

        // ─── Cookies YouTube du compte ─────────────────────────────────────
        "dl:cookies-status" => to(&cookies::status(&cookies_file())),
        "dl:set-cookies" => {
            let input = a(0).as_str().unwrap_or("");
            if !a(0).is_string() {
                return Err(http_error(400, "invalid_cookies", "Fichier vide."));
            }
            cookies::save(&cookies_file(), input).map_err(|e| http_error(400, "invalid_cookies", e.to_string()))?;
            to(&cookies::status(&cookies_file()))
        }
        "dl:clear-cookies" => {
            cookies::clear(&cookies_file());
            json!({ "present": false })
        }

        // ─── Serveur ───────────────────────────────────────────────────────
        "srv:info" => {
            let mut v = json!({
                "name": hub.server_name(), "role": user.role, "registrationEnabled": hub.setting_bool("registrationEnabled"),
                "allowUserDownloads": hub.setting_bool("allowUserDownloads"), "folders": hub.own_folders(&user.id),
                "commonFolders": hub.common_folders(), "downloadDir": hub.download_dir_for(user).to_string_lossy(),
                "usersRoot": hub.setting_str("usersRoot"), "hasTmdbKey": !hub.setting_str("tmdbKey").is_empty(),
            });
            if user.is_admin() {
                v["users"] = json!(hub.accounts.count());
            }
            v
        }
        "srv:browse" => hub.browse(&text(a(0)))?,
        "srv:add-folder" => hub.add_folder(&text(a(0)))?,
        "srv:remove-folder" => hub.remove_folder(&text(a(0)))?,
        "srv:rescan" => {
            hub.rescan_all();
            json!(true)
        }
        "srv:settings" => {
            hub.patch_settings(if a(0).is_object() { a(0) } else { &Value::Null })?;
            hub.public_settings()
        }
        "srv:settings:get" => hub.public_settings(),
        "srv:backup:status" => hub.backups.status(hub),
        "srv:backup:run" => {
            hub.backups.run(hub, "manuelle")?;
            hub.backups.status(hub)
        }
        "srv:backup:test" => json!(hub.backups.test(hub)?),
        "srv:update:status" => hub.updater.status(),
        "srv:update:check" => hub.updater.check(true),
        "srv:update:download" => hub.updater.download(),
        "srv:update:install" => hub.updater.install(),

        // ─── Comptes ───────────────────────────────────────────────────────
        "srv:users:list" => json!(hub.accounts.list()),
        "srv:users:create" => {
            let body = a(0);
            let created =
                hub.accounts.create(&text(&body["username"]), &text(&body["password"]), &text(&body["role"]))?;
            if !hub.setting_str("usersRoot").is_empty()
                && let Err(err) = hub.ensure_personal_folder(&created)
            {
                eprintln!("Dossier du compte : {}", err.message);
            }
            created.public()
        }
        "srv:users:detail" => {
            let u =
                hub.accounts.by_id(&text(a(0))).ok_or_else(|| http_error(404, "not_found", "Compte introuvable."))?;
            json!({ "folders": hub.own_folders(&u.id), "grants": hub.social.grants_of(&u.id), "downloadDir": hub.download_dir_for(&u).to_string_lossy() })
        }
        "srv:users:add-folder" => hub.add_own_folder(&text(a(0)), &text(a(1)))?,
        "srv:users:remove-folder" => hub.remove_own_folder(&text(a(0)), &text(a(1))),
        "srv:users:personal-folder" => {
            let u =
                hub.accounts.by_id(&text(a(0))).ok_or_else(|| http_error(404, "not_found", "Compte introuvable."))?;
            hub.ensure_personal_folder(&u)?
        }
        "srv:users:set-grants" => hub.social.set_grants(&text(a(0)), a(1))?,
        "srv:users:update" => {
            let (id, body) = (text(a(0)), a(1));
            let demote = body["role"].as_str().is_some_and(|r| !r.is_empty() && r != "admin");
            if id == user.id && body.is_object() && (truthy(&body["disabled"]) || demote) {
                return Err(http_error(409, "self", "Vous ne pouvez pas retirer vos propres droits."));
            }
            let disabled = body.get("disabled").filter(|v| !v.is_null()).map(truthy);
            hub.accounts.update(&id, body["role"].as_str(), disabled)?;
            if truthy(&body["disabled"]) {
                hub.drop_user(&id);
            }
            json!(true)
        }
        "srv:users:password" => {
            let id = text(a(0));
            hub.accounts.set_password(&id, a(1).as_str().unwrap_or(""))?;
            hub.drop_user(&id);
            json!(true)
        }
        "srv:users:delete" => {
            let id = text(a(0));
            if id == user.id {
                return Err(http_error(409, "self", "Vous ne pouvez pas supprimer votre propre compte."));
            }
            hub.accounts.remove(&id)?;
            hub.forget_user(&id);
            json!(true)
        }

        // ─── Profils, partages, comparaison ────────────────────────────────
        "social:me" => {
            let me = hub.accounts.by_id(&user.id).ok_or_else(|| http_error(404, "not_found", "Compte introuvable."))?;
            json!({
                "shareListening": me.share_listening, "profile": Social::profile(&me), "grants": hub.social.grants_of(&user.id),
                "folders": hub.own_folders(&user.id), "commonFolders": hub.common_folders(),
            })
        }
        "social:set-profile" => hub.social.set_profile(&user.id, if a(0).is_object() { a(0) } else { &Value::Null })?,
        "social:set-grants" => hub.social.set_grants(&user.id, a(0))?,
        "social:set-listening" => {
            let on = truthy(a(0));
            let _ = hub
                .db
                .lock()
                .execute("UPDATE users SET share_listening = ? WHERE id = ?", rusqlite::params![on as i64, user.id]);
            if !on {
                hub.presence.clear(hub, &user.id);
            }
            json!(on)
        }
        // Les comptes avec lesquels on partage, et ce que ceux qui l'ont choisi écoutent en ce moment.
        "social:others" => {
            let now = hub.presence.snapshot_for(hub, &user.id);
            let list: Vec<Value> = hub
                .social
                .others(user)
                .into_iter()
                .map(|(u, mut p, _)| {
                    p["listening"] = now.get(&u.id).cloned().unwrap_or(Value::Null);
                    p
                })
                .collect();
            json!(list)
        }
        "social:profile" => {
            let owner = owner_of(hub, a(0))?;
            let access = hub.social.access(user, &owner);
            let listening = hub.presence.snapshot_for(hub, &user.id).get(&owner.id).cloned().unwrap_or(Value::Null);
            let mut out = json!({
                "profile": Social::profile(&owner), "access": access.to_json(), "library": null, "playlists": [],
                "stats": null, "listening": listening,
            });
            if access.browse {
                let other = hub.entry(&owner)?;
                let roots = hub.owner_roots(&owner);
                let tracks: Vec<_> = other
                    .core
                    .library
                    .tracks()
                    .into_iter()
                    .filter(|t| t.kind == control_core::model::Kind::Audio && inside(&roots, &t.path))
                    .collect();
                let artist = |t: &control_core::model::Track| {
                    if t.album_artist.is_empty() { t.artist.clone() } else { t.album_artist.clone() }
                };
                let albums: HashSet<String> = tracks.iter().map(|t| format!("{}|{}", t.album, artist(t))).collect();
                let artists: HashSet<String> = tracks.iter().map(artist).collect();
                out["library"] = json!({ "tracks": tracks.len(), "albums": albums.len(), "artists": artists.len() });
                let (playlists, _) = public_playlists(&other);
                out["playlists"] = Value::Array(
                    playlists
                        .into_iter()
                        .map(|p| json!({ "id": p["id"], "name": p["name"], "description": p["description"], "cover": p["cover"], "count": p["count"] }))
                        .collect(),
                );
            }
            if access.compare {
                let year = control_core::util::local_datetime(crate::util::now_ms()).year();
                let other = if owner.id == user.id { entry.clone() } else { hub.entry(&owner)? };
                let stats = stats_of(&other, &format!("{year}-01-01"), &format!("{year}-12-31"), 5)?;
                let top: Vec<Value> = stats["music"]["topArtists"]
                    .as_array()
                    .map(|a| a.iter().take(3).map(|x| x["name"].clone()).collect())
                    .unwrap_or_default();
                out["stats"] = json!({
                    "year": year, "plays": stats["music"]["plays"], "seconds": stats["music"]["seconds"].as_f64().unwrap_or(0.0).round() as i64,
                    "topArtists": top, "streak": stats["streak"],
                });
            }
            out
        }
        "social:compare" => {
            let owner = owner_of(hub, a(0))?;
            if owner.id == user.id {
                return Err(http_error(400, "self", "Choisissez un autre compte."));
            }
            if !hub.social.access(user, &owner).compare {
                return Err(http_error(403, "forbidden", "Ce compte ne partage pas ses statistiques avec vous."));
            }
            let (from, to_) = (text(&a(1)["from"]), text(&a(1)["to"]));
            let mine = stats_of(entry, &from, &to_, 100)?;
            let theirs = stats_of(&*hub.entry(&owner)?, &from, &to_, 100)?;
            let mut out = Map::new();
            out.insert("with".into(), Social::profile(&owner));
            out.insert("years".into(), mine["years"].clone());
            out.insert("months".into(), mine["months"].clone());
            if let Value::Object(c) = crate::compare::compare(&mine, &theirs) {
                out.extend(c);
            }
            Value::Object(out)
        }

        // ─── Sessions et mots de passe ─────────────────────────────────────
        "srv:me:sessions" => json!(hub.accounts.sessions_of(&user.id, &user.sid)),
        "srv:me:revoke-session" => json!(hub.accounts.revoke_session(&user.id, &text(a(0)))),
        "srv:me:revoke-others" => json!(hub.accounts.revoke_others(&user.id, &user.sid)),
        "srv:me:subsonic" => json!({
            "enabled": hub.setting_bool("subsonicEnabled"), "hasSecret": hub.subsonic.secret_of(hub, &user.id).is_some(),
            "username": user.username,
        }),
        "srv:me:subsonic-reveal" | "srv:me:subsonic-renew" => {
            confirm_password(call, a(0))?;
            json!({ "secret": hub.subsonic.ensure_secret(hub, &user.id, channel == "srv:me:subsonic-renew") })
        }
        "srv:subsonic:log" => hub.subsonic.recent(),
        "srv:me:subsonic-revoke" => {
            hub.subsonic.revoke_secret(hub, &user.id);
            json!(true)
        }
        "srv:me:password" => {
            if hub.accounts.check(&user.username, &text(a(0))).is_none() {
                return Err(http_error(403, "wrong_password", "Mot de passe actuel incorrect."));
            }
            hub.accounts.set_password(&user.id, a(1).as_str().unwrap_or(""))?;
            json!(true)
        }
        _ => return Ok(None),
    }))
}
