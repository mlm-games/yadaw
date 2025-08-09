use crate::audio_state::AudioState;
use crate::automation_lane::{AutomationAction, AutomationLaneWidget};
use crate::config;
use crate::level_meter::LevelMeter;
use crate::lv2_plugin_host::PluginInfo;
use crate::piano_roll::{PianoRoll, PianoRollAction};
use crate::state::{
    AppState, AppStateSnapshot, AudioClip, AudioCommand, AutomationLane, AutomationTarget, Project,
    Track, UIUpdate,
};
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
enum TimelineInteraction {
    DragClip {
        clips_and_starts: Vec<(usize, usize, f64)>, // (track_id, clip_id, start_beat)
        start_drag_beat: f64,
    },
    ResizeClipLeft {
        track_id: usize,
        clip_id: usize,
        original_end_beat: f64, // start + length
    },
    ResizeClipRight {
        track_id: usize,
        clip_id: usize,
        original_start_beat: f64,
    },
}

pub struct YadawApp {
    state: Arc<Mutex<AppState>>,
    audio_state: Arc<AudioState>,
    command_tx: Sender<AudioCommand>,
    ui_rx: Receiver<UIUpdate>,
    config: config::Config,
    available_plugins: Vec<PluginInfo>,
    show_plugin_browser: bool,
    selected_track_for_plugin: Option<usize>,
    piano_roll: PianoRoll,
    selected_track: usize,
    selected_pattern: usize,
    project_path: Option<String>,
    show_save_dialog: bool,
    show_load_dialog: bool,
    timeline_zoom: f32,
    show_message: Option<String>,
    undo_stack: Vec<AppStateSnapshot>,
    redo_stack: Vec<AppStateSnapshot>,
    selected_clips: Vec<(usize, usize)>,
    show_clip_menu: bool,
    clip_menu_pos: egui::Pos2,
    show_mixer: bool,
    track_meters: Vec<LevelMeter>,
    clipboard: Option<Vec<AudioClip>>,
    show_rename_dialog: Option<(usize, usize)>, // (track_id, clip_id)
    rename_text: String,
    master_meter: LevelMeter,
    recording_level: f32,
    timeline_interaction: Option<TimelineInteraction>,
    automation_widgets: Vec<AutomationLaneWidget>,
    show_automation: bool,
    scroll_x: f32,
}

impl YadawApp {
    pub fn new(
        state: Arc<Mutex<AppState>>,
        audio_state: Arc<AudioState>,
        command_tx: Sender<AudioCommand>,
        ui_rx: Receiver<UIUpdate>,
        config: config::Config,
        available_plugins: Vec<PluginInfo>,
    ) -> Self {
        let num_tracks = {
            let state_lock = state.lock().unwrap();
            state_lock.tracks.len()
        };

        Self {
            state,
            audio_state,
            command_tx,
            ui_rx,
            config,
            available_plugins,
            show_plugin_browser: false,
            selected_track_for_plugin: None,
            piano_roll: PianoRoll::default(),
            selected_track: 1, // Start with MIDI track selected
            selected_pattern: 0,
            project_path: None,
            show_save_dialog: false,
            show_load_dialog: false,
            timeline_zoom: 100.0,
            show_message: None,
            undo_stack: vec![],
            redo_stack: vec![],
            selected_clips: vec![],
            show_clip_menu: false,
            clip_menu_pos: egui::Pos2::ZERO,
            show_mixer: false,
            track_meters: vec![LevelMeter::default(); num_tracks],
            clipboard: None,
            show_rename_dialog: None,
            rename_text: String::new(),
            master_meter: LevelMeter::default(),
            recording_level: 0.0,
            timeline_interaction: None,
            automation_widgets: vec![],
            show_automation: false,
            scroll_x: 0.0,
        }
    }

    fn push_undo(&mut self) {
        let state = self.state.lock().unwrap();
        self.undo_stack.push(state.snapshot());
        self.redo_stack.clear(); // Clear redo stack on new action

        // Limit undo stack size
        if self.undo_stack.len() > 100 {
            self.undo_stack.remove(0);
        }
    }

    fn undo(&mut self) {
        if let Some(snapshot) = self.undo_stack.pop() {
            let mut state = self.state.lock().unwrap();
            let current = state.snapshot();
            self.redo_stack.push(current);
            state.restore(snapshot);
        }
    }

    fn redo(&mut self) {
        if let Some(snapshot) = self.redo_stack.pop() {
            let mut state = self.state.lock().unwrap();
            let current = state.snapshot();
            self.undo_stack.push(current);
            state.restore(snapshot);
        }
    }

    fn cut_selected(&mut self) {
        self.copy_selected();
        self.delete_selected();
    }

    fn copy_selected(&mut self) {
        let mut clipboard = Vec::new();

        {
            let state = self.state.lock().unwrap();
            for (track_id, clip_id) in &self.selected_clips {
                if let Some(track) = state.tracks.get(*track_id) {
                    if let Some(clip) = track.audio_clips.get(*clip_id) {
                        clipboard.push(clip.clone());
                    }
                }
            }
        }

        self.clipboard = Some(clipboard);
    }

    fn paste_at_playhead(&mut self) {
        let clipboard = match &self.clipboard {
            Some(clips) => clips.clone(),
            None => return,
        };

        if clipboard.is_empty() {
            return;
        }

        self.push_undo();

        let current_beat = {
            let position = self.audio_state.get_position();
            let sample_rate = self.audio_state.sample_rate.load();
            let bpm = self.audio_state.bpm.load();
            (position / sample_rate as f64) * (bpm as f64 / 60.0)
        };

        let mut state = self.state.lock().unwrap();
        if let Some(track) = state.tracks.get_mut(self.selected_track) {
            if !track.is_midi {
                for clip in clipboard {
                    let mut new_clip = clip;
                    new_clip.start_beat = current_beat;
                    track.audio_clips.push(new_clip);
                }
            }
        }
    }

    fn delete_selected(&mut self) {
        if self.selected_clips.is_empty() {
            return;
        }

        self.push_undo();

        let mut state = self.state.lock().unwrap();

        // Sort clips by index in reverse order to delete from end to start
        let mut clips_to_delete = self.selected_clips.clone();
        clips_to_delete.sort_by(|a, b| b.1.cmp(&a.1));

        for (track_id, clip_id) in clips_to_delete {
            if let Some(track) = state.tracks.get_mut(track_id) {
                if clip_id < track.audio_clips.len() {
                    track.audio_clips.remove(clip_id);
                }
            }
        }

        self.selected_clips.clear();
    }

