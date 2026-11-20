use super::*;

fn asset(name: &str) -> Asset {
    Asset {
        name: name.into(),
        size: 100,
        browser_download_url: Some(format!("https://github.com/{REPO}/releases/download/v1.2.0/{name}")),
        url: Some(format!("https://api.github.com/repos/{REPO}/releases/assets/42")),
        digest: Some(format!("sha256:{}", "AB".repeat(32))),
    }
}

fn release(tag: &str) -> Release {
    Release {
        tag_name: tag.into(),
        name: None,
        body: Some("Notes".into()),
        html_url: Some(format!("https://github.com/{REPO}/releases/tag/{tag}")),
        draft: false,
        prerelease: false,
        assets: asset_names("1.2.0").iter().map(|n| asset(n)).collect(),
    }
}

#[test]
fn versions_compare_numerically() {
    assert_eq!(compare("1.10.0", "1.9.9"), Ordering::Greater);
    assert_eq!(compare("v1.2.0", "1.2"), Ordering::Equal);
    assert_eq!(compare("1.2.0-beta.1", "1.2.0"), Ordering::Equal);
    assert_eq!(compare("0.15.0", "1.0.0"), Ordering::Less);
    assert_eq!(compare("2", "1.99.99"), Ordering::Greater);
    assert_eq!(compare("x.y", "0.0"), Ordering::Equal);
}

#[test]
fn assets_match_the_edition() {
    let rel = release("v1.2.0");
    assert_eq!(pick_asset(&rel.assets, Edition::AppImage).unwrap().name, "control-1.2.0-x86_64.AppImage");
    assert_eq!(pick_asset(&rel.assets, Edition::Setup).unwrap().name, "control-1.2.0-setup.exe");
    assert_eq!(pick_asset(&rel.assets, Edition::Portable).unwrap().name, "control-1.2.0-portable.exe");
    assert!(pick_asset(&rel.assets, Edition::Deb).is_none());
    assert!(pick_asset(&rel.assets, Edition::Manual).is_none());
    let mut no_url = asset("control-1.2.0-SETUP.EXE");
    assert!(pick_asset(std::slice::from_ref(&no_url), Edition::Setup).is_some(), "insensible à la casse");
    no_url.browser_download_url = None;
    assert!(pick_asset(&[no_url], Edition::Setup).is_none());
    assert_eq!(Edition::parse("appimage"), Edition::AppImage);
    assert_eq!(Edition::parse("n'importe"), Edition::Manual);
}

#[test]
fn github_answers() {
    let raw = r#"{"tag_name":"v1.2.0","name":"control 1.2","body":"x","html_url":"https://github.com/nildontsleep/control/releases/tag/v1.2.0",
        "draft":false,"prerelease":false,"assets":[{"name":"control-1.2.0-x86_64.AppImage","size":5,"url":"https://api.github.com/repos/nildontsleep/control/releases/assets/9",
        "browser_download_url":"https://github.com/nildontsleep/control/releases/download/v1.2.0/control-1.2.0-x86_64.AppImage","digest":null,"extra":1}],"author":{}}"#;
    let rel: Release = serde_json::from_str(raw).unwrap();
    assert_eq!(rel.assets[0].size, 5);
    assert_eq!(rel.assets[0].digest, None);
    assert_eq!(api_error_code(404, false), "private");
    assert_eq!(api_error_code(404, true), "token");
    assert_eq!(api_error_code(401, false), "token");
    assert_eq!(api_error_code(403, false), "busy");
    assert_eq!(api_error_code(429, true), "token");
    assert_eq!(api_error_code(500, false), "http");
    assert_eq!(asset_sha256(Some(&format!("sha256:{}", "AB".repeat(32)))), Some("ab".repeat(32)));
    assert_eq!(asset_sha256(Some("sha256:abc")), None);
    assert_eq!(asset_sha256(Some(&format!("md5:{}", "a".repeat(64)))), None);
    assert_eq!(asset_sha256(None), None);
    assert_eq!(tag_from_location("https://github.com/nildontsleep/control/releases/tag/v1.3.0?x"), Some("v1.3.0".into()));
    assert_eq!(tag_from_location("https://github.com/login"), None);
}

#[test]
fn only_trusted_addresses() {
    assert!(trusted_url(&format!("https://github.com/{REPO}/releases/download/v1/control-1-setup.exe")));
    assert!(trusted_url(&format!("https://api.github.com/repos/{REPO}/releases/assets/123")));
    assert!(!trusted_url(&format!("https://api.github.com/repos/{REPO}/releases/assets/12a")));
    assert!(!trusted_url("https://github.com/someone/else/releases/download/v1/x.exe"));
    assert!(!trusted_url(&format!("http://github.com/{REPO}/releases/download/v1/x")));
    assert!(trusted_redirect("https://objects.githubusercontent.com/github-production-release-asset/1?sig=x"));
    assert!(trusted_redirect("https://release-assets.githubusercontent.com/x"));
    assert!(!trusted_redirect("http://objects.githubusercontent.com/x"));
    assert!(!trusted_redirect("https://evilgithub.com/x"));
    assert!(!trusted_redirect("https://github.com.evil.example/x"));
}

