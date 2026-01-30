use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Spacing {
    Linear,
    Geometric,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepSchedule {
    pub min: f64,
    pub max: f64,
    pub steps: usize,
    pub spacing: Spacing,
}

impl StepSchedule {
    pub fn cumulative(&self) -> Vec<f64> {
        let min = self.min.max(0.0);
        let max = self.max.max(min);
        let steps = self.steps.max(1);
        match self.spacing {
            Spacing::Linear => {
                if steps == 1 {
                    return vec![max];
                }
                let step = (max - min) / (steps as f64 - 1.0);
                (0..steps).map(|i| min + step * i as f64).collect()
            }
            Spacing::Geometric => {
                if steps == 1 {
                    return vec![max];
                }
                let min = min.max(f64::EPSILON);
                let ratio = (max / min).powf(1.0 / (steps as f64 - 1.0));
                (0..steps).map(|i| min * ratio.powf(i as f64)).collect()
            }
        }
    }

    pub fn segments(&self) -> Vec<f64> {
        let cumulative = self.cumulative();
        let mut prev = 0.0;
        cumulative
            .into_iter()
            .map(|v| {
                let seg = (v - prev).max(0.0);
                prev = v;
                seg
            })
            .filter(|v| *v > 0.0)
            .collect()
    }
}
