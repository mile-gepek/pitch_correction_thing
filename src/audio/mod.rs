//! Module for general audio related logic.

pub mod yin;
use self::yin::Yin;

use atomic_float::AtomicF64;
use cpal::{
    FromSample, SizedSample, Stream,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use rtrb::{Consumer, Producer, RingBuffer};
use std::{fmt::Display, sync::Arc, thread, time::Duration};

/// Start a thread to gather samples from the producer and attempt to estimate the frequency with the yin algorithm.
///
/// If a frequency is found, `frequency_updater` is updated with the new value.
fn frequency_detection_thread(
    yin: Yin,
    frame_size: usize,
    mut sample_consumer: rtrb::Consumer<f64>,
) -> Arc<AtomicF64> {
    let frequency_atomic = Arc::new(AtomicF64::new(0.));
    let frequency_cloned = frequency_atomic.clone();

    let mut frame = Vec::with_capacity(frame_size);
    // TODO: this thread handle should be stored somewhere along with the stream,
    // if one dies, the other should too.
    // OR
    // do the processing in the audio input thread.
    thread::spawn(move || {
        loop {
            if let Ok(sample) = sample_consumer.pop() {
                frame.push(sample);
                if frame.len() == frame_size {
                    let frequency = yin.detect_pitch(&frame);
                    frame.clear();
                    let Some(frequency) = frequency else {
                        continue;
                    };
                    frequency_atomic.store(frequency, std::sync::atomic::Ordering::Relaxed);
                }
            } else {
                // At a sample rate of 48000 hz, it takes ~21 milliseconds
                // to get 1024 samples, so we can sleep and still be sure
                // the buffer isn't full
                thread::sleep(Duration::from_millis(5));
            }
        }
    });

    frequency_cloned
}

pub fn init_frequency_detection(buffer_size: usize, frame_size: usize) -> (Arc<AtomicF64>, Stream) {
    let (yin_esimator, stream, sample_consumer) = build_stream_and_estimator(buffer_size);
    stream.play().unwrap();
    let frequency = frequency_detection_thread(yin_esimator, frame_size, sample_consumer);
    (frequency, stream)
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
                read_audio(data, &mut sample_buffer, channels)
            },
            |_| todo!("TODO: implement input stream error callback"),
            None,
        )
        .map_err(|_e| todo!("TODO: add error handling for stream building"))
        .unwrap();
    stream
}

/// Reads from the `data` slice and pushes it onto the producer,
/// currently uses only one of the data channels.
fn read_audio<T>(data: &[T], sample_producer: &mut Producer<f64>, channels: usize)
where
    T: SizedSample,
    f64: FromSample<T>,
{
    // Possible issue if the sample buffer fills up because of consumer lag.
    // This function (on average) writes samples at `sample_rate / channels`,
    // the consumer should be able to keep up with this.

    // TODO: figure out how to use all channels
    for sample in data.into_iter().step_by(channels) {
        if sample_producer.push(sample.to_sample()).is_err() {
            // Consumer fell behind..
            // TODO: printing in an audio thread is stupid.
            eprintln!("Consumer fell behind");
        }
    }
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
