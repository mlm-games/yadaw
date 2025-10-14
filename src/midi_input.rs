use anyhow::{Result, anyhow};
use crossbeam_channel::Sender;
use midir::{Ignore, MidiInput, MidiInputConnection, MidiInputPort};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RawMidiMessage {
    pub timestamp_us: u64,
    pub message: [u8; 3],
}

pub struct MidiInputHandler {
    midi_in: Arc<Mutex<MidiInput>>,
    connection: Arc<Mutex<Option<MidiInputConnection<()>>>>,
    command_tx: Sender<crate::messages::AudioCommand>,
    pub connected_port_name: Arc<Mutex<Option<String>>>,
}

impl MidiInputHandler {
    pub fn new(command_tx: Sender<crate::messages::AudioCommand>) -> Result<Self> {
        let midi_in = MidiInput::new("YADAW-MIDI-Input")?;
        Ok(Self {
            midi_in: Arc::new(Mutex::new(midi_in)),
            connection: Arc::new(Mutex::new(None)),
            command_tx,
            connected_port_name: Arc::new(Mutex::new(None)),
        })
    }

    pub fn connect(&self, port_name: &str) -> Result<()> {
        self.disconnect();

        let port_to_connect: MidiInputPort = {
            let midi_in_guard = self.midi_in.lock().unwrap();
            let ports = midi_in_guard.ports();

            ports
                .iter()
                .find(|p| midi_in_guard.port_name(p).as_deref() == Ok(port_name))
                .ok_or_else(|| anyhow!("MIDI Port not found: {}", port_name))?
                .clone()
        };

        let mut midi_in_for_connection = MidiInput::new(&format!("YADAW-conn-{}", port_name))?;
        // Disable unnecessary logging from this temporary instance.
        midi_in_for_connection.ignore(Ignore::All);

        let port_name_clone = port_name.to_string();
        let command_tx_clone = self.command_tx.clone();
        let connected_port_name_clone = self.connected_port_name.clone();

        let initial_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;

        log::info!("Attempting to connect to MIDI port: {}", port_name_clone);

        let conn = midi_in_for_connection
            .connect(
                &port_to_connect,
                &port_name_clone,
                move |stamp, message, _| {
                    if message.len() == 3 {
                        let raw_message = RawMidiMessage {
                            timestamp_us: initial_time + stamp,
                            message: [message[0], message[1], message[2]],
                        };
                        let _ = command_tx_clone
                            .try_send(crate::messages::AudioCommand::MidiInput(raw_message));
                    }
                },
                (),
            )
            .map_err(|e| {
                anyhow!(
                    "Failed to connect to MIDI port '{}': {}",
                    port_name_clone,
                    e
                )
            })?;

        *self.connection.lock().unwrap() = Some(conn);
        *connected_port_name_clone.lock().unwrap() = Some(port_name_clone);

        Ok(())
    }

    pub fn disconnect(&self) {
        if let Some(conn) = self.connection.lock().unwrap().take() {
            conn.close();
            *self.connected_port_name.lock().unwrap() = None;
            log::info!("MIDI input connection closed.");
        }
    }

    pub fn list_ports(&self) -> Vec<String> {
        let midi_in_guard = self.midi_in.lock().unwrap();
        midi_in_guard
            .ports()
            .iter()
            .filter_map(|p| midi_in_guard.port_name(p).ok())
            .collect()
    }
}
