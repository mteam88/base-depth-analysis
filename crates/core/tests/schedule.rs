use base_depth_core::schedule::{Spacing, StepSchedule};

#[test]
fn geometric_schedule_hits_endpoints() {
    let schedule = StepSchedule {
        min: 1.0,
        max: 100.0,
        steps: 3,
        spacing: Spacing::Geometric,
    };
    let values = schedule.cumulative();
    assert_eq!(values.len(), 3);
    assert!((values[0] - 1.0).abs() < 1e-6);
    assert!((values[1] - 10.0).abs() < 1e-6);
    assert!((values[2] - 100.0).abs() < 1e-6);
}
