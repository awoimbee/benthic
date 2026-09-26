//! Breathing-gas mixtures.

use serde::{Deserialize, Serialize};

/// A breathing gas, stored as permille of oxygen and helium.
///
/// Air is represented explicitly as 21.0% O2 / 0% He so that the native JSON
/// format is never ambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GasMix {
    /// Oxygen fraction in permille.
    pub o2_permille: u16,
    /// Helium fraction in permille.
    pub he_permille: u16,
}

/// Standard air.
pub const AIR: GasMix = GasMix {
    o2_permille: 210,
    he_permille: 0,
};
/// Pure oxygen.
pub const OXYGEN: GasMix = GasMix {
    o2_permille: 1000,
    he_permille: 0,
};

impl Default for GasMix {
    fn default() -> Self {
        AIR
    }
}

impl GasMix {
    pub const fn new(o2_permille: u16, he_permille: u16) -> Self {
        Self {
            o2_permille,
            he_permille,
        }
    }

    /// Build a mix from percentages, e.g. `GasMix::percent(32.0, 0.0)`.
    pub fn percent(o2: f64, he: f64) -> Self {
        Self {
            o2_permille: (o2 * 10.0).round() as u16,
            he_permille: (he * 10.0).round() as u16,
        }
    }

    pub fn o2_percent(self) -> f64 {
        self.o2_permille as f64 / 10.0
    }

    pub fn he_percent(self) -> f64 {
        self.he_permille as f64 / 10.0
    }

    /// Nitrogen fraction in permille (remainder of O2 and He).
    pub fn n2_permille(self) -> i32 {
        1000 - self.o2_permille as i32 - self.he_permille as i32
    }

    pub fn is_air(self) -> bool {
        self.o2_permille == AIR.o2_permille && self.he_permille == 0
    }

    pub fn is_oxygen(self) -> bool {
        self.o2_permille >= 995 && self.he_permille == 0
    }

    pub fn is_trimix(self) -> bool {
        self.he_permille > 0
    }

    pub fn is_nitrox(self) -> bool {
        self.he_permille == 0 && self.o2_permille > AIR.o2_permille
    }

    /// A short human-readable name: "Air", "EAN32", "Tx21/35", "O2".
    pub fn name(self) -> String {
        if self.is_air() {
            "Air".into()
        } else if self.is_oxygen() {
            "O2".into()
        } else if self.is_trimix() {
            format!("Tx{}/{}", self.o2_permille / 10, self.he_permille / 10)
        } else if self.o2_permille > AIR.o2_permille {
            format!("EAN{}", self.o2_permille / 10)
        } else {
            format!("{}% O2", self.o2_percent())
        }
    }
}
