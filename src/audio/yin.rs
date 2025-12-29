//! Implementation of the YIN pitch detection algorithm.
//!
//! To begin, construct a new Yin instance, and use the detect_pitch method on the samples

pub struct Yin {
    sample_rate: u32,
    min_freq: f64,
    max_freq: f64,
    threshold: f64,
}

impl Yin {
    pub fn new(sample_rate: u32, min_freq: f64, max_freq: f64, threshold: f64) -> Self {
        assert!(min_freq < max_freq);
        Self {
            sample_rate,
            min_freq,
            max_freq,
            threshold,
        }
    }

    pub fn detect_pitch(&self, frame: &[f64]) -> Option<f64> {
        let tau_min = (self.sample_rate as f64 / self.max_freq) as usize;
        let tau_max = (self.sample_rate as f64 / self.min_freq) as usize;
        assert!(tau_max < frame.len());

        let differences = Self::difference_function(&frame, tau_max);
        let cmndf = Self::cmndf(&differences);
        let Some(tau_star) = Self::absolute_threshold(&cmndf, tau_min, tau_max, self.threshold)
        else {
            return None;
        };
        let tau_interpolated = Self::parabolic_interpolation(&cmndf, tau_star);
        let frequency = self.sample_rate as f64 / tau_interpolated;
        Some(frequency)
    }

    fn difference_function(frame: &[f64], tau_max: usize) -> Vec<f64> {
        let mut differences = vec![0.; tau_max];
        for tau in 1..tau_max {
            for i in 0..frame.len() - tau_max {
                let delta = frame[i] - frame[i + tau];
                differences[tau] += delta * delta;
            }
        }
        differences
    }

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
