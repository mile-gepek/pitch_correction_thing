//! Module for general audio related logic.

mod yin;
use yin::Yin;
mod pitch;
pub use pitch::Pitch;
mod td_psola;

use atomic_float::AtomicF64;
use cpal::{
    BuildStreamError, Device, FromSample, SizedSample, Stream,
    traits::{DeviceTrait, StreamTrait},
};
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Producer, Split},
};
use tracing;

use std::{
    sync::{
        Arc,
        mpsc::{self, SyncSender},
    },
    thread,
    time::Duration,
};

#[derive(Copy, Clone)]
pub struct FrequencyDetector {
    min_freq: f64,
    max_freq: f64,
    threshold: f64,
}

impl FrequencyDetector {
    pub fn new(min_freq: f64, max_freq: f64, threshold: f64) -> Self {
        Self {
            min_freq,
            max_freq,
            threshold,
        }
    }

    /// Start the input stream for the given device,
    /// choosing the config with the highest bits per sample
    /// and highest sample rate.
    ///
    /// # Panics
    ///
    /// Panics if the device is not an input device.
    ///
    /// Panics if spawning the frequency detection thread fails.
    pub fn start_best_config(
        self,
        device: Device,
    ) -> Result<FrequencyDetectorHandle, BuildStreamError> {
        let config = device
            .default_output_config()
            .inspect_err(|e| {
                tracing::error!(
                    "Got error while getting default config for device {}: {e}",
                    device.id().unwrap()
                )
            })
            .unwrap();
        let sample_format = config.sample_format();
        let mut config = config.config();

        let yin = Yin::new(
            config.sample_rate,
            self.min_freq,
            self.max_freq,
            self.threshold,
        );
        let buffer_size = yin.minimum_frame_size();

        config.buffer_size = cpal::BufferSize::Fixed(buffer_size as u32);
        config.channels = 1;

        let (sample_producer, sample_consumer) = HeapRb::new(buffer_size).split();
        let (frequency, sample_signal, thread) = frequency_detection_thread(yin, sample_consumer);
        let stream = build_stream(
            device,
            config,
            sample_format,
            sample_producer,
            sample_signal,
        )
        .inspect_err(|e| tracing::error!("Failed to build stream, got error: {e}"))?;

        // TODO: return an error on stream fail
        stream
            .play()
            .expect("If playing the stream fails the device is disconnected");
        Ok(FrequencyDetectorHandle::new(frequency, thread, stream))
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
    tracing::debug!("Yin expecting minimum frame size {frame_size}");
    let mut frame = vec![0.; frame_size];
    // TODO: this thread handle should be stored somewhere along with the stream,
    // if one dies, the other should too.
    // OR
    // do the processing in the audio input thread.
    let thread = thread::Builder::new()
        .name("pitch detection".into())
        .spawn(move || {
            tracing::debug!("Starting frequency detection thread");
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
                thread::sleep(Duration::from_millis(4));
            }
            tracing::debug!("Exiting frequency detection thread")
        })
        .expect("We have other things to worry about if spawning a thread fails");

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
) -> Result<cpal::Stream, BuildStreamError> {
    impl_spawn_stream!(
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
    )
}

/// Build the input stream on the given device, for any Sample T.
fn build_input_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut sample_buffer: HeapProd<f64>,
    sample_signal: SyncSender<()>,
) -> Result<cpal::Stream, BuildStreamError>
where
    T: SizedSample,
    f64: FromSample<T>,
{
    let channels = config.channels as usize;
    device.build_input_stream(
        &config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            read_audio(data, &mut sample_buffer, channels, &sample_signal)
        },
        |e| tracing::error!("Got stream callback error: {e:?}"),
        None,
    )
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

    let samples_pushed = sample_producer.push_iter(samples);

    sample_signal.send(()).unwrap();

    #[cfg(debug_assertions)]
    if samples_pushed < sample_count_mono {
        // Yes I know I shouldn't log in the audio thread, sue me.
        tracing::debug!("Consumer fell behind");
    }
}
