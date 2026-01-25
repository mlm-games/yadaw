use anyhow::{Result, anyhow};
use chrono::Local;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::constants::PROJECT_EXTENSION;
use crate::paths::cache_dir;
use crate::project::{AppState, Project};

#[derive(Debug, Clone)]
pub struct ProjectInfo {
    pub path: PathBuf,
    pub name: String,
    pub modified: SystemTime,
    pub auto_save_path: Option<PathBuf>,
}

pub struct ProjectManager {
    current_project: Option<ProjectInfo>,
    recent_projects: Vec<PathBuf>,
    max_recent: usize,

    auto_save_enabled: bool,
    auto_save_interval: Duration,
    last_auto_save: Instant,

    is_dirty: bool,
}

impl Default for ProjectManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectManager {
    pub fn new() -> Self {
        Self {
            current_project: None,
            recent_projects: Self::load_recent_projects(),
            max_recent: 10,
            auto_save_enabled: false,
            auto_save_interval: Duration::from_secs(300), // 5 minutes
            last_auto_save: Instant::now(),
            is_dirty: false,
        }
    }

    pub fn mark_dirty(&mut self) {
        self.is_dirty = true;
    }

    pub fn is_dirty(&self) -> bool {
        self.is_dirty
    }

    pub fn save_project(&mut self, state: &AppState, path: &Path) -> Result<()> {
        // Handle Backup if file exists
        if path.exists() {
            self.create_backup(path)?;
        }

        // Save actual project
        let project = Project::from(state);
        let json = serde_json::to_string_pretty(&project)?;
        fs::write(path, json)?;

        // Update state
        self.current_project = Some(ProjectInfo {
            path: path.to_path_buf(),
            name: path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled")
                .to_string(),
            modified: SystemTime::now(),
            auto_save_path: None,
        });

        self.add_to_recent(path);

        // Mark as clean after successful save
        self.is_dirty = false;

        Ok(())
    }

