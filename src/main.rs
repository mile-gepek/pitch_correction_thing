mod audio;
mod signal;

use std::time::Duration;

use iced::{
    Element, Subscription,
    futures::{SinkExt, Stream},
    stream,
    widget::{self, column, row},
};

struct State {
    previous_frequencies: Vec<f64>,
}

#[derive(Clone)]
enum Message {
    FrequencyChange(f64),
}

impl State {
    fn new() -> Self {
        Self {
            previous_frequencies: Vec::with_capacity(5),
        }
    }

    fn get_median_frequency(&self) -> Option<f64> {
        if self.previous_frequencies.is_empty() {
            return None;
        }
        let mut frequencies = self.previous_frequencies.clone();
        frequencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let len = frequencies.len();
        // Yes I know this isn't the median if it's even lenght, I don't care
        Some(frequencies[len / 2])
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::FrequencyChange(frequency) => {
                if self.previous_frequencies.len() == self.previous_frequencies.capacity() {
                    self.previous_frequencies.remove(0);
                }
                self.previous_frequencies.push(frequency)
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let frequency = self.get_median_frequency().unwrap_or_default();
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
            // TODO: currently arbitrary
            let buffer_size = 1 << 10;
            let frame_size = 1 << 9;
            let (frequency_atomic, _stream) =
                audio::init_frequency_detection(buffer_size, frame_size);

            loop {
                // Not sleeping makes the repeated sending effectively block
                tokio::time::sleep(Duration::from_millis(10)).await;
                let frequency = frequency_atomic.load(std::sync::atomic::Ordering::Relaxed);
                _ = output.send(Message::FrequencyChange(frequency)).await;
            }
        })
    }
}

fn main() {
    _ = iced::application(State::new, State::update, State::view)
        .settings(iced::Settings {
            default_text_size: iced::Pixels(32.),
            ..Default::default()
        })
        .subscription(State::subscription)
        .run();
}
