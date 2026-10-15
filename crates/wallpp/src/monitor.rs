use crate::wallpp::provider::types::{FilterCriteria, Orientation};

/// Represents the physical or logical resolution of a display monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Monitor {
    pub width: u32,
    pub height: u32,
}

impl Monitor {
    /// Total screen pixel area.
    pub fn area(&self) -> u64 {
        (self.width as u64) * (self.height as u64)
    }

    /// Determine display orientation.
    pub fn orientation(&self) -> Orientation {
        if self.width > self.height {
            Orientation::Landscape
        } else if self.height > self.width {
            Orientation::Portrait
        } else {
            Orientation::Square
        }
    }
}

/// Detect connected monitors using display-info.
pub fn detect_monitors() -> Vec<Monitor> {
    let mut monitors = Vec::new();

    if let Ok(displays) = display_info::DisplayInfo::all() {
        for d in displays {
            let scale = if d.scale_factor > 0.0 {
                d.scale_factor
            } else {
                1.0
            };
            let w = (d.width as f32 * scale).round() as u32;
            let h = (d.height as f32 * scale).round() as u32;
            if w > 0 && h > 0 {
                monitors.push(Monitor {
                    width: w,
                    height: h,
                });
            }
        }
    }

    monitors
}

/// Find the monitor with the largest pixel area.
pub fn get_biggest_monitor() -> Option<Monitor> {
    detect_monitors().into_iter().max_by_key(|m| m.area())
}

/// Compute filter criteria based on the configured minimum percentage of the biggest display.
pub fn compute_filter_criteria(min_percentage: u32) -> FilterCriteria {
    if min_percentage == 0 {
        return FilterCriteria {
            min_width: None,
            min_height: None,
            orientation: None,
        };
    }

    if let Some(biggest) = get_biggest_monitor() {
        let pct = (min_percentage as f64) / 100.0;
        let min_w = ((biggest.width as f64) * pct).round() as u32;
        let min_h = ((biggest.height as f64) * pct).round() as u32;
        FilterCriteria {
            min_width: Some(min_w),
            min_height: Some(min_h),
            orientation: Some(biggest.orientation()),
        }
    } else {
        FilterCriteria {
            min_width: None,
            min_height: None,
            orientation: None,
        }
    }
}