    fn split_selected_at_playhead(&mut self) {
        if self.selected_clips.is_empty() {
            return;
        }
        let selected_clips = self.selected_clips.clone();

        self.push_undo();

        let current_beat = {
            let position = self.audio_state.get_position();
            let sample_rate = self.audio_state.sample_rate.load();
            let bpm = self.audio_state.bpm.load();
            (position / sample_rate as f64) * (bpm as f64 / 60.0)
        };

        let mut state = self.state.lock().unwrap();
        let bpm = state.bpm;

        for (track_id, clip_id) in selected_clips {
            if let Some(track) = state.tracks.get_mut(track_id) {
                if let Some(clip) = track.audio_clips.get_mut(clip_id) {
                    // Check if playhead is within clip
                    if current_beat > clip.start_beat
                        && current_beat < clip.start_beat + clip.length_beats
                    {
                        // Calculate split point
                        let split_offset = current_beat - clip.start_beat;
                        let split_sample =
                            (split_offset * 60.0 / bpm as f64 * clip.sample_rate as f64) as usize;

                        if split_sample < clip.samples.len() {
                            // Create new clip from split point
                            let new_clip = AudioClip {
                                name: format!("{} (2)", clip.name),
                                start_beat: current_beat,
                                length_beats: clip.length_beats - split_offset,
                                samples: clip.samples[split_sample..].to_vec(),
                                sample_rate: clip.sample_rate,
                            };

                            clip.length_beats = split_offset;
                            clip.samples.truncate(split_sample);

                            track.audio_clips.push(new_clip);
                        }
                    }
                }
            }
        }
    }

    fn normalize_selected(&mut self) {
        if self.selected_clips.is_empty() {
            return;
        }

        self.push_undo();

        let mut state = self.state.lock().unwrap();

        for (track_id, clip_id) in &self.selected_clips {
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(clip) = track.audio_clips.get_mut(*clip_id) {
                    // Find peak
                    let peak = clip
                        .samples
                        .iter()
                        .map(|s| s.abs())
                        .fold(0.0f32, |a, b| a.max(b));

                    if peak > 0.0 {
                        // Normalize to -0.1 dB (0.989)
                        let gain = 0.989 / peak;
                        for sample in &mut clip.samples {
                            *sample *= gain;
                        }
                    }
                }
            }
        }
    }

    fn reverse_selected(&mut self) {
        if self.selected_clips.is_empty() {
            return;
        }

        self.push_undo();

        let mut state = self.state.lock().unwrap();

        for (track_id, clip_id) in &self.selected_clips {
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(clip) = track.audio_clips.get_mut(*clip_id) {
                    clip.samples.reverse();
                }
            }
        }
    }

    fn save_project(&self, path: &str) {
        let state = self.state.lock().unwrap();
        let project = Project::from(&*state);

        match std::fs::write(path, serde_json::to_string_pretty(&project).unwrap()) {
            Ok(_) => println!("Project saved to {}", path),
            Err(e) => eprintln!("Failed to save project: {}", e),
        }
    }

    fn load_project(&self, path: &str) {
        match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<Project>(&content) {
                Ok(project) => {
                    let mut state = self.state.lock().unwrap();
                    state.load_project(project);
                    println!("Project loaded from {}", path);
                }
                Err(e) => eprintln!("Failed to parse project: {}", e),
            },
            Err(e) => eprintln!("Failed to read project file: {}", e),
        }
    }

    fn draw_timeline_grid(&self, painter: &egui::Painter, rect: egui::Rect, bpm: f32) {
        // Vertical lines (beats)
        let beats_visible = (rect.width() / self.timeline_zoom) as i32 + 2;

        for beat in 0..beats_visible {
            let x = rect.left() + beat as f32 * self.timeline_zoom;

            if x >= rect.left() && x <= rect.right() {
                let color = if beat % 4 == 0 {
                    egui::Color32::from_gray(60)
                } else {
                    egui::Color32::from_gray(40)
                };

                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    egui::Stroke::new(1.0, color),
                );

                // Beat numbers
                if beat % 4 == 0 {
                    painter.text(
                        egui::pos2(x + 2.0, rect.top() + 2.0),
                        egui::Align2::LEFT_TOP,
                        format!("{}", beat / 4 + 1),
                        egui::FontId::default(),
                        egui::Color32::from_gray(100),
                    );
                }
            }
        }
    }
}

