/// Visual style of a live cell (letters match the approved mockups).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CellStyle {
    Candy,
    Rounded,
    Dots,
    Neon,
}

impl CellStyle {
    pub const ALL: [CellStyle; 4] = [
        CellStyle::Candy,
        CellStyle::Rounded,
        CellStyle::Dots,
        CellStyle::Neon,
    ];

    pub fn next(self) -> Self {
        match self {
            CellStyle::Candy => CellStyle::Rounded,
            CellStyle::Rounded => CellStyle::Dots,
            CellStyle::Dots => CellStyle::Neon,
            CellStyle::Neon => CellStyle::Candy,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CellStyle::Candy => "C — Bonbon",
            CellStyle::Rounded => "A — Arrondis",
            CellStyle::Dots => "B — Pastilles",
            CellStyle::Neon => "D — Néon",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub style: CellStyle,
    /// Main color, sRGB components in 0..1.
    pub color: [f32; 3],
    /// Hue/lightness variation amount, 0..1.
    pub variation: f32,
    pub effect_age: bool,
    pub effect_trails: bool,
    pub effect_anim: bool,
    pub degraded: bool,
    /// Randomization density.
    pub density: f32,
    pub panel_open: bool,
    pub tool_erase: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            style: CellStyle::Candy,
            // #3fb8a8
            color: [
                0x3f as f32 / 255.0,
                0xb8 as f32 / 255.0,
                0xa8 as f32 / 255.0,
            ],
            variation: 0.35,
            effect_age: false,
            effect_trails: false,
            effect_anim: false,
            degraded: false,
            density: 0.35,
            panel_open: true,
            tool_erase: false,
        }
    }
}

impl Settings {
    /// Whether the simulation must maintain effect metadata (age, trails).
    pub fn effects_needed(&self) -> bool {
        !self.degraded && (self.effect_age || self.effect_trails)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_cycle() {
        assert_eq!(CellStyle::Candy.next(), CellStyle::Rounded);
        assert_eq!(CellStyle::Rounded.next(), CellStyle::Dots);
        assert_eq!(CellStyle::Dots.next(), CellStyle::Neon);
        assert_eq!(CellStyle::Neon.next(), CellStyle::Candy);
    }
}
