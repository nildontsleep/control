//! Installation d'une mise à jour : ce qu'il faut lancer (fonctions pures) et le lancement lui-même.
//! Le programme lancé attend la fin de control, met le nouveau fichier en place, puis relance control.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

/// Programme à lancer juste avant de quitter.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstallPlan {
    pub command: String,
    pub args: Vec<String>,
    /// Windows : arguments transmis tels quels, sans guillemets ajoutés.
    pub verbatim: bool,
    /// Variables à ne pas transmettre au nouveau programme.
    pub env_remove: Vec<String>,
    /// Script à écrire avant le lancement : (chemin, contenu).
    pub script: Option<(PathBuf, String)>,
    /// À faire avant le lancement : copier le fichier téléchargé (premier) à la place de l'exécutable (second).
    pub replace: Option<(PathBuf, PathBuf)>,
}

/// Installeur Windows : installation silencieuse, puis il relance control.
pub fn setup_plan(file: &Path) -> InstallPlan {
    InstallPlan {
        command: file.to_string_lossy().into_owned(),
        args: vec!["/S".into(), "--updated".into(), "--force-run".into()],
        verbatim: false,
        env_remove: Vec::new(),
        script: None,
        replace: None,
    }
}

fn q(p: &Path) -> String {
    format!("\"{}\"", p.to_string_lossy().replace('"', ""))
}

/// Script de l'édition portable : attend que control se ferme (l'exécutable reste verrouillé tant qu'il tourne),
/// remplace l'ancien exécutable par le nouveau (rangé à côté, au nom de la nouvelle version), puis le lance.
pub fn portable_script(old: &Path, file: &Path, dest: &Path) -> String {
    let (old, file, dest) = (q(old), q(file), q(dest));
    [
        "@echo off".to_string(),
        "chcp 65001 >nul".into(),
        "setlocal".into(),
        "set /a n=0".into(),
        ":wait".into(),
        "set /a n+=1".into(),
        format!("if %n% gtr 90 ( if exist {old} start \"\" {old} & exit /b 1 )"),
        "ping 127.0.0.1 -n 2 >nul".into(),
        format!("if exist {old} del /F /Q {old} >nul 2>&1"),
        format!("if exist {old} goto wait"),
        format!("move /Y {file} {dest} >nul 2>&1"),
        format!("if not exist {dest} exit /b 1"),
        format!("start \"\" {dest}"),
        "(goto) 2>nul & del \"%~f0\"".into(),
        String::new(),
    ]
    .join("\r\n")
}

/// Édition portable : le script ci-dessus, lancé par `cmd.exe`.
pub fn portable_plan(old: &Path, file: &Path, dest: &Path, script_path: &Path, comspec: &str) -> InstallPlan {
    InstallPlan {
        command: comspec.to_string(),
        args: vec!["/d".into(), "/s".into(), "/c".into(), format!("\"{}\"", q(script_path))],
        verbatim: true,
        env_remove: Vec::new(),
        script: Some((script_path.to_path_buf(), portable_script(old, file, dest))),
        replace: None,
    }
}

/// AppImage : le nouveau fichier prend la place de l'ancien (même nom, donc les raccourcis restent valables ;
/// Linux permet de remplacer un exécutable en cours d'utilisation). Un petit script attend la fin de control
/// (au plus 30 s) puis lance la nouvelle version, pour que le verrou d'instance unique soit libre.
pub fn appimage_plan(target: &Path, source: &Path, pid: u32) -> InstallPlan {
    let script = r#"pid=$1; shift; i=0; while kill -0 "$pid" 2>/dev/null && [ "$i" -lt 150 ]; do sleep 0.2; i=$((i+1)); done; exec "$@""#;
    InstallPlan {
        command: "/bin/sh".into(),
        args: vec!["-c".into(), script.into(), "sh".into(), pid.to_string(), target.to_string_lossy().into_owned()],
        verbatim: false,
        // Le nouveau programme ne doit pas hériter de l'environnement de l'ancien (points de montage de l'ancienne image).
        env_remove: ["APPIMAGE", "APPDIR", "ARGV0", "OWD"].iter().map(|s| s.to_string()).collect(),
        script: None,
        replace: Some((source.to_path_buf(), target.to_path_buf())),
    }
}

impl InstallPlan {
    /// Met le fichier en place si besoin, écrit le script, et lance le programme détaché de control.
    pub fn run(&self) -> std::io::Result<()> {
        if let Some((source, target)) = &self.replace {
            let mut staged = target.as_os_str().to_owned();
            staged.push(".new");
            let staged = PathBuf::from(staged);
            // Copie plutôt que déplacement : le dossier de destination peut être sur un autre disque.
            std::fs::copy(source, &staged)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
            }
            std::fs::rename(&staged, target)?;
            let _ = std::fs::remove_file(source);
        }
        if let Some((path, text)) = &self.script {
            std::fs::write(path, text)?;
        }
        let mut cmd = Command::new(&self.command);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            if self.verbatim {
                for a in &self.args {
                    cmd.raw_arg(a);
                }
            } else {
                cmd.args(&self.args);
            }
        }
        #[cfg(not(windows))]
        cmd.args(&self.args);
        for k in &self.env_remove {
            cmd.env_remove(k);
        }
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        crate::process::hide_window(&mut cmd);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        cmd.spawn().map(drop)
    }
}
