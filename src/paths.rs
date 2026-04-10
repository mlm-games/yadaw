use std::path::{Path, PathBuf};
#[cfg(not(target_os = "android"))]
use std::process::Command;

#[cfg(target_os = "android")]
pub fn projects_dir() -> PathBuf {
    let base = files_dir_pathbuf();
    let p = base.join("projects");
    let _ = std::fs::create_dir_all(&p);
    p
}

#[cfg(not(target_os = "android"))]
pub fn projects_dir() -> PathBuf {
    let dirs =
        directories::ProjectDirs::from("com", "yadaw", "yadaw").expect("ProjectDirs available");
    let p = dirs.data_dir().join("projects");
    let _ = std::fs::create_dir_all(&p);
    p
}

#[cfg(target_os = "android")]
pub fn config_path() -> PathBuf {
    let p = files_dir_pathbuf().join("config");
    let _ = std::fs::create_dir_all(&p);
    p.join("config.json")
}

#[cfg(not(target_os = "android"))]
pub fn config_path() -> PathBuf {
    directories::ProjectDirs::from("com", "yadaw", "yadaw")
        .unwrap()
        .config_dir()
        .join("config.json")
}

#[cfg(target_os = "android")]
pub fn cache_dir() -> PathBuf {
    let p = files_dir_pathbuf().join("cache");
    let _ = std::fs::create_dir_all(&p);
    p.to_path_buf()
}

#[cfg(not(target_os = "android"))]
pub fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "yadaw", "yadaw")
        .unwrap()
        .cache_dir()
        .to_path_buf()
}

#[cfg(target_os = "android")]
pub fn plugins_dir() -> PathBuf {
    let dir = files_dir_pathbuf().join("plugins").join("clap");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[cfg(not(target_os = "android"))]
pub fn plugins_dir() -> PathBuf {
    // Next to the executable: <exedir>/plugins/clap

    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    let base = exe.parent().unwrap_or(Path::new(".")).to_path_buf();
    let dir = base.join("plugins").join("clap");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[cfg(target_os = "android")]
pub fn presets_dir() -> PathBuf {
    let dir = files_dir_pathbuf().join("presets");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[cfg(not(target_os = "android"))]
pub fn presets_dir() -> PathBuf {
    if let Some(dirs) = directories::ProjectDirs::from("com", "yadaw", "yadaw") {
        let dir = dirs.config_dir().join("presets");
        let _ = std::fs::create_dir_all(&dir);
        dir
    } else {
        let dir = PathBuf::from("./presets");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }
}

#[cfg(target_os = "android")]
fn files_dir_pathbuf() -> PathBuf {
    use anyhow::Context;
    crate::android_saf::with_env(|env, context| {
        let file_obj = env
            .call_method(&context, "getFilesDir", "()Ljava/io/File;", &[])?
            .l()?;
        let jpath = env
            .call_method(&file_obj, "getAbsolutePath", "()Ljava/lang/String;", &[])?
            .l()?;
        let s: String = env.get_string(&jni::objects::JString::from(jpath))?.into();
        Ok(PathBuf::from(s))
    })
    .expect("getFilesDir failed")
}

#[cfg(not(target_os = "android"))]
pub fn config_root_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "yadaw", "yadaw")
        .unwrap()
        .config_dir()
        .to_path_buf()
}

#[cfg(target_os = "android")]
pub fn config_root_dir() -> PathBuf {
    files_dir_pathbuf().join("config")
}

pub fn shortcuts_path() -> PathBuf {
    let dir = config_root_dir();
    let _ = std::fs::create_dir_all(&dir);
    dir.join("shortcuts.json")
}

pub fn custom_themes_path() -> PathBuf {
    let dir = config_root_dir();
    let _ = std::fs::create_dir_all(&dir);
    dir.join("custom_themes.json")
}

pub fn current_theme_path() -> PathBuf {
    let dir = config_root_dir();
    let _ = std::fs::create_dir_all(&dir);
    dir.join("current_theme.json")
}

pub fn open_path_in_file_manager(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open").arg(path).spawn()?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).spawn()?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer").arg(path).spawn()?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Opening preset folders is not supported on this platform",
    ))
}