#[test]
fn evaluate_follows_the_state_machine() {
    let (state, pending) = evaluate(&release("v1.0.0"), "1.0.0", Edition::AppImage, false).unwrap();
    assert_eq!(state, json!({"phase": "none"}));
    assert!(pending.is_none());
    let mut pre = release("v2.0.0");
    pre.prerelease = true;
    assert_eq!(evaluate(&pre, "1.0.0", Edition::AppImage, false).unwrap().0["phase"], "none");

    let (state, pending) = evaluate(&release("v1.2.0"), "1.0.0", Edition::AppImage, false).unwrap();
    assert_eq!(
        state,
        json!({"phase": "available", "version": "1.2.0", "name": "v1.2.0", "notes": "Notes",
               "page": format!("https://github.com/{REPO}/releases/tag/v1.2.0"), "size": 100})
    );
    let p = pending.unwrap();
    assert!(!p.api);
    assert_eq!(p.file_name, "control-1.2.0-x86_64.AppImage");
    assert_eq!(p.digest, Some("ab".repeat(32)));

    // Avec un jeton : téléchargement par l'API.
    let (_, p) = evaluate(&release("v1.2.0"), "1.0.0", Edition::Setup, true).unwrap();
    assert!(p.unwrap().api);

    // Paquet .deb : on prévient seulement.
    let (state, p) = evaluate(&release("v1.2.0"), "1.0.0", Edition::Deb, false).unwrap();
    assert_eq!(state["manual"], true);
    assert_eq!(state["phase"], "available");
    assert!(state.get("size").is_none());
    assert!(p.is_none());

    // Fichier absent ou adresse douteuse.
    let mut empty = release("v1.2.0");
    empty.assets.clear();
    assert_eq!(evaluate(&empty, "1.0.0", Edition::Portable, false).unwrap_err().0, "asset");
    let mut evil = release("v1.2.0");
    for a in &mut evil.assets {
        a.browser_download_url = Some(format!("https://evil.example/{}", a.name));
    }
    if !test_override() {
        assert_eq!(evaluate(&evil, "1.0.0", Edition::Portable, false).unwrap_err().0, "untrusted");
    }
}

#[test]
fn stale_files() {
    assert!(stale_download("control-1.0.0-setup.exe", "1.0.0"));
    assert!(stale_download("control-0.9.0-x86_64.AppImage", "1.0.0"));
    assert!(!stale_download("control-1.1.0-portable.exe", "1.0.0"));
    assert!(stale_download("control-1.1.0-portable.exe.part", "1.0.0"));
    assert!(!stale_download("notes.txt", "1.0.0"));
    assert!(!stale_download("control-0.1.0-amd64.deb", "1.0.0"));
}

#[test]
fn install_plans() {
    let p = setup_plan(Path::new("/u/control-1.2.0-setup.exe"));
    assert_eq!(p.args, vec!["/S", "--updated", "--force-run"]);

    let p = appimage_plan(Path::new("/home/a/control.AppImage"), Path::new("/d/control-1.2.0-x86_64.AppImage"), 42);
    assert_eq!(p.command, "/bin/sh");
    assert_eq!(&p.args[2..], &["sh", "42", "/home/a/control.AppImage"]);
    assert!(p.args[1].contains("kill -0"));
    assert!(p.env_remove.contains(&"APPIMAGE".to_string()));
    assert_eq!(p.replace.as_ref().unwrap().1, PathBuf::from("/home/a/control.AppImage"));

    let p = portable_plan(
        Path::new(r"C:\Apps\control.exe"),
        Path::new(r"C:\Data\updates\control-1.2.0-portable.exe"),
        Path::new(r"C:\Apps\control-1.2.0-portable.exe"),
        Path::new(r"C:\Data\updates\apply-update.cmd"),
        "cmd.exe",
    );
    assert!(p.verbatim);
    assert_eq!(p.args[3], r#"""C:\Data\updates\apply-update.cmd"""#);
    let (_, script) = p.script.unwrap();
    assert!(script.starts_with("@echo off\r\n"));
    assert!(script.contains(r#"move /Y "C:\Data\updates\control-1.2.0-portable.exe" "C:\Apps\control-1.2.0-portable.exe""#));
    assert!(script.contains(r#"if exist "C:\Apps\control.exe" goto wait"#));
}

#[test]
fn appimage_replacement_runs() {
    if !cfg!(unix) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("control.AppImage");
    let source = dir.path().join("control-1.2.0-x86_64.AppImage");
    std::fs::write(&target, b"old").unwrap();
    std::fs::write(&source, b"new").unwrap();
    let mut plan = appimage_plan(&target, &source, 1);
    // Rien à relancer pendant le test : la commande ne fait que sortir.
    plan.command = "true".into();
    plan.args.clear();
    plan.run().unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
    assert!(!source.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o755);
    }
}

#[test]
fn states_keep_current_version_and_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let u = Updater::new(UpdaterOptions {
        version: "1.0.0".into(),
        edition: Edition::AppImage,
        dir: dir.path().to_path_buf(),
        emit: Arc::new(move |e| {
            if let Event::Json(STATE_CHANNEL, v) = e {
                sink.lock().push(v);
            }
        }),
        self_file: None,
        token: None,
    });
    assert_eq!(u.status(), json!({"phase": "idle", "current": "1.0.0"}));
    // Rien de prêt : ni téléchargement ni installation.
    assert_eq!(u.download(), json!({"phase": "idle", "current": "1.0.0"}));
    assert_eq!(u.install(), InstallOutcome::Stay);
    u.set(json!({"phase": "none", "checkedAt": 5}));
    assert_eq!(u.status()["current"], "1.0.0");
    assert_eq!(seen.lock().len(), 1);

    for name in ["control-0.9.0-x86_64.AppImage", "control-2.0.0-x86_64.AppImage", "control-2.0.0-setup.exe.part", "apply-update.cmd"] {
        std::fs::write(dir.path().join(name), b"x").unwrap();
    }
    u.cleanup();
    let mut left: Vec<String> =
        std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    assert_eq!(left, vec!["control-2.0.0-x86_64.AppImage".to_string()]);
}
