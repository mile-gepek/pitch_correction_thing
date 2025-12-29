mod fft;
mod yin;

use std::{thread, time::Duration};

use cpal::{
    FromSample, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use iced::{
    Element, Subscription,
    futures::{SinkExt, Stream},
    stream, widget,
};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::yin::Yin;

struct State {
    previous_frequencies: Vec<f64>,
}

#[derive(Clone)]
enum Message {
    FrequencyChange(f64),
    ButtonPress,
}

impl State {
    fn new() -> Self {
        Self {
            previous_frequencies: Vec::with_capacity(5),
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::FrequencyChange(frequency) => {
                if self.previous_frequencies.len() == self.previous_frequencies.capacity() {
                    self.previous_frequencies.remove(0);
                }
                self.previous_frequencies.push(frequency)
            }
            Message::ButtonPress => {
                println!("button pressed");
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let mut frequencies = self.previous_frequencies.clone();
        frequencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let len = frequencies.len();
        let median_frequency = if len % 2 == 0 && len >= 2 {
            let lower = frequencies[len / 2 - 1];
            let upper = frequencies[len / 2];
            (lower + upper) / 2.
        } else if len > 0 {
            frequencies[len / 2]
        } else {
            0.
        };
        let pitch_text = widget::text(format!("{:.2}", median_frequency));
        let button = widget::button("BLA").on_press(Message::ButtonPress);
        widget::column![pitch_text, button].into()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::run(Self::spawn_frequency_detection_stream)
    }

    /// Iced subscription method, sends PitchChange messages when it detects a pitch.
    fn spawn_frequency_detection_stream() -> impl Stream<Item = Message> {
        stream::channel(100, async |mut output| {
            // TODO: currently arbitrary
            let buffer_size = 1 << 10;
            let frame_size = 1 << 10;

            let (yin_esimator, _stream, sample_consumer) = build_stream_and_estimator(buffer_size);

            let (frequency_updater, mut frequency_reader) = triple_buffer::triple_buffer(&0.);
            // Spawning a new thread increases consumer reading speed,
            // because there is no need to await and tokio::sleep
            thread::spawn(move || {
                frequency_detection(yin_esimator, frame_size, sample_consumer, frequency_updater)
            });

            loop {
                frequency_reader.update();
                let frequency = frequency_reader.read();
                _ = output.send(Message::FrequencyChange(*frequency)).await;
                // Not sleeping makes the repeated sending effectively block
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
    }
}

/// Gather samples from the producer and attempt to estimate the frequency with the yin algorithm.
///
/// If a frequency is found, `frequency_updater` is updated with the new value.
fn frequency_detection(
    yin: Yin,
    frame_size: usize,
    mut sample_consumer: rtrb::Consumer<f64>,
    mut frequency_updater: triple_buffer::Input<f64>,
) {
    let mut frame = Vec::with_capacity(frame_size);
    loop {
        if let Ok(sample) = sample_consumer.pop() {
            frame.push(sample);
            if frame.len() == frame_size {
                let frequency = yin.detect_pitch(&frame);
                frame.clear();
                let Some(frequency) = frequency else {
                    continue;
                };
                frequency_updater.write(frequency);
            }
        } else {
            // At a sample rate of 48000 hz, it takes ~21 milliseconds
            // to get 1024 samples, so we can sleep and still be sure
            // the buffer isn't full
            thread::sleep(Duration::from_millis(10));
        }
    }
}

macro_rules! impl_spawn_stream {
    (
        $device:expr,
        $config:expr,
        $producer:expr,
        [
            $($p:ident => $t:ty),+
            $(,)?
        ]
    ) => {
            {
            match $config.sample_format() {
                $(cpal::SampleFormat::$p => {
                    build_input_stream::<$t>(
                        $device,
                        &($config).into(),
                        $producer,
                    )
                })+,
                format => panic!("unsupported sample format {}", format)
            }
        }
    }
}

/// Find the default device and build a stream for it, alongside the Yin estimator.
fn build_stream_and_estimator(buffer_size: usize) -> (Yin, cpal::Stream, Consumer<f64>) {
    let host = cpal::default_host();
    let device = host.default_input_device().unwrap();

    let mut config_range = device.supported_input_configs().unwrap();
    config_range.next().unwrap();
    let config = config_range.next().unwrap().with_max_sample_rate();

    let (sample_producer, sample_consumer) = RingBuffer::<f64>::new(buffer_size);

    let freq_min = 80.;
    let freq_max = 1000.;
    let threshold = 0.1;
    let yin = Yin::new(config.sample_rate(), freq_min, freq_max, threshold);

    let stream = impl_spawn_stream!(
        &device,
        config,
        sample_producer,
        [
            I8 => i8,
            I16 => i16,
            I32 => i32,
            I64 => i64,
            U8 => u8,
            U16 => u16,
            U32 => u32,
            U64 => u64,
            F32 => f32,
            F64 => f64,
        ]
    );
    (yin, stream, sample_consumer)
}

/// Build the input stream on the given device, for any Sample T.
fn build_input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut sample_buffer: Producer<f64>,
) -> cpal::Stream
where
    T: SizedSample,
    f64: FromSample<T>,
{
    let channels = config.channels as usize;
    let stream = device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                write_audio(data, &mut sample_buffer, channels)
            },
            |_| todo!("TODO: implement input stream error callback"),
            None,
        )
        .map_err(|_e| todo!("TODO: add error handling for stream building"))
        .unwrap();
    stream.play().unwrap();
    stream
}

fn write_audio<T>(data: &[T], sample_producer: &mut Producer<f64>, channels: usize)
where
    T: SizedSample,
    f64: FromSample<T>,
{
    // Possible issue if the sample buffer fills up because of consumer lag.
    // This function (on average) writes samples at `sample_rate / channels`,
    // the consumer should be able to keep up with this.
    for sample in data.into_iter().step_by(channels) {
        if sample_producer.push(sample.to_sample()).is_err() {
            // Producer fell behind..
            // TODO: printing in an audio thread is stupid.
            eprintln!("producer fell behind");
        }
    }
}

fn main() {
    _ = iced::application(State::new, State::update, State::view)
        .subscription(State::subscription)
        .run();
    // let sample_rate = 48000;
    // let frequencies = [(220., 1)];
    // let angles: Vec<(f64, u8)> = frequencies
    //     .iter()
    //     .map(|&(freq, i)| (TAU * freq / sample_rate as f64, i))
    //     .collect();
    // let steps = 1 << 12;
    // let samples: Vec<f64> = (0..steps)
    //     .map(|i| {
    //         angles
    //             .iter()
    //             .map(|&(angle, mul)| {
    //                 let value: f64 = angle * i as f64;
    //                 value.sin() / mul as f64
    //             })
    //             .sum()
    //     })
    //     .collect();

    // // println!("Amplitude, time");
    // // for (i, sample) in samples.iter().enumerate() {
    // //     println!("{}, {}", i, sample);
    // // }
    // let yin = yin::Yin::new(sample_rate, 80., 1000., 0.15);
    // let detected = yin.detect_pitch(&samples);
    // println!("{:?}", detected);
}
