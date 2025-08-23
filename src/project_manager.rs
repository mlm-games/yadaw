use crate::constants::PROJECT_EXTENSION;
use crate::state::{AppState, Project};
use anyhow::{anyhow, Result};
use chrono::Local;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ProjectInfo {
    pub path: PathBuf,
    pub name: String,
    pub modified: std::time::SystemTime,
    pub auto_save_path: Option<PathBuf>,
}

pub struct ProjectManager {
    current_project: Option<ProjectInfo>,
    recent_projects: Vec<PathBuf>,
    max_recent: usize,
    auto_save_enabled: bool,
    auto_save_interval: std::time::Duration,
    last_auto_save: std::time::Instant,
}

impl ProjectManager {
    pub fn new() -> Self {
        Self {
            current_project: None,
            recent_projects: Self::load_recent_projects(),
            max_recent: 10,
            auto_save_enabled: false,
            auto_save_interval: std::time::Duration::from_secs(300), // 5 minutes
            last_auto_save: std::time::Instant::now(),
        }
    }

    pub fn save_project(&mut self, state: &AppState, path: &Path) -> Result<()> {
        // Create backup if file exists
        if path.exists() {
            let backup_path = self.create_backup_path(path);
            fs::copy(path, backup_path)?;
        }

        // Save project
        let project = Project::from(state);
        let json = serde_json::to_string_pretty(&project)?;
        fs::write(path, json)?;

        // Update current project info
        self.current_project = Some(ProjectInfo {
            path: path.to_path_buf(),
            name: path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled")
                .to_string(),
            modified: std::time::SystemTime::now(),
            auto_save_path: None,
        });

        // Add to recent projects
        self.add_to_recent(path);

        Ok(())
    }

    pub fn load_project(&mut self, path: &Path) -> Result<Project> {
        if !path.exists() {
            return Err(anyhow!("Project file does not exist"));
        }

        let contents = fs::read_to_string(path)?;
        let project: Project = serde_json::from_str(&contents)?;

        // Update current project info
        self.current_project = Some(ProjectInfo {
            path: path.to_path_buf(),
            name: project.name.clone(),
            modified: path.metadata()?.modified()?,
            auto_save_path: None,
        });

        // Add to recent projects
        self.add_to_recent(path);

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

        self.last_auto_save = std::time::Instant::now();
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

            // TODO: Copy audio files to bundle
            // This would require tracking original audio file paths
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

    fn get_auto_save_path(&self) -> Result<PathBuf> {
        let auto_save_dir = directories::ProjectDirs::from("com", "yadaw", "yadaw")
            .ok_or_else(|| anyhow!("Cannot determine auto-save directory"))?
            .cache_dir()
            .join("autosave");

        fs::create_dir_all(&auto_save_dir)?;

        let filename = if let Some(info) = &self.current_project {
            format!("{}_autosave.{}", info.name, PROJECT_EXTENSION)
        } else {
            format!("untitled_autosave.{}", PROJECT_EXTENSION)
        };

        Ok(auto_save_dir.join(filename))
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

    fn load_recent_projects() -> Vec<PathBuf> {
        if let Some(dirs) = directories::ProjectDirs::from("com", "yadaw", "yadaw") {
            let recent_file = dirs.config_dir().join("recent_projects.json");
            if recent_file.exists() {
                if let Ok(contents) = fs::read_to_string(recent_file) {
                    if let Ok(recent) = serde_json::from_str(&contents) {
                        return recent;
                    }
                }
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

    pub fn has_unsaved_changes(&self) -> bool {
        // This would need to track if state has changed since last save
        false
    }

    pub fn set_auto_save(&mut self, enabled: bool) {
        self.auto_save_enabled = enabled;
    }
}
