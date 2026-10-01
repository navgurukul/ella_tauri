//! Keeps the window the same composition on every laptop.
//!
//! The layout is drawn for a window of at least `DESIGN_WIDTH` by
//! `DESIGN_HEIGHT` CSS pixels. A Mac's 1440 by 900 window has that, but Windows
//! display scaling hands the webview far fewer: a 1080p laptop at the usual
//! 150% is 1280 by about 650 once the taskbar and title bar are off, and a
//! 1366 by 768 screen is about 690 tall. Squeezed into that, Home hid the
//! talk's blurb, cramped the topics and gave the right rail a scrollbar.
//!
//! So a window smaller than the design zooms the webview out until the page
//! has the design's room again, the way Ctrl+minus would in a browser, and the
//! window opens no bigger than the screen it is on. `tauri.conf.json`'s
//! minimum, 960 by 500, fits a 1366 by 768 screen at 125% with its taskbar.

use std::sync::Mutex;

use tauri::{LogicalSize, Runtime, WebviewWindow};

/// The smallest page, in CSS pixels, the layout keeps its full design at. The
/// height sits just above `styles.css`'s compact `max-height: 820px` rules.
pub const DESIGN_WIDTH: f64 = 1240.0;
pub const DESIGN_HEIGHT: f64 = 830.0;

/// Below this the type gets too small to read; a smaller window gets the
/// compact layout at this zoom instead.
pub const MIN_ZOOM: f64 = 0.65;

/// Zoom moves in steps of 1/40, so a drag-resize does not relayout the page on
/// every pixel.
const ZOOM_STEPS: f64 = 40.0;

/// The zoom the page was last given, and the size in logical pixels it was
/// given it for.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fitted {
    zoom: f64,
    width: f64,
    height: f64,
}

static FITTED: Mutex<Option<Fitted>> = Mutex::new(None);

/// The zoom that gives a window of `width` by `height` logical pixels at least
/// the design's room, rounded down to a step and never above 1.
pub fn zoom_for(width: f64, height: f64) -> f64 {
    let fit = (width / DESIGN_WIDTH).min(height / DESIGN_HEIGHT);
    ((fit * ZOOM_STEPS).floor() / ZOOM_STEPS).clamp(MIN_ZOOM, 1.0)
}

/// What the page should be fitted to now that it measures `width` by `height`
/// logical pixels, or `None` to leave it be. A page measured in whole CSS
/// pixels comes out a fraction different at each zoom, so a change under a
/// pixel is that rounding, not a resize, and is ignored: otherwise a size on
/// the edge between two zooms would flip between them.
fn refit(last: Option<Fitted>, width: f64, height: f64) -> Option<Fitted> {
    if let Some(last) = last {
        if (last.width - width).abs() < 1.0 && (last.height - height).abs() < 1.0 {
            return None;
        }
    }
    Some(Fitted { zoom: zoom_for(width, height), width, height })
}

/// The inner size that fits a window with `frame` (title bar and borders) on a
/// screen's `work_area`, all in logical pixels; `None` when it already fits.
pub fn fitted_inner_size(
    inner: (f64, f64),
    frame: (f64, f64),
    work_area: (f64, f64),
) -> Option<(f64, f64)> {
    let width = inner.0.min(work_area.0 - frame.0);
    let height = inner.1.min(work_area.1 - frame.1);
    (width < inner.0 || height < inner.1).then_some((width, height))
}

/// Shrinks a window that opened bigger than its screen's work area, which the
/// 1440 by 900 default is on most Windows laptops, so none of it hides behind
/// the taskbar. On Windows it is then maximized, as such a small screen wants.
pub fn fit_to_screen<R: Runtime>(window: &WebviewWindow<R>) -> tauri::Result<()> {
    let Some(monitor) = window.current_monitor()? else {
        return Ok(());
    };
    let scale = window.scale_factor()?;
    let work = monitor.work_area().size.to_logical::<f64>(scale);
    let inner = window.inner_size()?.to_logical::<f64>(scale);
    let outer = window.outer_size()?.to_logical::<f64>(scale);
    let frame = (outer.width - inner.width, outer.height - inner.height);
    if let Some((width, height)) =
        fitted_inner_size((inner.width, inner.height), frame, (work.width, work.height))
    {
        window.set_size(LogicalSize::new(width, height))?;
        window.center()?;
        if cfg!(target_os = "windows") {
            window.maximize()?;
        }
    }
    Ok(())
}