    /// Moves the current file at `path` to a `Backups/` subdirectory
    fn create_backup(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            let backup_dir = parent.join("Backups");
            if !backup_dir.exists() {
                fs::create_dir_all(&backup_dir)?;
            }

            let timestamp = Local::now().format("%Y%m%d_%H%M%S");
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("project");

            let backup_filename = format!("{}_{}.{}", stem, timestamp, PROJECT_EXTENSION);
            let backup_path = backup_dir.join(backup_filename);

            fs::copy(path, &backup_path)?;

            self.rotate_backups(&backup_dir, stem)?;
        }
        Ok(())
    }

    fn rotate_backups(&self, backup_dir: &Path, stem: &str) -> Result<()> {
        let mut backups = Vec::new();

        // Collect existing backups for this project
        if let Ok(entries) = fs::read_dir(backup_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with(stem) && name.ends_with(PROJECT_EXTENSION) {
                            backups.push(path);
                        }
                    }
                }
            }
        }

        // Sort by modification time (oldest first)
        backups.sort_by_key(|p| p.metadata().and_then(|m| m.modified()).ok());

        // Delete oldest if we have more than 10
        const MAX_BACKUPS: usize = 10;
        if backups.len() > MAX_BACKUPS {
            let to_remove = backups.len() - MAX_BACKUPS;
            for path in backups.iter().take(to_remove) {
                let _ = fs::remove_file(path);
            }
        }

        Ok(())
    }

    pub fn load_project(&mut self, path: &Path) -> Result<Project> {
        if !path.exists() {
            return Err(anyhow!("Project file does not exist"));
        }

        let contents = fs::read_to_string(path)?;
        let project: Project = serde_json::from_str(&contents)?;

        self.current_project = Some(ProjectInfo {
            path: path.to_path_buf(),
            name: project.name.clone(),
            modified: path.metadata()?.modified()?,
            auto_save_path: None,
        });

        self.add_to_recent(path);

        self.is_dirty = false;

        Ok(project)
    }

    pub fn auto_save(&mut self, state: &AppState) -> Result<()> {
        if !self.auto_save_enabled {
            return Ok(());
        }

        if self.last_auto_save.elapsed() < self.auto_save_interval {
            return Ok(());
        }

        let auto_save_path = self.get_auto_save_path()?;
        let project = state.to_project();

        let json = serde_json::to_string_pretty(&project)?;
        fs::write(&auto_save_path, json)?;

        if let Some(info) = &mut self.current_project {
            info.auto_save_path = Some(auto_save_path);
        }

        self.last_auto_save = Instant::now();
        Ok(())
    }

    pub fn recover_auto_save(&mut self) -> Result<Project> {
        let auto_save_path = self.get_auto_save_path()?;
        if !auto_save_path.exists() {
            return Err(anyhow!("No auto-save file found"));
        }

        let contents = fs::read_to_string(&auto_save_path)?;
        let project: Project = serde_json::from_str(&contents)?;

        // Clean up auto-save after recovery
        let _ = fs::remove_file(auto_save_path);

        Ok(project)
    }

    pub fn export_project(&self, state: &AppState, path: &Path, include_audio: bool) -> Result<()> {
        if include_audio {
            // Create a directory for the project bundle
            let bundle_dir = path.with_extension("");
            fs::create_dir_all(&bundle_dir)?;

            // Save project file
            let project_file = bundle_dir.join(format!(
                "{}.{}",
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("project"),
                PROJECT_EXTENSION
            ));

            let project = Project::from(state);
            let json = serde_json::to_string_pretty(&project)?;
            fs::write(&project_file, json)?;

            // Create audio directory and copy/export audio clips
            let audio_dir = bundle_dir.join("audio");
            fs::create_dir_all(&audio_dir)?;

            // TODO: Copy audio files to bundle (requires tracking original file paths)
        } else {
            // Regular save
            let project = Project::from(state);
            let json = serde_json::to_string_pretty(&project)?;
            fs::write(path, json)?;
        }

        Ok(())
    }

    fn create_backup_path(&self, original: &Path) -> PathBuf {
        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        let stem = original
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("project");

        original.with_file_name(format!(
            "{}_backup_{}.{}",
            stem, timestamp, PROJECT_EXTENSION
        ))
    }

    fn get_auto_save_path(&self) -> anyhow::Result<std::path::PathBuf> {
        let dir = cache_dir().join("autosave");
        std::fs::create_dir_all(&dir)?;
        let filename = if let Some(info) = &self.current_project {
            let safe_name = info
                .name
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>();
            format!("{}_autosave.{}", safe_name, PROJECT_EXTENSION)
        } else {
            format!("untitled_autosave.{}", PROJECT_EXTENSION)
        };
        Ok(dir.join(filename))
    }

    fn add_to_recent(&mut self, path: &Path) {
        let path_buf = path.to_path_buf();

        // Remove if already exists
        self.recent_projects.retain(|p| p != &path_buf);

        // Add to front
        self.recent_projects.insert(0, path_buf);

        // Trim to max
        self.recent_projects.truncate(self.max_recent);

        // Save recent projects list
        let _ = self.save_recent_projects();
    }

    pub fn mark_clean(&mut self) {
        self.is_dirty = false;
    }

    fn load_recent_projects() -> Vec<PathBuf> {
        if let Some(dirs) = directories::ProjectDirs::from("com", "yadaw", "yadaw") {
            let recent_file = dirs.config_dir().join("recent_projects.json");
            if recent_file.exists()
                && let Ok(contents) = fs::read_to_string(recent_file)
                && let Ok(recent) = serde_json::from_str(&contents)
            {
                return recent;
            }
        }
        Vec::new()
    }

    fn save_recent_projects(&self) -> Result<()> {
        if let Some(dirs) = directories::ProjectDirs::from("com", "yadaw", "yadaw") {
            let recent_file = dirs.config_dir().join("recent_projects.json");
            fs::create_dir_all(dirs.config_dir())?;
            let json = serde_json::to_string_pretty(&self.recent_projects)?;
            fs::write(recent_file, json)?;
        }
        Ok(())
    }

    pub fn get_recent_projects(&self) -> &[PathBuf] {
        &self.recent_projects
    }
    pub fn clear_recent_projects(&mut self) {
        self.recent_projects.clear();
        let _ = self.save_recent_projects();
    }
    pub fn get_current_project(&self) -> Option<&ProjectInfo> {
        self.current_project.as_ref()
    }
    pub fn set_auto_save(&mut self, enabled: bool) {
        self.auto_save_enabled = enabled;
    }
}
