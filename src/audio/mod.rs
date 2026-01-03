//! Module for general audio related logic.

mod yin;
use self::yin::Yin;

use atomic_float::AtomicF64;
use cpal::{
    Device, FromSample, SizedSample, Stream,
    traits::{DeviceTrait, StreamTrait},
};
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Producer, Split},
};

use std::{
    fmt::Display,
    sync::{
        Arc,
        mpsc::{self, SyncSender},
    },
    thread,
};

#[derive(Copy, Clone)]
pub struct FrequencyDetector {
    min_freq: f64,
    max_freq: f64,
    threshold: f64,
    buffer_size: usize,
}

impl FrequencyDetector {
    pub fn new(min_freq: f64, max_freq: f64, threshold: f64, buffer_size: usize) -> Self {
        Self {
            min_freq,
            max_freq,
            threshold,
            buffer_size,
        }
    }

    /// Start the input stream for the given device,
    /// choosing the config with the highest bits per sample
    /// and highest sample rate.
    ///
    /// # Panics
    ///
    /// Panics if the device is not an input device.
    pub fn start_best_config(self, device: Device) -> FrequencyDetectorHandle {
        let config_range = device.supported_input_configs().unwrap();
        let config = config_range
            .into_iter()
            .max_by_key(|config| {
                let format = config.sample_format();
                let bits = format.bits_per_sample();
                bits * config.max_sample_rate()
            })
            .unwrap()
            .try_with_sample_rate(48000) //;
            .unwrap();
        let sample_format = config.sample_format();
        let mut config = config.config();
        let stream_buffer_size = 256;
        config.buffer_size = cpal::BufferSize::Fixed(256);
        config.channels = 1;
        dbg!(&config);

        let yin = Yin::new(
            config.sample_rate,
            self.min_freq,
            self.max_freq,
            self.threshold,
        );

        let (sample_producer, sample_consumer) = HeapRb::new(2 * stream_buffer_size).split();
        let (frequency, sample_signal, thread) = frequency_detection_thread(yin, sample_consumer);
        let stream = build_stream(
            device,
            config,
            sample_format,
            sample_producer,
            sample_signal,
        );

        stream.play().unwrap();
        FrequencyDetectorHandle::new(frequency, thread, stream)
    }
}

pub struct FrequencyDetectorHandle {
    frequency: Arc<AtomicF64>,
    detector_thread: thread::JoinHandle<()>,
    stream: Stream,
}

impl FrequencyDetectorHandle {
    fn new(frequency: Arc<AtomicF64>, thread: thread::JoinHandle<()>, stream: Stream) -> Self {
        Self {
            frequency,
            detector_thread: thread,
            stream,
        }
    }

