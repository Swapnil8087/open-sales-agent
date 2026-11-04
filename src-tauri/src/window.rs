//! Notch-aware placement of the floating command bar.

/// A monitor work area in physical pixels (excludes menu bar / notch region).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

pub const BAR_WIDTH_LOGICAL: f64 = 680.0;
pub const TOP_MARGIN_LOGICAL: f64 = 8.0;

/// Top-center position (physical px) for a window of `win_width` physical px.
pub fn compute_position(area: WorkArea, win_width: u32) -> (i32, i32) {
    let slack = area.width.saturating_sub(win_width) as i32;
    let x = area.x + slack / 2;
    let y = area.y + (TOP_MARGIN_LOGICAL * area.scale).round() as i32;
    (x, y)
}

/// Index of the area containing the point, if any.
pub fn area_containing(areas: &[WorkArea], px: i32, py: i32) -> Option<usize> {
    areas.iter().position(|a| {
        px >= a.x
            && py >= a.y
            && (px as i64) < a.x as i64 + a.width as i64
            && (py as i64) < a.y as i64 + a.height as i64
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(x: i32, y: i32, w: u32, h: u32, s: f64) -> WorkArea {
        WorkArea { x, y, width: w, height: h, scale: s }
    }

    #[test]
    fn centers_horizontally_below_menu_bar_on_retina() {
        // 3024x1964 panel, work area starts below 74px (37pt) menu bar.
        let a = area(0, 74, 3024, 1890, 2.0);
        let win_w = (BAR_WIDTH_LOGICAL * 2.0) as u32; // 1360
        let (x, y) = compute_position(a, win_w);
        assert_eq!(x, (3024 - 1360) / 2);
        assert_eq!(y, 74 + 16); // 8pt margin * 2
    }

    #[test]
    fn respects_secondary_monitor_offset() {
        let a = area(-1920, 25, 1920, 1055, 1.0);
        let (x, y) = compute_position(a, 680);
        assert_eq!(x, -1920 + (1920 - 680) / 2);
        assert_eq!(y, 25 + 8);
    }

    #[test]
    fn window_wider_than_screen_clamps_to_left_edge() {
        let a = area(100, 0, 500, 800, 1.0);
        let (x, _) = compute_position(a, 680);
        assert_eq!(x, 100);
    }

    #[test]
    fn finds_area_containing_cursor() {
        let areas = [area(0, 0, 1920, 1080, 1.0), area(1920, 0, 1920, 1080, 1.0)];
        assert_eq!(area_containing(&areas, 100, 100), Some(0));
        assert_eq!(area_containing(&areas, 2500, 500), Some(1));
        assert_eq!(area_containing(&areas, 5000, 5000), None);
    }
}
