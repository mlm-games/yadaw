use yadaw::integration::{DawCore, ExportFormat};
use yadaw::state::MidiNote;
use yadaw::track_manager::TrackType;

fn main() {
    // Initialize the DAW core
    let daw = DawCore::new();

    // Create a new project
    daw.create_new_project("My Song".to_string());

    // Add some tracks
    {
        let mut track_manager = daw.track_manager.write();
        let mut state = daw.state.write();

        let drum_track = track_manager.create_track(TrackType::Midi, Some("Drums".to_string()));
        state.tracks.push(drum_track);

        let bass_track = track_manager.create_track(TrackType::Midi, Some("Bass".to_string()));
        state.tracks.push(bass_track);

        let vocal_track = track_manager.create_track(TrackType::Audio, Some("Vocals".to_string()));
        state.tracks.push(vocal_track);
    }

    // Load some plugins
    {
        let mut plugin_host = daw.plugin_host.write();
        let plugins = plugin_host.get_available_plugins().to_vec();

        // Load reverb plugin
        if let Some(reverb) = plugins.iter().find(|p| p.name == "Reverb") {
            if let Ok(plugin_id) = plugin_host.load_plugin(reverb) {
                println!("Loaded reverb plugin with ID: {}", plugin_id);
            }
        }
    }

    // Create some automation
    {
        let mut automation = daw.automation.write();

        // Create volume automation for track 0
        let lane_id = automation.create_lane(
            "Track 1 Volume".to_string(),
            yadaw::automation::AutomationTarget::TrackVolume(0),
        );

        // Add automation points
        automation.add_point(lane_id, 0.0, 0.0); // Start at silence
        automation.add_point(lane_id, 4.0, 1.0); // Fade in over 4 beats
        automation.add_point(lane_id, 32.0, 1.0); // Stay at full volume
        automation.add_point(lane_id, 36.0, 0.0); // Fade out
    }

    // Set up MIDI routing
    {
        let midi_ports = yadaw::midi_engine::MidiEngine::list_output_ports();
        println!("Available MIDI output ports:");
        for (i, port) in midi_ports.iter().enumerate() {
            println!("  {}: {}", i, port);
        }
    }

    // Create a simple beat pattern
    {
        let mut state = daw.state.write();
        if let Some(drum_track) = state.tracks.iter_mut().find(|t| t.name == "Drums") {
            if let Some(pattern) = drum_track.patterns.first_mut() {
                pattern.notes.clear();

                // Kick drum on beats 1 and 3
                pattern.notes.push(MidiNote {
                    pitch: 36,
                    velocity: 100,
                    start: 0.0,
                    duration: 0.5,
                });
                pattern.notes.push(MidiNote {
                    pitch: 36,
                    velocity: 100,
                    start: 2.0,
                    duration: 0.5,
                });

                // Snare on beats 2 and 4
                pattern.notes.push(MidiNote {
                    pitch: 38,
                    velocity: 90,
                    start: 1.0,
                    duration: 0.25,
                });
                pattern.notes.push(MidiNote {
                    pitch: 38,
                    velocity: 90,
                    start: 3.0,
                    duration: 0.25,
                });

                // Hi-hats
                for i in 0..8 {
                    pattern.notes.push(MidiNote {
                        pitch: 42,
                        velocity: 60,
                        start: i as f64 * 0.5,
                        duration: 0.1,
                    });
                }
            }
        }
    }

    // Monitor performance
    {
        let performance = daw.performance.read();
        if let Some(metrics) = performance.get_current_metrics() {
            println!("Current performance metrics:");
            println!("  CPU Usage: {:.1}%", metrics.cpu_usage * 100.0);
            println!("  Memory: {} MB", metrics.memory_usage / (1024 * 1024));
            println!("  Latency: {:.1} ms", metrics.latency_ms);
        }
    }

    println!("DAW initialized successfully!");
    println!("Project has {} tracks", daw.state.read().tracks.len());
}
