use std::path::PathBuf;

#[cfg(target_os = "android")]
pub fn projects_dir() -> PathBuf {
    let base = std::path::Path::new("/data/data/com.yadaw.app/files");
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
    let p = std::path::Path::new("/data/data/com.yadaw.app/files/config");
    let _ = std::fs::create_dir_all(p);
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
    let p = std::path::Path::new("/data/data/com.yadaw.app/cache");
    let _ = std::fs::create_dir_all(p);
    p.to_path_buf()
}

#[cfg(not(target_os = "android"))]
pub fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "yadaw", "yadaw")
        .unwrap()
        .cache_dir()
        .to_path_buf()
}
