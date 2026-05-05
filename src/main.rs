use std::fmt::Display;
use std::time::Duration;

mod audio;

use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{DeviceId, default_host};
use iced::futures::channel::mpsc;
use iced::widget::pick_list;
use iced::{
    Element, Subscription,
    futures::{SinkExt, Stream},
    stream,
    widget::{self, column, row},
};

#[derive(Clone, Debug, PartialEq)]
struct DeviceRepresentation {
    id: DeviceId,
    name: String,
}

impl Display for DeviceRepresentation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl DeviceRepresentation {
    fn new(id: DeviceId, name: String) -> Self {
        Self { id, name }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn id(&self) -> &DeviceId {
        &self.id
    }
}

struct State {
    input_devices: Vec<DeviceRepresentation>,
    device: Option<DeviceRepresentation>,
    device_sender: Option<mpsc::Sender<DeviceRepresentation>>,
    frequency: f64,
    theme: iced::Theme,
}

#[derive(Clone)]
enum Message {
    FrequencyChange(f64),
    GetInputDevices,
    SubscriptionSetup(mpsc::Sender<DeviceRepresentation>),
    InputDeviceChanged(DeviceRepresentation),
}

impl State {
    fn new() -> Self {
        Self {
            input_devices: Vec::new(),
            device: None,
            device_sender: None,
            frequency: 0.,
            theme: iced::Theme::GruvboxDark,
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::FrequencyChange(frequency) => self.frequency = frequency,
            Message::GetInputDevices => self.input_devices = self.get_input_devices(),
            Message::SubscriptionSetup(sender) => self.device_sender = Some(sender),
            Message::InputDeviceChanged(device) => {
                tracing::debug!("Device change");
                self.device = Some(device.clone());
                // Send this device to the subscription task, which starts the frequency detection thread
                if let Some(sender) = &mut self.device_sender {
                    _ = sender.try_send(device);
                }
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let frequency = self.frequency;
        let frequency_text = widget::text(format!("Frequency: {:.2}", frequency));

        let pitch = audio::Pitch::closest_from_frequency(frequency);
        let note = pitch.note();
        let octave = pitch.octave();

        let input_device_picklist =
            pick_list(self.input_devices.clone(), self.device.as_ref(), |device| {
                Message::InputDeviceChanged(device)
            })
            .placeholder("Select input device")
            .on_open(Message::GetInputDevices);

        let note_text = widget::text(note.to_string());
        let octave_text = widget::text(octave).size(24);
        let pitch_info = row!["Pitch: ", note_text, octave_text].align_y(iced::Bottom);

        column![row![frequency_text, pitch_info], input_device_picklist].into()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::run(Self::spawn_frequency_detection_stream)
    }

    fn get_input_devices(&self) -> Vec<DeviceRepresentation> {
        let host = default_host();
        host.input_devices()
            .unwrap()
            .filter_map(|device| {
                let description = device.description().ok()?;
                if description.driver()?.starts_with("plughw:") {
                    return Some(DeviceRepresentation::new(
                        device.id().ok()?,
                        description.name().to_string(),
                    ));
                }
                None
            })
            .collect()
    }

    /// Iced subscription method, sends PitchChange messages when it detects a pitch.
    fn spawn_frequency_detection_stream() -> impl Stream<Item = Message> {
        stream::channel(100, async move |mut output| {
            tracing::debug!("Starting subscription stream");

            let (sender, mut receiver) = mpsc::channel(1);
            _ = output.send(Message::SubscriptionSetup(sender)).await;

            // TODO: currently arbitrary
            let min_freq = 60.;
            let max_freq = 1000.;
            let threshold = 0.10;
            let buffer_size = 1 << 8;
            let detector =
                audio::FrequencyDetector::new(min_freq, max_freq, threshold, buffer_size);
            let mut handle = None;

            loop {
                if let Ok(device) = receiver.try_recv() {
                    tracing::debug!("Got device: {}", device);
                    let Some(device) = default_host().device_by_id(&device.id()) else {
                        tracing::error!(
                            "Device {} not found, likely disconnected while starting thread",
                            device.id()
                        );
                        continue;
                    };
                    handle = detector
                        .start_best_config(device)
                        .map_err(|e| {
                            tracing::error!("Failed to start frequency detector, got error {e:?}")
                        })
                        .ok();
                }

                // Not sleeping makes the repeated sending effectively block
                tokio::time::sleep(Duration::from_millis(10)).await;
                if let Some(handle) = &handle {
                    let frequency = handle.frequency();
                    _ = output.send(Message::FrequencyChange(frequency)).await;
                }
            }
        })
    }

    pub fn theme(&self) -> iced::Theme {
        self.theme.clone()
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("autotune_rs=debug")),
        )
        .init();

    tracing::info!("Starting app");

    _ = iced::application(State::new, State::update, State::view)
        .settings(iced::Settings {
            default_text_size: iced::Pixels(32.),
            ..Default::default()
        })
        .subscription(State::subscription)
        .theme(State::theme)
        .run();
}
