//! The two themes the run's pages are drawn in, paper and night, defined once: the map's page
//! reads them from its data, the summary page from here.

use crate::blob::{string, strings};
use crate::rules::DELAY_BAND_LABELS;

/// A theme's colours, as `#rrggbb`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// The page's background.
    pub surface: &'static str,
    /// The text, and a softer and a muted text.
    pub ink: &'static str,
    /// The softer text.
    pub ink2: &'static str,
    /// The muted text: labels, notes.
    pub muted: &'static str,
    /// The network's grey, and the cards' tint and border.
    pub base: &'static str,
    /// The openmobisim mark.
    pub mark: &'static str,
    /// Whether the road ribbons glow (night).
    pub glow: bool,
    /// The kinds of transit service (rail, metro, tram, bus, ferry, other), and a pair's
    /// journeys, the most travelled first: one palette for both, as approved.
    pub kinds: [&'static str; 6],
    /// The modes: car, bike, walk, transit.
    pub modes: [&'static str; 4],
    /// The delay's ramp, from free flow to the worst.
    pub ramp_delay: [&'static str; 4],
    /// The volume's ramp, from the least to the most.
    pub ramp_volume: &'static [&'static str],
}

/// The paper theme: dark ink on a light surface.
pub const PAPER: Theme = Theme {
    surface: "#fbfcfd",
    ink: "#0a1626",
    ink2: "#3d4c5f",
    muted: "#55657a",
    base: "#c5cfda",
    mark: "#04938e",
    glow: false,
    kinds: ["#2a78d6", "#eb6834", "#1baf7a", "#eda100", "#e87ba4", "#008300"],
    modes: ["#ef4b53", "#04938e", "#ce9c0f", "#099acf"],
    ramp_delay: ["#04938e", "#ce9c0f", "#ef4b53", "#6b0014"],
    ramp_volume: &["#0eb5f1", "#099acf", "#057da9", "#055a7a"],
};

/// The night theme: light ink on a dark surface.
pub const NIGHT: Theme = Theme {
    surface: "#0a1220",
    ink: "#f2f6fa",
    ink2: "#cdd6e1",
    muted: "#93a2b6",
    base: "#364966",
    mark: "#40c8c1",
    glow: true,
    kinds: ["#3987e5", "#d95926", "#199e70", "#c98500", "#d55181", "#008300"],
    modes: ["#ef4b53", "#40c8c1", "#dcaa1e", "#0eb5f1"],
    ramp_delay: ["#40c8c1", "#dcaa1e", "#ef4b53", "#a8101f"],
    ramp_volume: &["#057da9", "#099acf", "#0eb5f1", "#67cdfe", "#ace2ff"],
};

/// The steps the delay's ramp is read at: the band colours are its steps `round(i / 3 × 23)`.
const STEPS: f64 = 24.0;

/// A `#rrggbb` colour's three channels.
fn channels(hex: &str) -> [f64; 3] {
    let at = |i: usize| f64::from(u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0));
    [at(1), at(3), at(5)]
}

impl Theme {
    /// The colours of the delay's four bands (also volume ÷ capacity's): the delay ramp read at
    /// `i / 3` of its length, on a ramp of 24 steps, each channel interpolated and rounded.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a handful of ramp stops and colour channels from 0 to 255"
    )]
    pub fn bands(&self) -> [String; 4] {
        let stops: Vec<[f64; 3]> = self.ramp_delay.iter().map(|h| channels(h)).collect();
        let last = (stops.len() - 1) as f64;
        core::array::from_fn(|i| {
            let n = (DELAY_BAND_LABELS.len() - 1) as f64;
            let step = (i as f64 / n * (STEPS - 1.0)).round();
            let t = step / (STEPS - 1.0) * last;
            let k = t.floor().min(last - 1.0);
            let (a, b) = (stops[k as usize], stops[k as usize + 1]);
            let c: Vec<String> = (0..3)
                .map(|j| format!("{:02x}", (a[j] + (b[j] - a[j]) * (t - k)).round() as u8))
                .collect();
            format!("#{}", c.concat())
        })
    }

    /// The theme as JSON, for the map's page.
    fn json(&self) -> String {
        let bands = self.bands();
        format!(
            "{{\"surface\":{},\"ink\":{},\"ink2\":{},\"muted\":{},\"base\":{},\"mark\":{},\
             \"glow\":{},\"kinds\":{},\"modes\":{},\"ramp_delay\":{},\"ramp_volume\":{},\
             \"bands\":{}}}",
            string(self.surface),
            string(self.ink),
            string(self.ink2),
            string(self.muted),
            string(self.base),
            string(self.mark),
            self.glow,
            strings(self.kinds),
            strings(self.modes),
            strings(self.ramp_delay),
            strings(self.ramp_volume.iter().copied()),
            strings(bands.iter().map(String::as_str)),
        )
    }
}

/// Both themes as JSON, `{"paper": ..., "night": ...}`, for the map's page.
pub(crate) fn tokens() -> String {
    format!("{{\"paper\":{},\"night\":{}}}", PAPER.json(), NIGHT.json())
}