    /// Returns the last detected frequency.
    #[inline(always)]
    pub fn frequency(&self) -> f64 {
        self.frequency.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Start a thread to gather samples from the producer and attempt to estimate the frequency with the yin algorithm.
///
/// If a frequency is found, `frequency_updater` is updated with the new value.
fn frequency_detection_thread(
    yin: Yin,
    mut sample_consumer: HeapCons<f64>,
) -> (Arc<AtomicF64>, SyncSender<()>, thread::JoinHandle<()>) {
    let frequency_atomic = Arc::new(AtomicF64::new(0.));
    let frequency_cloned = frequency_atomic.clone();

    let (signal_sender, signal_reader) = mpsc::sync_channel(0);

    let frame_size = yin.minimum_frame_size();
    let mut frame = vec![0.; frame_size];
    // TODO: this thread handle should be stored somewhere along with the stream,
    // if one dies, the other should too.
    // OR
    // do the processing in the audio input thread.
    let thread = thread::Builder::new()
        .name("Autotune.rs - pitch detection".into())
        .spawn(move || {
            let mut i = 0;
            loop {
                if signal_reader.recv().is_err() {
                    break;
                };
                for sample in sample_consumer.pop_iter() {
                    frame[i] = sample;
                    i += 1;
                    if i == frame_size {
                        i = 0;
                        let frequency = yin.detect_frequency(&frame);
                        if let Some(frequency) = frequency {
                            frequency_atomic.store(frequency, std::sync::atomic::Ordering::Relaxed);
                        };
                    }
                }
            }
        })
        .unwrap();

    (frequency_cloned, signal_sender, thread)
}

macro_rules! impl_spawn_stream {
    (
        $device:expr,
        $config:expr,
        $sample_format:expr,
        $producer:expr,
        $sample_signal:expr,
        [
            $($p:ident => $t:ty),+
            $(,)?
        ]
    ) => {
            {
            match $sample_format {
                $(cpal::SampleFormat::$p => {
                    build_input_stream::<$t>(
                        $device,
                        $config,
                        $producer,
                        $sample_signal,
                    )
                })+,
                format => panic!("unsupported sample format {}", format)
            }
        }
    }
}

/// Build the stream for the given device and config.
fn build_stream(
    device: cpal::Device,
    config: cpal::StreamConfig,
    sample_format: cpal::SampleFormat,
    sample_producer: HeapProd<f64>,
    sample_signal: SyncSender<()>,
) -> cpal::Stream {
    let stream = impl_spawn_stream!(
        &device,
        config,
        sample_format,
        sample_producer,
        sample_signal,
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
    stream
}

/// Build the input stream on the given device, for any Sample T.
fn build_input_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut sample_buffer: HeapProd<f64>,
    sample_signal: SyncSender<()>,
) -> cpal::Stream
where
    T: SizedSample,
    f64: FromSample<T>,
{
    let channels = config.channels as usize;
    let stream = device
        .build_input_stream(
            &config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                read_audio(data, &mut sample_buffer, channels, &sample_signal)
            },
            |_| todo!("TODO: implement input stream error callback"),
            None,
        )
        .map_err(|_e| todo!("TODO: add error handling for stream building"))
        .unwrap();
    stream
}

/// Reads from the `samples` slice, pushes it onto the producer,
/// then unparks the pitch estimation thread.
///
/// Currently uses only one of the data channels.
fn read_audio<T>(
    samples: &[T],
    sample_producer: &mut HeapProd<f64>,
    channels: usize,
    sample_signal: &SyncSender<()>,
) where
    T: SizedSample,
    f64: FromSample<T>,
{
    // Possible issue if the sample buffer fills up because of consumer lag.
    //
    // This function (on average) writes samples at `sample_rate / channels`,
    // the consumer should be able to keep up with this.

    let sample_count_mono = samples.len() / channels;

    let samples = samples
        .into_iter()
        .step_by(channels)
        .map(|sample| sample.to_sample());

    #[cfg(debug_assertions)]
    if sample_producer.push_iter(samples) < sample_count_mono {
        eprintln!("Consumer fell behind");
    }

    sample_signal.send(()).unwrap();
}

/// A struct that carries information about a Note and an octave,
/// can be converted into a frequency, and approximated from a frequency (see [`closest_from_frequency`])
///
/// [`closest_from_frequency`]: Self::closest_from_frequency
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Pitch {
    note: Note,
    octave: u8,
}

impl Pitch {
    pub const C0: Self = Self {
        note: Note::C,
        octave: 0,
    };

    pub const A440: Self = Self {
        note: Note::A,
        octave: 4,
    };
    pub const A4: Self = Self::A440;

    pub fn new(note: Note, octave: u8) -> Self {
        Pitch { note, octave }
    }

    pub fn note(&self) -> &Note {
        &self.note
    }

    pub fn octave(&self) -> &u8 {
        &self.octave
    }

    fn semitones_to_c0(&self) -> f64 {
        self.note.to_semitones_from_c() + 12. * self.octave as f64
    }

    /// Calculate the frequency based on equal temperament, relative to [`A440`].
    ///
    /// [`A440`]: Self::A440
    pub fn frequency(&self) -> f64 {
        let semitone = 2_f64.powf(1. / 12.);
        let semitones_to_c0 = self.semitones_to_c0();
        let semitone_to_a4 = semitones_to_c0 as i32 - (4 * 12 + 9);
        440. * semitone.powi(semitone_to_a4)
    }

    fn frequency_to_semitones_from_c0(frequency: f64) -> f64 {
        12. * (frequency / Self::C0.frequency()).log2()
    }