/// Zooms the page for the window's inner size, before the page has loaded.
///
/// This is only a first guess: on a Mac the content view runs under the title
/// bar, so the inner size counts the title bar's height too. The page then
/// reports its own size through `zoom_to_page`, which has the last word.
pub fn zoom_to_window<R: Runtime>(window: &WebviewWindow<R>) {
    let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) else {
        return;
    };
    let logical = size.to_logical::<f64>(scale);
    zoom_page(window, logical.width, logical.height);
}

/// Zooms the page for the size it measures itself, `width` by `height` CSS
/// pixels at the zoom it has now. The page sends it when it loads and on
/// every resize.
pub fn zoom_to_page<R: Runtime>(window: &WebviewWindow<R>, width: f64, height: f64) {
    let zoom = current().map_or(1.0, |fitted| fitted.zoom);
    zoom_page(window, width * zoom, height * zoom);
}

fn current() -> Option<Fitted> {
    *FITTED.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn zoom_page<R: Runtime>(window: &WebviewWindow<R>, width: f64, height: f64) {
    // Minimized, a window on Windows reports no size at all.
    if width < 1.0 || height < 1.0 {
        return;
    }
    let mut fitted = FITTED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(next) = refit(*fitted, width, height) else {
        return;
    };
    let zoom = fitted.map_or(1.0, |last| last.zoom);
    if next.zoom == zoom {
        *fitted = Some(next);
        return;
    }
    match window.set_zoom(next.zoom) {
        Ok(()) => {
            eprintln!("[window] {width:.0}x{height:.0}: page zoom {}", next.zoom);
            *fitted = Some(next);
        }
        Err(error) => eprintln!("[window] could not zoom the page to {}: {error}", next.zoom),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_with_the_designs_room_is_not_zoomed() {
        assert_eq!(zoom_for(1440.0, 900.0), 1.0);
        assert_eq!(zoom_for(DESIGN_WIDTH, DESIGN_HEIGHT), 1.0);
        assert_eq!(zoom_for(2560.0, 1440.0), 1.0);
    }

    #[test]
    fn a_short_window_zooms_out_until_the_page_has_the_designs_height() {
        // A 1080p laptop at 150%, maximized: 1280 by about 645.
        let zoom = zoom_for(1280.0, 645.0);
        assert_eq!(zoom, 0.775);
        assert!(645.0 / zoom >= DESIGN_HEIGHT);
        assert!(1280.0 / zoom >= DESIGN_WIDTH);
        // A 1366 by 768 screen at 100%.
        let zoom = zoom_for(1366.0, 690.0);
        assert!(690.0 / zoom >= DESIGN_HEIGHT);
        assert!(690.0 / (zoom + 1.0 / ZOOM_STEPS) < DESIGN_HEIGHT);
    }

    #[test]
    fn a_narrow_window_zooms_out_for_its_width() {
        let zoom = zoom_for(1100.0, 900.0);
        assert!(1100.0 / zoom >= DESIGN_WIDTH);
        assert!(zoom < 1.0);
    }

    #[test]
    fn a_tiny_window_stops_at_the_smallest_readable_zoom() {
        assert_eq!(zoom_for(700.0, 400.0), MIN_ZOOM);
    }

    #[test]
    fn a_page_measured_a_fraction_differently_keeps_its_zoom() {
        let last = refit(None, 1280.0, 645.0).unwrap();
        assert_eq!(last.zoom, 0.775);
        // 645 logical pixels at 0.775 is 832.26 CSS pixels; the page reports
        // 832, which is 644.8 back in logical pixels.
        assert_eq!(refit(Some(last), 1651.0 * 0.775, 832.0 * 0.775), None);
        let resized = refit(Some(last), 1280.0, 700.0).unwrap();
        assert_eq!(resized.zoom, 0.825);
    }

    #[test]
    fn a_window_that_fits_its_screen_is_left_alone() {
        assert_eq!(fitted_inner_size((1440.0, 900.0), (0.0, 28.0), (1512.0, 944.0)), None);
    }

    #[test]
    fn a_window_taller_than_its_screen_shrinks_to_the_work_area() {
        // The 1440 by 900 default on a 1080p screen at 150%, taskbar at the
        // bottom: 1280 by 672 to work in, a 31-pixel title bar, 7-pixel borders.
        assert_eq!(
            fitted_inner_size((1440.0, 900.0), (14.0, 38.0), (1280.0, 672.0)),
            Some((1266.0, 634.0))
        );
        // At 125% only the height is short.
        assert_eq!(
            fitted_inner_size((1440.0, 900.0), (14.0, 38.0), (1536.0, 824.0)),
            Some((1440.0, 786.0))
        );
    }
}
