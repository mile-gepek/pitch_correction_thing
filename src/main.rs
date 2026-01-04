mod audio;

use std::time::Duration;

use cpal::traits::HostTrait;
use iced::{
    Element, Subscription,
    futures::{SinkExt, Stream},
    stream,
    widget::{self, column, row},
};

struct State {
    frequency: f64,
    theme: iced::Theme,
}

#[derive(Clone)]
enum Message {
    FrequencyChange(f64),
}

impl State {
    fn new() -> Self {
        Self {
            frequency: 0.,
            theme: iced::Theme::GruvboxDark,
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::FrequencyChange(frequency) => self.frequency = frequency,
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let frequency = self.frequency;
        let frequency_text = widget::text(format!("Frequency: {:.2}", frequency));

        let pitch = audio::Pitch::closest_from_frequency(frequency);
        let note = pitch.note();
        let octave = pitch.octave();

        let note_text = widget::text(note.to_string());
        let octave_text = widget::text(octave).size(24);
        let pitch_info = row!["Pitch: ", note_text, octave_text].align_y(iced::Bottom);

        column![frequency_text, pitch_info].into()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::run(Self::spawn_frequency_detection_stream)
    }

    /// Iced subscription method, sends PitchChange messages when it detects a pitch.
    fn spawn_frequency_detection_stream() -> impl Stream<Item = Message> {
        stream::channel(100, async |mut output| {
            let host = cpal::default_host();
            let device = host.default_input_device().unwrap();

            // TODO: currently arbitrary
            let min_freq = 60.;
            let max_freq = 1000.;
            let threshold = 0.10;
            let buffer_size = 1 << 8;
            let handle = audio::FrequencyDetector::new(min_freq, max_freq, threshold, buffer_size)
                .start_best_config(device);

            loop {
                // Not sleeping makes the repeated sending effectively block
                tokio::time::sleep(Duration::from_millis(10)).await;
                let frequency = handle.frequency();
                _ = output.send(Message::FrequencyChange(frequency)).await;
            }
        })
    }

    pub fn theme(&self) -> iced::Theme {
        self.theme.clone()
    }
}

fn main() {
    _ = iced::application(State::new, State::update, State::view)
        .settings(iced::Settings {
            default_text_size: iced::Pixels(32.),
            ..Default::default()
        })
        .subscription(State::subscription)
        .theme(State::theme)
        .run();
}