    /// Estimate the closest [`Pitch`] ([`Note`] and octave) for the given frequency.
    ///
    /// The closest pitch is calculated by finding the number of semitones from [`C0`],
    /// rounding it, and converting it back into a pitch.
    ///
    /// [`C0`]: Self::C0
    pub fn closest_from_frequency(frequency: f64) -> Self {
        let semitones = Self::frequency_to_semitones_from_c0(frequency).round() as usize;
        let (semitones, octave) = (semitones % 12, semitones / 12);
        let note = Note::from_semitones_from_c(semitones as isize);
        Self::new(note, octave as u8)
    }
}

/// An octave-independant note representation.
///
/// Use [`Pitch`] to include octave information.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Note {
    C,
    CSharp,
    D,
    EFlat,
    E,
    F,
    FSharp,
    G,
    GSharp,
    A,
    BFlat,
    B,
}

impl Note {
    /// Returns the number of semitones from C.
    pub fn to_semitones_from_c(&self) -> f64 {
        match self {
            Self::C => 0.,
            Self::CSharp => 1.,
            Self::D => 2.,
            Self::EFlat => 3.,
            Self::E => 4.,
            Self::F => 5.,
            Self::FSharp => 6.,
            Self::G => 7.,
            Self::GSharp => 8.,
            Self::A => 9.,
            Self::BFlat => 10.,
            Self::B => 11.,
        }
    }

    /// Returns the Note `semitones` away from C, ignoring the octave.
    pub fn from_semitones_from_c(mut semitones: isize) -> Self {
        semitones %= 12;
        if semitones < 0 {
            semitones += 12;
        }
        match semitones {
            0 => Self::C,
            1 => Self::CSharp,
            2 => Self::D,
            3 => Self::EFlat,
            4 => Self::E,
            5 => Self::F,
            6 => Self::FSharp,
            7 => Self::G,
            8 => Self::GSharp,
            9 => Self::A,
            10 => Self::BFlat,
            11 => Self::B,
            _ => unreachable!("mod 12 ensures it's within 0..12"),
        }
    }
}

impl Display for Note {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let note = match self {
            Self::C => "C",
            Self::CSharp => "C♯",
            Self::D => "D",
            Self::EFlat => "E♭",
            Self::E => "E",
            Self::F => "F",
            Self::FSharp => "F♯",
            Self::G => "G",
            Self::GSharp => "G♯",
            Self::A => "A",
            Self::BFlat => "B♭",
            Self::B => "B",
        };
        f.write_str(note)
    }
}

#[cfg(test)]
mod tests {
    use super::{Note, Pitch};
    #[test]
    fn semitones_to_note() {
        let semitones = -3;
        let note = Note::from_semitones_from_c(semitones);
        assert_eq!(note, Note::A);
    }

    #[test]
    fn semitones_from_c0() {
        let pitch = Pitch::new(Note::CSharp, 2);
        let semitones_away = pitch.semitones_to_c0();
        assert_eq!(semitones_away, 25.);
    }

    #[test]
    fn frequency_a4() {
        let pitch = Pitch::new(Note::A, 4);
        let frequency = pitch.frequency();
        assert_eq!(frequency, 440.);
    }

    #[test]
    fn frequency_c0() {
        let pitch = Pitch::new(Note::C, 0);
        let frequency = pitch.frequency();
        assert_eq!(frequency, 16.351597831287375);
    }

    #[test]
    fn frequency_to_semitones_from_c0() {
        // D1
        let frequency = 36.70809598967586;
        let semitones = Pitch::frequency_to_semitones_from_c0(frequency);
        assert_eq!(semitones.round(), 14.);
    }

    #[test]
    fn frequency_to_closest_pitch() {
        let frequency = 36.70809598967586;
        let pitch = Pitch::closest_from_frequency(frequency);
        assert_eq!(pitch, Pitch::new(Note::D, 1));

        let frequency = 438.;
        let pitch = Pitch::closest_from_frequency(frequency);
        assert_eq!(pitch, Pitch::new(Note::A, 4));
    }
}