impl eframe::App for YadawApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle drag and drop
        ctx.input(|i| {
            if !i.raw.dropped_files.is_empty() {
                let dropped_files = i.raw.dropped_files.clone();
                let bpm = self.audio_state.bpm.load();

                self.push_undo();
                let mut state = self.state.lock().unwrap();

                for file in dropped_files {
                    // Handle both path and path_or_text cases
                    let path = if let Some(path) = &file.path {
                        Some(path.clone())
                    } else if let Some(bytes) = &file.bytes {
                        // Try to save bytes to temp file if path is not available
                        if !bytes.is_empty() {
                            let temp_dir = std::env::temp_dir();
                            let temp_path = temp_dir.join(format!(
                                "yadaw_import_{}.tmp",
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_millis()
                            ));
                            if std::fs::write(&temp_path, &**bytes).is_ok() {
                                Some(temp_path)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some(path) = path {
                        // Determine target track
                        let target_track_id = self.selected_track;

                        if let Some(track) = state.tracks.get_mut(target_track_id) {
                            if !track.is_midi {
                                match crate::audio_import::import_audio_file(&path, bpm) {
                                    Ok(mut clip) => {
                                        // Place clip at end of track
                                        let drop_beat = track
                                            .audio_clips
                                            .iter()
                                            .map(|c| c.start_beat + c.length_beats)
                                            .fold(0.0, f64::max);
                                        clip.start_beat = drop_beat;
                                        track.audio_clips.push(clip);
                                    }
                                    Err(e) => {
                                        self.show_message = Some(format!(
                                            "Failed to import {}: {}",
                                            path.display(),
                                            e
                                        ));
                                    }
                                }

                                // Clean up temp file if we created one
                                if path.starts_with(std::env::temp_dir()) {
                                    let _ = std::fs::remove_file(&path);
                                }
                            } else {
                                self.show_message =
                                    Some("Cannot drop audio on a MIDI track.".to_string());
                            }
                        }
                    }
                }
            }
        });

        // Process UI updates from audio thread
        while let Ok(update) = self.ui_rx.try_recv() {
            match update {
                UIUpdate::Position(pos) => {
                    if let Ok(state) = self.state.lock() {
                        let current_beat = state.position_to_beats(pos);
                        ctx.memory_mut(|mem| {
                            mem.data
                                .insert_temp(egui::Id::new("current_beat"), current_beat);
                        });
                    }
                }
                UIUpdate::RecordingFinished(track_id, clip) => {
                    let mut state = self.state.lock().unwrap();
                    if let Some(track) = state.tracks.get_mut(track_id) {
                        track.audio_clips.push(clip);
                    }
                }
                UIUpdate::TrackLevels(levels) => {
                    // Update track meters
                    if self.track_meters.len() < levels.len() {
                        self.track_meters
                            .resize_with(levels.len(), Default::default);
                    }

                    for (i, (left, right)) in levels.iter().enumerate() {
                        let samples = [left.max(*right)];
                        self.track_meters[i].update(&samples, 1.0 / 60.0);
                    }
                }
                UIUpdate::RecordingLevel(level) => {
                    self.recording_level = level;
                }
                UIUpdate::MasterLevel(left, right) => {
                    let samples = [left.max(right)];
                    self.master_meter.update(&samples, 1.0 / 60.0);
                }
                UIUpdate::PushUndo(snapshot) => {
                    self.undo_stack.push(snapshot);
                    self.redo_stack.clear();
                    if self.undo_stack.len() > 100 {
                        self.undo_stack.remove(0);
                    }
                }
                _ => {}
            }
        }

        // Plugin browser window
        let mut show_browser = self.show_plugin_browser;
        egui::Window::new("Plugin Browser")
            .open(&mut show_browser)
            .show(ctx, |ui| {
                ui.heading("Available Plugins");

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for plugin in &self.available_plugins {
                        if ui.button(&plugin.name).clicked() {
                            if let Some(track_id) = self.selected_track_for_plugin {
                                let _ = self
                                    .command_tx
                                    .send(AudioCommand::AddPlugin(track_id, plugin.uri.clone()));
                                self.show_plugin_browser = false;
                            }
                        }
                    }
                });
            });
        self.show_plugin_browser = show_browser;

        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New Project").clicked() {
                        let mut state = self.state.lock().unwrap();
                        *state = AppState::new();
                        self.project_path = None;
                        ui.close();
                    }

                    if ui.button("Open Project...").clicked() {
                        self.show_load_dialog = true;
                        ui.close();
                    }

                    if ui.button("Save Project").clicked() {
                        if let Some(path) = &self.project_path {
                            self.save_project(path);
                        } else {
                            self.show_save_dialog = true;
                        }
                        ui.close();
                    }

                    if ui.button("Save Project As...").clicked() {
                        self.show_save_dialog = true;
                        ui.close();
                    }

                    ui.separator();

                    if ui.button("Import Audio...").clicked() {
                        let bpm = self.audio_state.bpm.load();
                        ui.close();

                        if let Some(paths) = rfd::FileDialog::new()
                            .add_filter("Audio Files", &["wav", "mp3", "flac", "ogg"])
                            .add_filter("All Files", &["*"])
                            .pick_files()
                        {
                            for path in paths {
                                match crate::audio_import::import_audio_file(&path, bpm) {
                                    Ok(clip) => {
                                        let mut state = self.state.lock().unwrap();
                                        if let Some(track) =
                                            state.tracks.get_mut(self.selected_track)
                                        {
                                            if !track.is_midi {
                                                track.audio_clips.push(clip);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        self.show_message = Some(format!(
                                            "Failed to import {}: {}",
                                            path.display(),
                                            e
                                        ));
                                    }
                                }
                            }
                        }
                    }

                    ui.separator();

                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("Edit", |ui| {
                    if ui.button("Undo").clicked() {
                        self.undo();
                        ui.close();
                    }

                    if ui.button("Redo").clicked() {
                        self.redo();
                        ui.close();
                    }

                    ui.separator();

                    if ui.button("Cut").clicked() {
                        self.cut_selected();
                        ui.close();
                    }

                    if ui.button("Copy").clicked() {
                        self.copy_selected();
                        ui.close();
                    }

                    if ui.button("Paste").clicked() {
                        self.paste_at_playhead();
                        ui.close();
                    }

                    if ui.button("Delete").clicked() {
                        self.delete_selected();
                        ui.close();
                    }
                });

                ui.menu_button("View", |ui| {
                    if ui.checkbox(&mut self.show_mixer, "Mixer").clicked() {
                        ui.close();
                    }
                });

                ui.menu_button("Preferences", |ui| {
                    if ui.button("Audio Settings...").clicked() {
                        // TODO: Show audio settings dialog
                        ui.close();
                    }
                    if ui.button("Save Preferences").clicked() {
                        if let Err(e) = self.config.save() {
                            self.show_message = Some(format!("Failed to save preferences: {}", e));
                        } else {
                            self.show_message = Some("Preferences saved".to_string());
                        }
                        ui.close();
                    }
                });
            });
        });

        // File dialogs
        if self.show_save_dialog {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("YADAW Project", &["yadaw"])
                .add_filter("All Files", &["*"])
                .set_file_name("untitled.yadaw")
                .save_file()
            {
                self.save_project(path.to_str().unwrap());
                self.project_path = Some(path.to_str().unwrap().to_string());
            }
            self.show_save_dialog = false;
        }

        if self.show_load_dialog {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("YADAW Project", &["yadaw"])
                .add_filter("All Files", &["*"])
                .pick_file()
            {
                self.load_project(path.to_str().unwrap());
                self.project_path = Some(path.to_str().unwrap().to_string());
            }
            self.show_load_dialog = false;
        }

        if let Some(message) = &self.show_message.clone() {
            egui::Window::new("Message")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(message);
                    if ui.button("OK").clicked() {
                        self.show_message = None;
                    }
                });
        }

        // Top panel - Transport controls
        egui::TopBottomPanel::top("transport").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Read from audio state for real-time values
                let is_playing = self.audio_state.playing.load(Ordering::Relaxed);
                let is_recording = self.audio_state.recording.load(Ordering::Relaxed);
                let position = self.audio_state.get_position();
                let sample_rate = self.audio_state.sample_rate.load();
                let bpm = self.audio_state.bpm.load();
                let master_volume = self.audio_state.master_volume.load();

                if ui.button(if is_playing { "⏸" } else { "▶" }).clicked() {
                    if is_playing {
                        let _ = self.command_tx.send(AudioCommand::Stop);
                    } else {
                        let _ = self.command_tx.send(AudioCommand::Play);
                    }
                }

                if ui.button("⏹").clicked() {
                    let _ = self.command_tx.send(AudioCommand::Stop);
                }

                if ui
                    .button(if is_recording { "⏺ Recording" } else { "⏺" })
                    .on_hover_text("Record")
                    .clicked()
                {
                    if is_recording {
                        let _ = self.command_tx.send(AudioCommand::StopRecording);
                    } else {
                        let state = self.state.lock().unwrap();
                        let armed_track = state.tracks.iter().position(|t| t.armed && !t.is_midi);
                        drop(state);

                        if let Some(track_id) = armed_track {
                            let _ = self.command_tx.send(AudioCommand::StartRecording(track_id));
                        } else {
                            self.show_message =
                                Some("Please arm an audio track for recording".to_string());
                        }
                    }
                }

                ui.separator();

                // Time display
                let time_seconds = position / sample_rate as f64;
                let minutes = (time_seconds / 60.0) as i32;
                let seconds = time_seconds % 60.0;
                ui.label(format!("{:02}:{:05.2}", minutes, seconds));

                ui.separator();
                ui.label(format!("BPM: {:.1}", bpm));

                ui.separator();
                ui.label("Master Vol:");
                let mut master_vol = master_volume;
                if ui
                    .add(egui::Slider::new(&mut master_vol, 0.0..=1.0).show_value(false))
                    .changed()
                {
                    self.audio_state.master_volume.store(master_vol);
                    // Also update app state for persistence
                    if let Ok(mut state) = self.state.lock() {
                        state.master_volume = master_vol;
                    }
                }

                if is_recording {
                    ui.separator();
                    ui.colored_label(egui::Color32::RED, "● REC");

                    let level_db = 20.0 * self.recording_level.max(0.0001).log10();
                    let normalized = (level_db + 60.0) / 60.0;

                    let (response, painter) =
                        ui.allocate_painter(egui::vec2(100.0, 10.0), egui::Sense::hover());
                    let rect = response.rect;
                    painter.rect_filled(rect, 2.0, egui::Color32::from_gray(40));

                    let level_rect = egui::Rect::from_min_size(
                        rect.min,
                        egui::vec2(rect.width() * normalized.clamp(0.0, 1.0), rect.height()),
                    );

                    let color = if level_db > -3.0 {
                        egui::Color32::RED
                    } else if level_db > -12.0 {
                        egui::Color32::YELLOW
                    } else {
                        egui::Color32::GREEN
                    };

                    painter.rect_filled(level_rect, 2.0, color);
                    ui.label(format!("{:.1} dB", level_db));
                }
            });
        });

        // Left panel - Track controls
        egui::SidePanel::left("tracks")
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.heading("Tracks");

                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut commands_to_send = Vec::new();
                    let mut add_track_clicked = false;

                    {
                        let mut state = self.state.lock().unwrap();
                        let num_tracks = state.tracks.len();

                        for track_idx in 0..num_tracks {
                            let is_selected = track_idx == self.selected_track;

                            ui.group(|ui| {
                                let track = &mut state.tracks[track_idx];

                                ui.horizontal(|ui| {
                                    if ui.selectable_label(is_selected, &track.name).clicked() {
                                        self.selected_track = track_idx;
                                    }
                                    ui.label(if track.is_midi { "🎹" } else { "🎵" });

                                    if ui
                                        .small_button(if track.muted { "🔇" } else { "🔊" })
                                        .clicked()
                                    {
                                        let muted = !track.muted;
                                        commands_to_send
                                            .push(AudioCommand::MuteTrack(track_idx, muted));
                                    }

                                    if ui
                                        .small_button(if track.solo { "S" } else { "s" })
                                        .on_hover_text("Solo")
                                        .clicked()
                                    {
                                        let solo = !track.solo;
                                        commands_to_send
                                            .push(AudioCommand::SoloTrack(track_idx, solo));
                                    }

                                    if ui
                                        .small_button(if track.armed { "🔴" } else { "⭕" })
                                        .on_hover_text("Record Arm")
                                        .clicked()
                                    {
                                        track.armed = !track.armed;
                                    }
                                });

                                ui.horizontal(|ui| {
                                    ui.label("Vol:");
                                    let mut volume = track.volume;
                                    if ui
                                        .add(
                                            egui::Slider::new(&mut volume, 0.0..=1.0)
                                                .show_value(false),
                                        )
                                        .changed()
                                    {
                                        commands_to_send
                                            .push(AudioCommand::SetTrackVolume(track_idx, volume));
                                    }
                                    ui.label(format!("{:.0}%", volume * 100.0));
                                });

                                ui.horizontal(|ui| {
                                    ui.label("Pan:");
                                    let mut pan = track.pan;
                                    if ui
                                        .add(
                                            egui::Slider::new(&mut pan, -1.0..=1.0)
                                                .show_value(false),
                                        )
                                        .changed()
                                    {
                                        commands_to_send
                                            .push(AudioCommand::SetTrackPan(track_idx, pan));
                                    }
                                    let pan_text = if pan.abs() < 0.01 {
                                        "C".to_string()
                                    } else if pan < 0.0 {
                                        format!("L{:.0}", -pan * 100.0)
                                    } else {
                                        format!("R{:.0}", pan * 100.0)
                                    };
                                    ui.label(pan_text);
                                });

                                // Plugin chain
                                ui.separator();
                                ui.horizontal(|ui| {
                                    ui.label("Plugins:");
                                    if ui.small_button("+").clicked() {
                                        self.show_plugin_browser = true;
                                        self.selected_track_for_plugin = Some(track_idx);
                                    }
                                });

                                ui.menu_button("Automate", |ui| {
                                    if ui.button("Volume").clicked() {
                                        // Create volume automation lane
                                        let _ =
                                            self.command_tx.send(AudioCommand::AddAutomationPoint(
                                                track_idx,
                                                AutomationTarget::TrackVolume,
                                                0.0,
                                                track.volume,
                                            ));
                                        ui.close();
                                    }

                                    if ui.button("Pan").clicked() {
                                        // Create pan automation lane
                                        let _ =
                                            self.command_tx.send(AudioCommand::AddAutomationPoint(
                                                track_idx,
                                                AutomationTarget::TrackPan,
                                                0.0,
                                                (track.pan + 1.0) / 2.0,
                                            ));
                                        ui.close();
                                    }

                                    ui.separator();

                                    // Plugin parameters
                                    for (plugin_idx, plugin) in
                                        track.plugin_chain.iter().enumerate()
                                    {
                                        ui.menu_button(&plugin.name, |ui| {
                                            for (param_name, param) in &plugin.params {
                                                if ui.button(&param.name).clicked() {
                                                    let normalized = (param.value - param.min)
                                                        / (param.max - param.min);
                                                    let _ = self.command_tx.send(
                                                        AudioCommand::AddAutomationPoint(
                                                            track_idx,
                                                            AutomationTarget::PluginParam {
                                                                plugin_idx,
                                                                param_name: param_name.clone(),
                                                            },
                                                            0.0,
                                                            normalized,
                                                        ),
                                                    );
                                                    ui.close();
                                                }
                                            }
                                        });
                                    }
                                });

                                if !track.automation_lanes.is_empty() {
                                    ui.separator();
                                    ui.label(format!(
                                        "🎛️ {} automation lanes",
                                        track.automation_lanes.len()
                                    ));
                                }

                                let mut plugin_to_remove = None;
                                for (j, plugin) in track.plugin_chain.iter().enumerate() {
                                    ui.collapsing(&plugin.name, |ui| {
                                        ui.horizontal(|ui| {
                                            // Bypass toggle
                                            let mut bypass = plugin.bypass;
                                            if ui.checkbox(&mut bypass, "Bypass").changed() {
                                                commands_to_send.push(
                                                    AudioCommand::SetPluginBypass(
                                                        track_idx, j, bypass,
                                                    ),
                                                );
                                            }

                                            // Remove button
                                            if ui.small_button("×").clicked() {
                                                plugin_to_remove = Some(j);
                                            }
                                        });

                                        // Parameter controls
                                        for (param_name, param) in &plugin.params {
                                            ui.horizontal(|ui| {
                                                ui.label(&param.name);
                                                let mut value = param.value;

                                                // Use appropriate widget based on parameter range
                                                if param.max - param.min <= 1.0 && param.min == 0.0
                                                {
                                                    // Likely a toggle
                                                    let mut enabled = value > 0.5;
                                                    if ui.checkbox(&mut enabled, "").changed() {
                                                        value = if enabled { 1.0 } else { 0.0 };
                                                        commands_to_send.push(
                                                            AudioCommand::SetPluginParam(
                                                                track_idx,
                                                                j,
                                                                param_name.clone(),
                                                                value,
                                                            ),
                                                        );
                                                    }
                                                } else {
                                                    // Slider
                                                    if ui
                                                        .add(
                                                            egui::Slider::new(
                                                                &mut value,
                                                                param.min..=param.max,
                                                            )
                                                            .show_value(true),
                                                        )
                                                        .changed()
                                                    {
                                                        commands_to_send.push(
                                                            AudioCommand::SetPluginParam(
                                                                track_idx,
                                                                j,
                                                                param_name.clone(),
                                                                value,
                                                            ),
                                                        );
                                                    }
                                                }

                                                // Reset button
                                                if ui
                                                    .small_button("↺")
                                                    .on_hover_text("Reset to default")
                                                    .clicked()
                                                {
                                                    commands_to_send.push(
                                                        AudioCommand::SetPluginParam(
                                                            track_idx,
                                                            j,
                                                            param_name.clone(),
                                                            param.default,
                                                        ),
                                                    );
                                                }
                                            });
                                        }
                                    });
                                }

                                if let Some(j) = plugin_to_remove {
                                    commands_to_send.push(AudioCommand::RemovePlugin(track_idx, j));
                                }
                            });
                            ui.add_space(5.0);
                        }

                        if ui.button("➕ Add Track").clicked() {
                            add_track_clicked = true;
                        }
                    }

                    if add_track_clicked {
                        let mut state = self.state.lock().unwrap();
                        let new_track_id = state.tracks.len();
                        state.tracks.push(Track {
                            id: new_track_id,
                            name: format!("Track {}", new_track_id + 1),
                            volume: 0.7,
                            pan: 0.0,
                            muted: false,
                            solo: false,
                            armed: false,
                            plugin_chain: vec![],
                            patterns: vec![],
                            is_midi: false,
                            audio_clips: vec![],
                            automation_lanes: vec![],
                        });
                        self.track_meters.push(LevelMeter::default());
                    }

                    // Send commands after releasing the lock
                    for cmd in commands_to_send {
                        let _ = self.command_tx.send(cmd);
                    }
                });
            });

        // Central panel - Timeline/Arrangement
        egui::CentralPanel::default().show(ctx, |ui| {
            // Check if selected track is MIDI
            let is_midi_track = {
                let state = self.state.lock().unwrap();
                state
                    .tracks
                    .get(self.selected_track)
                    .map(|t| t.is_midi)
                    .unwrap_or(false)
            };

            if is_midi_track {
                // Show piano roll for MIDI tracks
                ui.heading("Piano Roll");

                // Pattern selector
                ui.horizontal(|ui| {
                    ui.label("Pattern:");
                    let state = self.state.lock().unwrap();
                    if let Some(track) = state.tracks.get(self.selected_track) {
                        for (i, pattern) in track.patterns.iter().enumerate() {
                            if ui
                                .selectable_label(i == self.selected_pattern, &pattern.name)
                                .clicked()
                            {
                                self.selected_pattern = i;
                            }
                        }
                    }
                });

                ui.separator();

                // Piano roll editor
                let mut pattern_actions = Vec::new();

                {
                    let state = self.state.lock().unwrap();
                    if state.playing {
                        let current_beat = state.position_to_beats(self.audio_state.get_position());
                        let pattern_beat = current_beat
                            % state
                                .tracks
                                .get(self.selected_track)
                                .and_then(|t| t.patterns.first())
                                .map(|p| p.length)
                                .unwrap_or(4.0);
                        ctx.memory_mut(|mem| {
                            mem.data
                                .insert_temp(egui::Id::new("current_beat"), pattern_beat)
                        });
                    }
                }

                {
                    let mut state = self.state.lock().unwrap();
                    if let Some(track) = state.tracks.get_mut(self.selected_track) {
                        if let Some(pattern) = track.patterns.get_mut(self.selected_pattern) {
                            pattern_actions = self.piano_roll.ui(ui, pattern);
                        }
                    }
                }

                // Process piano roll actions
                for action in pattern_actions {
                    match action {
                        PianoRollAction::AddNote(note) => {
                            let _ = self.command_tx.send(AudioCommand::AddNote(
                                self.selected_track,
                                self.selected_pattern,
                                note,
                            ));
                        }
                        PianoRollAction::RemoveNote(index) => {
                            let _ = self.command_tx.send(AudioCommand::RemoveNote(
                                self.selected_track,
                                self.selected_pattern,
                                index,
                            ));
                        }
                        PianoRollAction::UpdateNote(index, note) => {
                            let _ = self.command_tx.send(AudioCommand::UpdateNote(
                                self.selected_track,
                                self.selected_pattern,
                                index,
                                note,
                            ));
                        }
                        PianoRollAction::PreviewNote(pitch) => {
                            let _ = self
                                .command_tx
                                .send(AudioCommand::PreviewNote(self.selected_track, pitch));
                        }
                        PianoRollAction::StopPreview => {
                            let _ = self.command_tx.send(AudioCommand::StopPreviewNote);
                        }
                    }
                }
            } else {
                // Show timeline for audio tracks
                ui.heading("Timeline");

                let (is_playing, current_position, sample_rate, bpm) = (
                    self.audio_state.playing.load(Ordering::Relaxed),
                    self.audio_state.get_position(),
                    self.audio_state.sample_rate.load(),
                    self.audio_state.bpm.load(),
                );

                // Timeline controls
                ui.horizontal(|ui| {
                    ui.label("Zoom:");
                    if ui.button("-").clicked() {
                        self.timeline_zoom *= 0.8;
                    }
                    if ui.button("+").clicked() {
                        self.timeline_zoom *= 1.25;
                    }
                    ui.label(format!("{:.0} pixels/beat", self.timeline_zoom));
                });

                if ui
                    .checkbox(&mut self.show_automation, "Show Automation")
                    .clicked()
                {}

                ui.separator();

                // Timeline scroll area
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let base_track_height = 80.0;

                        // Calculate track heights including automation lanes
                        let track_heights: Vec<f32> = {
                            let state = self.state.lock().unwrap();
                            state
                                .tracks
                                .iter()
                                .map(|track| {
                                    let visible_lanes = if self.show_automation {
                                        track.automation_lanes.iter().filter(|l| l.visible).count()
                                    } else {
                                        0
                                    };
                                    base_track_height + (visible_lanes as f32 * 30.0)
                                })
                                .collect()
                        };

                        // Allocate space for all tracks
                        let total_height =
                            self.state.lock().unwrap().tracks.len() as f32 * base_track_height;
                        let (response, painter) = ui.allocate_painter(
                            egui::vec2(
                                ui.available_width(),
                                total_height.max(ui.available_height()),
                            ),
                            egui::Sense::hover(),
                        );
                        let timeline_rect = response.rect;

                        // Draw grid
                        self.draw_timeline_grid(&painter, timeline_rect, bpm);

                        // First, collect automation data BEFORE the main drawing loop
                        let automation_data: Vec<(usize, Vec<(usize, AutomationLane)>)> = {
                            let state = self.state.lock().unwrap();
                            state
                                .tracks
                                .iter()
                                .enumerate()
                                .map(|(track_idx, track)| {
                                    let lanes: Vec<(usize, AutomationLane)> = track
                                        .automation_lanes
                                        .iter()
                                        .enumerate()
                                        .map(|(idx, lane)| (idx, lane.clone()))
                                        .collect();
                                    (track_idx, lanes)
                                })
                                .collect()
                        };

                        // Draw tracks and clips
                        {
                            let mut state = self.state.lock().unwrap();
                            let tracks_len = state.tracks.len();

                            for track_idx in 0..tracks_len {
                                let track_rect = egui::Rect::from_min_size(
                                    timeline_rect.min
                                        + egui::vec2(0.0, track_idx as f32 * base_track_height),
                                    egui::vec2(timeline_rect.width(), base_track_height),
                                );

                                // Get track info
                                let (track_name, _is_midi, clips_count) = {
                                    let track = &state.tracks[track_idx];
                                    (track.name.clone(), track.is_midi, track.audio_clips.len())
                                };

                                // Track background
                                painter.rect_filled(
                                    track_rect,
                                    0.0,
                                    if track_idx % 2 == 0 {
                                        egui::Color32::from_gray(25)
                                    } else {
                                        egui::Color32::from_gray(30)
                                    },
                                );

                                // Track name
                                painter.text(
                                    track_rect.min + egui::vec2(5.0, 5.0),
                                    egui::Align2::LEFT_TOP,
                                    &track_name,
                                    egui::FontId::default(),
                                    egui::Color32::WHITE,
                                );

                                // Draw audio clips
                                for clip_idx in 0..clips_count {
                                    let clip = &state.tracks[track_idx].audio_clips[clip_idx];
                                    let clip_x = clip.start_beat as f32 * self.timeline_zoom;
                                    let clip_width = clip.length_beats as f32 * self.timeline_zoom;

                                    let clip_rect = egui::Rect::from_min_size(
                                        track_rect.min + egui::vec2(clip_x, 20.0),
                                        egui::vec2(clip_width, base_track_height - 25.0),
                                    );

                                    // Draw waveform and border
                                    crate::waveform::draw_waveform(
                                        &painter,
                                        clip_rect,
                                        clip,
                                        self.timeline_zoom,
                                        0.0,
                                    );

                                    if self.selected_clips.contains(&(track_idx, clip_idx)) {
                                        painter.rect_stroke(
                                            clip_rect,
                                            2.0,
                                            egui::Stroke::new(2.0, egui::Color32::WHITE),
                                            egui::StrokeKind::Inside,
                                        );
                                    }

                                    let clip_response = ui.interact(
                                        clip_rect,
                                        ui.id().with((track_idx, clip_idx)),
                                        egui::Sense::click_and_drag(),
                                    );

                                    if clip_response.secondary_clicked() {
                                        self.selected_clips = vec![(track_idx, clip_idx)];
                                        self.show_clip_menu = true;
                                        self.clip_menu_pos = clip_response
                                            .interact_pointer_pos()
                                            .unwrap_or_default();
                                    }
                                    if clip_response.clicked() {
                                        self.selected_clips.clear();
                                        self.selected_clips.push((track_idx, clip_idx));
                                    }

                                    let edge_threshold = 5.0;
                                    let hover_left_edge =
                                        clip_response.hover_pos().map_or(false, |p| {
                                            (p.x - clip_rect.left()).abs() < edge_threshold
                                        });
                                    let hover_right_edge =
                                        clip_response.hover_pos().map_or(false, |p| {
                                            (clip_rect.right() - p.x).abs() < edge_threshold
                                        });

                                    if hover_left_edge || hover_right_edge {
                                        ui.ctx()
                                            .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                                    }

                                    if clip_response.drag_started()
                                        && self.timeline_interaction.is_none()
                                    {
                                        // self.push_undo();
                                        let mouse_beat =
                                            clip_response.interact_pointer_pos().map_or(0.0, |p| {
                                                (p.x - timeline_rect.left()) / self.timeline_zoom
                                            }) as f64;

                                        let interaction = if hover_left_edge {
                                            TimelineInteraction::ResizeClipLeft {
                                                track_id: track_idx,
                                                clip_id: clip_idx,
                                                original_end_beat: clip.start_beat
                                                    + clip.length_beats,
                                            }
                                        } else if hover_right_edge {
                                            TimelineInteraction::ResizeClipRight {
                                                track_id: track_idx,
                                                clip_id: clip_idx,
                                                original_start_beat: clip.start_beat,
                                            }
                                        } else {
                                            let clips_and_starts = if self
                                                .selected_clips
                                                .contains(&(track_idx, clip_idx))
                                            {
                                                self.selected_clips
                                                    .iter()
                                                    .map(|(tid, cid)| {
                                                        let c =
                                                            &state.tracks[*tid].audio_clips[*cid];
                                                        (*tid, *cid, c.start_beat)
                                                    })
                                                    .collect()
                                            } else {
                                                vec![(track_idx, clip_idx, clip.start_beat)]
                                            };

                                            TimelineInteraction::DragClip {
                                                clips_and_starts,
                                                start_drag_beat: mouse_beat,
                                            }
                                        };
                                        self.timeline_interaction = Some(interaction);
                                    }
                                }
                            }

                            // Handle timeline interaction
                            if let (Some(interaction), Some(pointer_pos)) = (
                                self.timeline_interaction.clone(),
                                ui.ctx().pointer_interact_pos(),
                            ) {
                                let mouse_beat = ((pointer_pos.x - timeline_rect.left())
                                    / self.timeline_zoom)
                                    as f64;

                                match interaction {
                                    TimelineInteraction::DragClip {
                                        clips_and_starts,
                                        start_drag_beat,
                                    } => {
                                        let beat_delta = mouse_beat - start_drag_beat;
                                        for (track_id, clip_id, original_start) in clips_and_starts
                                        {
                                            if let Some(track) = state.tracks.get_mut(track_id) {
                                                if let Some(clip) =
                                                    track.audio_clips.get_mut(clip_id)
                                                {
                                                    clip.start_beat =
                                                        (original_start + beat_delta).max(0.0);
                                                }
                                            }
                                        }
                                    }
                                    TimelineInteraction::ResizeClipLeft {
                                        track_id,
                                        clip_id,
                                        original_end_beat,
                                    } => {
                                        if let Some(track) = state.tracks.get_mut(track_id) {
                                            if let Some(clip) = track.audio_clips.get_mut(clip_id) {
                                                let new_start = mouse_beat
                                                    .max(0.0)
                                                    .min(original_end_beat - 0.1);
                                                let new_len = original_end_beat - new_start;
                                                clip.start_beat = new_start;
                                                clip.length_beats = new_len;
                                            }
                                        }
                                    }
                                    TimelineInteraction::ResizeClipRight {
                                        track_id,
                                        clip_id,
                                        original_start_beat,
                                    } => {
                                        if let Some(track) = state.tracks.get_mut(track_id) {
                                            if let Some(clip) = track.audio_clips.get_mut(clip_id) {
                                                let new_len =
                                                    (mouse_beat - original_start_beat).max(0.1);
                                                clip.length_beats = new_len;
                                            }
                                        }
                                    }
                                }
                            }

                            if ui.input(|i| i.pointer.primary_released()) {
                                self.timeline_interaction = None;
                            }
                        }

                        if self.show_automation {
                            let mut widget_index = 0;
                            for (track_idx, lanes) in automation_data {
                                let track_rect = egui::Rect::from_min_size(
                                    timeline_rect.min
                                        + egui::vec2(0.0, track_idx as f32 * base_track_height),
                                    egui::vec2(timeline_rect.width(), base_track_height),
                                );

                                for (lane_idx, mut lane) in lanes {
                                    if lane.visible {
                                        let lane_rect = egui::Rect::from_min_size(
                                            track_rect.min
                                                + egui::vec2(
                                                    0.0,
                                                    base_track_height
                                                        - 30.0
                                                        - (lane_idx as f32 * 30.0),
                                                ), // Stack lanes
                                            egui::vec2(timeline_rect.width(), 30.0),
                                        );

                                        // Ensure we have enough widgets
                                        while self.automation_widgets.len() <= widget_index {
                                            self.automation_widgets
                                                .push(AutomationLaneWidget::default());
                                        }

                                        let actions = self.automation_widgets[lane_idx].ui(
                                            ui,
                                            &mut lane,
                                            lane_rect,
                                            self.timeline_zoom,
                                            self.scroll_x,
                                        );

                                        // Process automation actions
                                        for action in actions {
                                            match action {
                                                AutomationAction::AddPoint { beat, value } => {
                                                    self.push_undo();
                                                    let _ = self.command_tx.send(
                                                        AudioCommand::AddAutomationPoint(
                                                            track_idx,
                                                            lane.parameter.clone(),
                                                            beat,
                                                            value,
                                                        ),
                                                    );
                                                }
                                                AutomationAction::RemovePoint(beat) => {
                                                    self.push_undo();
                                                    let _ = self.command_tx.send(
                                                        AudioCommand::RemoveAutomationPoint(
                                                            track_idx, lane_idx, beat,
                                                        ),
                                                    );
                                                }
                                                AutomationAction::MovePoint {
                                                    old_beat,
                                                    new_beat,
                                                    new_value,
                                                } => {
                                                    self.push_undo();
                                                    let _ = self.command_tx.send(
                                                        AudioCommand::RemoveAutomationPoint(
                                                            track_idx, lane_idx, old_beat,
                                                        ),
                                                    );
                                                    let _ = self.command_tx.send(
                                                        AudioCommand::AddAutomationPoint(
                                                            track_idx,
                                                            lane.parameter.clone(),
                                                            new_beat,
                                                            new_value,
                                                        ),
                                                    );
                                                }
                                            }
                                        }
                                        widget_index += 1;
                                    }
                                }
                            }
                        }

                        // Draw playhead
                        if is_playing {
                            let current_beat =
                                (current_position / sample_rate as f64) * (bpm / 60.0) as f64;
                            let playhead_x =
                                timeline_rect.left() + (current_beat as f32 * self.timeline_zoom);

                            painter.line_segment(
                                [
                                    egui::pos2(playhead_x, timeline_rect.top()),
                                    egui::pos2(playhead_x, timeline_rect.bottom()),
                                ],
                                egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 100, 100)),
                            );
                        }
                    });

                ctx.input(|i| {
                    if i.modifiers.ctrl {
                        if i.key_pressed(egui::Key::Z) {
                            if i.modifiers.shift {
                                self.redo();
                            } else {
                                self.undo();
                            }
                        }

                        if i.key_pressed(egui::Key::S) {
                            if let Some(path) = &self.project_path {
                                self.save_project(path);
                            } else {
                                self.show_save_dialog = true;
                            }
                        }

                        if i.key_pressed(egui::Key::O) {
                            self.show_load_dialog = true;
                        }
                    }

                    if i.key_pressed(egui::Key::Space) {
                        let is_playing = self.audio_state.playing.load(Ordering::Relaxed);
                        if is_playing {
                            let _ = self.command_tx.send(AudioCommand::Stop);
                        } else {
                            let _ = self.command_tx.send(AudioCommand::Play);
                        }
                    }

                    if i.key_pressed(egui::Key::Delete) && !self.selected_clips.is_empty() {
                        self.delete_selected();
                    }
                });

                if self.show_clip_menu {
                    let mut close_menu = false;
                    egui::Area::new(egui::Id::new("clip_menu_area"))
                        .fixed_pos(self.clip_menu_pos)
                        .show(ctx, |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                ui.set_min_width(140.0);
                                if ui.button("Split at Playhead").clicked() {
                                    self.split_selected_at_playhead();
                                    close_menu = true;
                                }
                                if ui.button("Delete").clicked() {
                                    self.delete_selected();
                                    close_menu = true;
                                }
                                if ui.button("Rename...").clicked() {
                                    if let Some((track_id, clip_id)) = self.selected_clips.first() {
                                        self.show_rename_dialog = Some((*track_id, *clip_id));
                                        if let Ok(state) = self.state.lock() {
                                            if let Some(clip) = state
                                                .tracks
                                                .get(*track_id)
                                                .and_then(|t| t.audio_clips.get(*clip_id))
                                            {
                                                self.rename_text = clip.name.clone();
                                            }
                                        }
                                    }
                                    close_menu = true;
                                }
                                ui.separator();
                                if ui.button("Normalize").clicked() {
                                    self.normalize_selected();
                                    close_menu = true;
                                }
                                if ui.button("Reverse").clicked() {
                                    self.reverse_selected();
                                    close_menu = true;
                                }
                            });
                        });
                    if close_menu
                        || ctx
                            .input(|i| i.pointer.any_click() && !i.pointer.is_decidedly_dragging())
                    {
                        self.show_clip_menu = false;
                    }
                }

                if let Some((track_id, clip_id)) = self.show_rename_dialog {
                    let mut keep_open = true;
                    egui::Window::new("Rename Clip")
                        .collapsible(false)
                        .resizable(false)
                        .show(ctx, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Name:");
                                ui.text_edit_singleline(&mut self.rename_text);
                            });

                            ui.horizontal(|ui| {
                                if ui.button("OK").clicked() {
                                    self.push_undo();
                                    if let Ok(mut state) = self.state.lock() {
                                        if let Some(clip) = state
                                            .tracks
                                            .get_mut(track_id)
                                            .and_then(|t| t.audio_clips.get_mut(clip_id))
                                        {
                                            clip.name = self.rename_text.clone();
                                        }
                                    }
                                    keep_open = false;
                                }
                                if ui.button("Cancel").clicked() {
                                    keep_open = false;
                                }
                            });
                        });

                    if !keep_open {
                        self.show_rename_dialog = None;
                        self.rename_text.clear();
                    }
                }
            }

            if self.show_mixer {
                egui::Window::new("Mixer")
                    .default_size(egui::vec2(600.0, 400.0))
                    .show(ctx, |ui| {
                        egui::ScrollArea::horizontal().show(ui, |ui| {
                            ui.horizontal(|ui| {
                                // Get track info first
                                let track_info: Vec<_> = {
                                    let state = self.state.lock().unwrap();
                                    state
                                        .tracks
                                        .iter()
                                        .enumerate()
                                        .map(|(i, track)| {
                                            (
                                                i,
                                                track.name.clone(),
                                                track.volume,
                                                track.pan,
                                                track.muted,
                                                track.solo,
                                            )
                                        })
                                        .collect()
                                };

                                // Track strips
                                for (i, name, volume, pan, muted, solo) in track_info {
                                    ui.group(|ui| {
                                        ui.set_min_width(80.0);
                                        ui.vertical(|ui| {
                                            ui.label(&name);
                                            ui.separator();

                                            if i < self.track_meters.len() {
                                                self.track_meters[i].ui(ui, true);
                                            }
                                            ui.separator();

                                            let mut vol = volume;
                                            if ui
                                                .add(
                                                    egui::Slider::new(&mut vol, 0.0..=1.2)
                                                        .vertical()
                                                        .show_value(false),
                                                )
                                                .changed()
                                            {
                                                let _ = self
                                                    .command_tx
                                                    .send(AudioCommand::SetTrackVolume(i, vol));
                                            }
                                            ui.label(format!(
                                                "{:.1} dB",
                                                20.0 * vol.max(0.0001).log10()
                                            ));
                                            ui.separator();

                                            let mut p = pan;
                                            if ui
                                                .add(
                                                    egui::Slider::new(&mut p, -1.0..=1.0)
                                                        .show_value(false),
                                                )
                                                .changed()
                                            {
                                                let _ = self
                                                    .command_tx
                                                    .send(AudioCommand::SetTrackPan(i, p));
                                            }
                                            ui.label(if p.abs() < 0.01 {
                                                "C".to_string()
                                            } else if p < 0.0 {
                                                format!("L{:.0}", -p * 100.0)
                                            } else {
                                                format!("R{:.0}", p * 100.0)
                                            });
                                            ui.separator();

                                            ui.horizontal(|ui| {
                                                if ui
                                                    .button(if muted { "M" } else { "m" })
                                                    .clicked()
                                                {
                                                    let _ = self
                                                        .command_tx
                                                        .send(AudioCommand::MuteTrack(i, !muted));
                                                }

                                                if ui.button(if solo { "S" } else { "s" }).clicked()
                                                {
                                                    let _ = self
                                                        .command_tx
                                                        .send(AudioCommand::SoloTrack(i, !solo));
                                                }
                                            });
                                        });
                                    });
                                }

                                // Master strip
                                ui.separator();
                                ui.group(|ui| {
                                    ui.set_min_width(100.0);
                                    ui.vertical(|ui| {
                                        ui.heading("Master");
                                        ui.separator();
                                        self.master_meter.ui(ui, true);
                                        ui.separator();

                                        let mut master_vol = self.audio_state.master_volume.load();

                                        if ui
                                            .add(
                                                egui::Slider::new(&mut master_vol, 0.0..=1.2)
                                                    .vertical()
                                                    .show_value(false),
                                            )
                                            .changed()
                                        {
                                            self.audio_state.master_volume.store(master_vol);
                                            if let Ok(mut state) = self.state.lock() {
                                                state.master_volume = master_vol;
                                            }
                                        }
                                        ui.label(format!(
                                            "{:.1} dB",
                                            20.0 * master_vol.max(0.0001).log10()
                                        ));
                                    });
                                });
                            });
                        });
                    });
            }

            // Request repaint for smooth playback
            if self.audio_state.playing.load(Ordering::Relaxed) {
                ctx.request_repaint();
            }
        });
    }
}
