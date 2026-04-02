//! Implementation of the YIN frequency detection algorithm.
//!
//! To begin, construct a new Yin instance, and use the [`detect_frequency`] method on the samples.
//!
//! [`detect_frequency`]: Yin::detect_frequency

// The yin estimator settings.
pub struct Yin {
    pub sample_rate: u32,
    pub tau_min: usize,
    pub tau_max: usize,
    pub threshold: f64,
}

impl Yin {
    /// Create a new yin instance with the given settings.
    ///
    /// # Panics
    ///
    /// Panics if `min_freq` is higher or equal to `max_freq`
    pub fn new(sample_rate: u32, min_freq: f64, max_freq: f64, threshold: f64) -> Self {
        assert!(min_freq < max_freq);
        let tau_min = (sample_rate as f64 / max_freq) as usize;
        let tau_max = (sample_rate as f64 / min_freq) as usize;
        Self {
            sample_rate,
            tau_min,
            tau_max,
            threshold,
        }
    }

    /// Returns the minimum size for input with the current settings.
    ///
    /// When using the yin algorithm, the input frame must have
    /// at least this many samples to get decent results.
    /// Because of this, the method [`detect_frequency`] panics
    /// if the input frame does not have enough samples.
    ///
    /// [`detect_frequency`]: Self::detect_frequency
    pub fn minimum_frame_size(&self) -> usize {
        2 * self.tau_max
    }

    /// Try to estimate the frequency of the input frame.
    ///
    /// # Panics
    ///
    /// Panics if the frame does not have enough samples (the minimum can be found with [`minimum_frame_size`]).
    ///
    /// [`minimum_frame_size`]: Self::minimum_frame_size
    pub fn detect_frequency(&self, frame: &[f64]) -> Option<f64> {
        assert!(frame.len() >= self.tau_max);
        let differences = self.difference_function(&frame);
        let cmndf = Self::cmndf(&differences);
        let tau_star =
            Self::absolute_threshold(&cmndf, self.tau_min, self.tau_max, self.threshold)?;
        let tau_interpolated = Self::parabolic_interpolation(&cmndf, tau_star);
        let frequency = self.sample_rate as f64 / tau_interpolated;
        Some(frequency)
    }

    /// Step 2. The difference function d_tau (figure 6).
    fn difference_function(&self, frame: &[f64]) -> Vec<f64> {
        // TODO: implement with fft
        let mut differences = vec![0.; self.tau_max];
        for tau in 1..self.tau_max {
            for i in 0..frame.len() - self.tau_max {
                let delta = frame[i] - frame[i + tau];
                differences[tau] += delta * delta;
            }
        }
        differences
    }

    // Step 3. Cumulative mean normalized difference function d'_tau (figure 8).
    fn cmndf(differences: &[f64]) -> Vec<f64> {
        let mut cmndf = vec![1.; differences.len()];

        let mut sum = 0.;
        for tau in 1..differences.len() {
            sum += differences[tau];
            if sum != 0. {
                cmndf[tau] = differences[tau] * tau as f64 / sum
            }
        }

        cmndf
    }

    // Step 4. Absolute threshold.
    fn absolute_threshold(
        cmndf: &[f64],
        tau_min: usize,
        tau_max: usize,
        threshold: f64,
    ) -> Option<usize> {
        for mut tau in tau_min..tau_max {
            if cmndf[tau] < threshold {
                while tau + 1 < tau_max && cmndf[tau + 1] < cmndf[tau] {
                    tau += 1;
                }
                return Some(tau);
            }
        }
        return None;
    }

    // Step 5. Parabolic interpolation.
    fn parabolic_interpolation(cmndf: &[f64], tau: usize) -> f64 {
        if tau == 0 || tau + 1 >= cmndf.len() {
            return tau as f64;
        }
        let y0 = cmndf[tau - 1];
        let y1 = cmndf[tau];
        let y2 = cmndf[tau + 1];

        let denom = y0 - 2. * y1 + y2;
        if denom.abs() < 1e-12 {
            return tau as f64;
        }
        tau as f64 + 0.5 * (y0 - y2) / denom
    }
}

