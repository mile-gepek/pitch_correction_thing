use std::fmt::Display;

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

    use autotune_rs::assert_nearly_equal;

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
        assert_nearly_equal!(frequency, 440.);
    }

    #[test]
    fn frequency_c0() {
        let pitch = Pitch::new(Note::C, 0);
        let frequency = pitch.frequency();
        assert_nearly_equal!(frequency, 16.351597831287375);
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
