use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FactorPoint {
    pub x: f32,
    pub y: f32,
}

impl Default for FactorPoint {
    fn default() -> Self {
        Self { x: 0.5, y: 0.5 }
    }
}

impl FactorPoint {
    pub fn clamp(mut self) -> Self {
        self.x = self.x.clamp(0.0, 1.0);
        self.y = self.y.clamp(0.0, 1.0);
        self
    }

    pub fn director_params(&self) -> DirectorParams {
        let p = self.clamp();
        DirectorParams {
            divergence: p.x,
            specificity: 1.0 - p.x,
            mood: p.y,
            technical: 1.0 - p.y,
        }
    }

    pub fn ps_params(&self) -> PsParams {
        let p = self.clamp();
        PsParams {
            detail_level: p.x,
            lighting_drama: p.y,
        }
    }

    pub fn quadrant_label(&self, x_low: &str, x_high: &str, y_low: &str, y_high: &str) -> String {
        let p = self.clamp();
        let x_label = if p.x < 0.5 { x_low } else { x_high };
        let y_label = if p.y < 0.5 { y_low } else { y_high };
        format!("{x_label}·{y_label}")
    }
}

#[derive(Debug, Clone)]
pub struct DirectorParams {
    pub divergence: f32,
    pub specificity: f32,
    pub mood: f32,
    pub technical: f32,
}

#[derive(Debug, Clone)]
pub struct PsParams {
    pub detail_level: f32,
    pub lighting_drama: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct FactorPreset {
    pub name: &'static str,
    pub director: FactorPoint,
    pub ps: FactorPoint,
}

pub const FACTOR_PRESETS: &[FactorPreset] = &[
    FactorPreset {
        name: "电影感",
        director: FactorPoint { x: 0.35, y: 0.75 },
        ps: FactorPoint { x: 0.7, y: 0.85 },
    },
    FactorPreset {
        name: "产品图",
        director: FactorPoint { x: 0.85, y: 0.25 },
        ps: FactorPoint { x: 0.6, y: 0.2 },
    },
    FactorPreset {
        name: "概念艺术",
        director: FactorPoint { x: 0.2, y: 0.55 },
        ps: FactorPoint { x: 0.45, y: 0.65 },
    },
];