#[cfg(test)]
mod tests {
    use autotune_rs::assert_nearly_equal;
    use std::f64::consts::TAU;

    use super::Yin;

    fn generate_sin(sample_rate: u32, frequency: f64, sample_count: usize) -> Vec<f64> {
        let step = TAU * frequency / sample_rate as f64;
        (0..sample_count)
            .map(move |i| (step * i as f64).sin())
            .collect()
    }

    /// Generates a sum of sine waves
    ///
    /// E.g. for `frequency_amplitudes` = `[(440., 1.), (600., 0.3)]`, the waveform is `1 * sin(440hz), 0.3 * sin(600hz)`.
    fn generate_sin_sum(
        sample_rate: u32,
        frequency_amplitudes: &[(f64, f64)],
        sample_count: usize,
    ) -> Vec<f64> {
        let step_amplitudes: Vec<(f64, f64)> = frequency_amplitudes
            .iter()
            .map(|(frequency, amplitude)| {
                let step = TAU * frequency / sample_rate as f64;
                (step, *amplitude)
            })
            .collect();
        (0..sample_count)
            .map(|i| {
                step_amplitudes
                    .iter()
                    .map(|(step, amplitude)| amplitude * (step * i as f64).sin())
                    .sum()
            })
            .collect()
    }

    #[test]
    fn test_basic_sine() {
        let sample_rate = 48_000;
        let frequency = 440.;

        let min_freq = 60.;
        let max_freq = 1000.;
        let threshold = 0.1;
        let yin = Yin::new(sample_rate, min_freq, max_freq, threshold);

        let sample_count = yin.minimum_frame_size();
        let wave = generate_sin(sample_rate, frequency, sample_count);

        let detected = yin.detect_frequency(&wave).unwrap_or_default();
        assert_nearly_equal!(detected, frequency, 0.05);
    }

    #[test]
    fn test_harmonics() {
        let sample_rate = 48_000;
        let frequency_main = 440.;
        let frequency_amplitudes = [
            (frequency_main, 1.),
            (frequency_main * 2., 1.),
            (frequency_main * 3., 1.),
        ];

        let min_freq = 60.;
        let max_freq = 1000.;
        // Threshold is higher for noisy signal
        let threshold = 0.25;
        let yin = Yin::new(sample_rate, min_freq, max_freq, threshold);

        let sample_count = 2 * yin.minimum_frame_size();
        let wave = generate_sin_sum(sample_rate, &frequency_amplitudes, sample_count);

        let detected = yin.detect_frequency(&wave).unwrap_or_default();
        // Raise the margin for a noisy signal.
        assert_nearly_equal!(detected, frequency_main, 0.05);
    }

    #[test]
    fn test_low_sample_rate_sine() {
        let sample_rate = 10_000;
        let frequency = 440.;

        let min_freq = 60.;
        let max_freq = 1000.;
        let threshold = 0.1;
        let yin = Yin::new(sample_rate, min_freq, max_freq, threshold);

        let sample_count = yin.minimum_frame_size();
        let wave = generate_sin(sample_rate, frequency, sample_count);

        let detected = yin.detect_frequency(&wave).unwrap_or_default();
        assert_nearly_equal!(detected, frequency, 0.5);
    }

    #[test]
    fn test_low_sample_rate_harmonics() {
        let sample_rate = 10_000;
        let frequency_main = 440.;
        let frequency_amplitudes = [
            (frequency_main, 1.),
            (frequency_main * 2., 1.),
            (frequency_main * 3., 1.),
        ];

        let min_freq = 60.;
        let max_freq = 1000.;
        // Threshold is higher for noisy signal
        let threshold = 0.25;
        let yin = Yin::new(sample_rate, min_freq, max_freq, threshold);

        let sample_count = yin.minimum_frame_size();
        let wave = generate_sin_sum(sample_rate, &frequency_amplitudes, sample_count);

        let detected = yin.detect_frequency(&wave).unwrap_or_default();
        // Raise the margin for a noisy signal.
        assert_nearly_equal!(detected, frequency_main, 0.5);
    }
}
