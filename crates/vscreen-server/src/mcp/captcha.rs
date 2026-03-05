use std::sync::Arc;
use std::time::Duration;

use crate::supervisor::InstanceSupervisor;
use super::VScreenMcpServer;

impl VScreenMcpServer {
    /// Check if the reCAPTCHA is solved by looking for the green checkmark in the anchor iframe.
    pub(super) async fn captcha_is_solved(&self, sup: &Arc<InstanceSupervisor>) -> bool {
        let frame_tree = match sup.get_frame_tree().await {
            Ok(ft) => ft,
            Err(_) => return false,
        };
        let child_frames = frame_tree
            .get("frameTree")
            .or(Some(&frame_tree))
            .and_then(|ft| ft.get("childFrames"))
            .and_then(|cf| cf.as_array());

        if let Some(frames) = child_frames {
            for child in frames {
                let url = child
                    .get("frame")
                    .and_then(|f| f.get("url"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("");
                if !url.contains("recaptcha") || !url.contains("anchor") {
                    continue;
                }
                let frame_id = child
                    .get("frame")
                    .and_then(|f| f.get("id"))
                    .and_then(|id| id.as_str())
                    .unwrap_or("");
                if frame_id.is_empty() {
                    continue;
                }
                let check_js = r#"(function(){
                    const anchor = document.getElementById('recaptcha-anchor');
                    if (anchor && anchor.getAttribute('aria-checked') === 'true') return 'solved';
                    return 'unsolved';
                })()"#;
                if let Ok(val) = sup.evaluate_js_in_frame(check_js, frame_id).await {
                    let s = val.as_str().unwrap_or("");
                    if s == "solved" {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Find the reCAPTCHA challenge (bframe) iframe and return its page-space bounds.
    pub(super) async fn find_captcha_challenge_iframe(
        &self,
        sup: &Arc<InstanceSupervisor>,
    ) -> Option<(f64, f64, f64, f64)> {
        let js = r#"JSON.stringify(Array.from(document.querySelectorAll('iframe')).map(f => {
            const r = f.getBoundingClientRect();
            return {
                title: f.title || '',
                src: f.src || '',
                x: r.left + window.scrollX,
                y: r.top + window.scrollY,
                width: r.width,
                height: r.height,
                visible: r.width > 0 && r.height > 0 && r.top > -9000,
            };
        }))"#;
        let result = sup.evaluate_js(js).await.ok()?;
        let s = result.as_str().unwrap_or("[]");
        let iframes: Vec<serde_json::Value> = serde_json::from_str(s).unwrap_or_default();

        iframes.iter().find_map(|f| {
            let title = f.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let src = f.get("src").and_then(|v| v.as_str()).unwrap_or("");
            let visible = f.get("visible").and_then(|v| v.as_bool()).unwrap_or(false);
            if !visible {
                return None;
            }
            if title.contains("challenge") || src.contains("bframe") {
                let x = f.get("x").and_then(|v| v.as_f64())?;
                let y = f.get("y").and_then(|v| v.as_f64())?;
                let w = f.get("width").and_then(|v| v.as_f64())?;
                let h = f.get("height").and_then(|v| v.as_f64())?;
                if w > 100.0 && h > 100.0 {
                    return Some((x, y, w, h));
                }
            }
            None
        })
    }

    /// Check if the reCAPTCHA challenge has expired by looking for expiry text
    /// in both the main page and the anchor iframe.
    pub(super) async fn captcha_check_expired(&self, sup: &Arc<InstanceSupervisor>) -> bool {
        let js = r#"document.body.innerText.includes('Verification challenge expired') ||
                     document.body.innerText.includes('challenge expired')"#;
        if sup.evaluate_js(js).await.ok().and_then(|v| v.as_bool()).unwrap_or(false) {
            return true;
        }
        // Also check inside the anchor iframe
        let frame_tree = match sup.get_frame_tree().await {
            Ok(ft) => ft,
            Err(_) => return false,
        };
        let child_frames = frame_tree
            .get("frameTree")
            .or(Some(&frame_tree))
            .and_then(|ft| ft.get("childFrames"))
            .and_then(|cf| cf.as_array());
        if let Some(frames) = child_frames {
            for child in frames {
                let url = child.get("frame").and_then(|f| f.get("url")).and_then(|u| u.as_str()).unwrap_or("");
                if !url.contains("recaptcha") { continue; }
                let fid = child.get("frame").and_then(|f| f.get("id")).and_then(|id| id.as_str()).unwrap_or("");
                if fid.is_empty() { continue; }
                let check = r#"document.body.innerText.includes('expired') || document.body.innerText.includes('Expired')"#;
                if let Ok(val) = sup.evaluate_js_in_frame(check, fid).await {
                    if val.as_bool().unwrap_or(false) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Find the reCAPTCHA challenge (bframe) iframe's CDP frame ID for running JS inside it.
    pub(super) async fn find_captcha_bframe_id(&self, sup: &Arc<InstanceSupervisor>) -> Option<String> {
        let frame_tree = sup.get_frame_tree().await.ok()?;
        let child_frames = frame_tree
            .get("frameTree")
            .or(Some(&frame_tree))
            .and_then(|ft| ft.get("childFrames"))
            .and_then(|cf| cf.as_array())?;

        for child in child_frames {
            let url = child
                .get("frame")
                .and_then(|f| f.get("url"))
                .and_then(|u| u.as_str())
                .unwrap_or("");
            if url.contains("recaptcha") && url.contains("bframe") {
                let frame_id = child
                    .get("frame")
                    .and_then(|f| f.get("id"))
                    .and_then(|id| id.as_str())
                    .unwrap_or("");
                if !frame_id.is_empty() {
                    return Some(frame_id.to_string());
                }
            }
        }
        None
    }

    /// Check the challenge iframe DOM for tile replacement indicators.
    /// Returns a JSON object with:
    ///   - `has_new_images`: true if "Please also check the new images" is visible
    ///   - `tiles_animating`: count of tiles currently in animation/transition
    ///   - `header_text`: the current instruction header text
    pub(super) async fn captcha_challenge_state(
        &self,
        sup: &Arc<InstanceSupervisor>,
        bframe_id: &str,
    ) -> Option<serde_json::Value> {
        let js = r#"(function(){
            var header = document.querySelector('.rc-imageselect-desc-wrapper, .rc-imageselect-desc, .rc-imageselect-desc-no-canonical');
            var headerText = header ? header.innerText.trim() : '';
            var hasNewImages = headerText.toLowerCase().includes('also check the new images') ||
                               headerText.toLowerCase().includes('check the new images');
            var tiles = document.querySelectorAll('.rc-imageselect-tile');
            var animating = 0;
            tiles.forEach(function(t) {
                var style = window.getComputedStyle(t);
                if (style.transition && style.transition !== 'none' && style.transition !== 'all 0s ease 0s') {
                    var td = t.querySelector('.rc-image-tile-33, .rc-image-tile-44, .rc-image-tile-11');
                    if (td && td.classList.contains('rc-image-tile-33')) animating++;
                }
            });
            var dynamicTiles = document.querySelectorAll('.rc-imageselect-dynamic-selected');
            return JSON.stringify({
                has_new_images: hasNewImages,
                tiles_animating: animating,
                dynamic_selected: dynamicTiles.length,
                header_text: headerText,
                is_dynamic: document.querySelectorAll('.rc-imageselect-dynamic-selected, .rc-imageselect-tileselected').length > 0 || hasNewImages
            });
        })()"#;
        let val = sup.evaluate_js_in_frame(js, bframe_id).await.ok()?;
        let s = val.as_str().unwrap_or("{}");
        serde_json::from_str(s).ok()
    }

    /// Wait for tile replacement animations to settle by polling DOM state.
    /// Returns when no tiles are animating or after max_wait.
    pub(super) async fn wait_for_tile_animation(
        &self,
        sup: &Arc<InstanceSupervisor>,
        bframe_id: &str,
        max_wait: Duration,
    ) {
        let start = std::time::Instant::now();
        let poll_interval = Duration::from_millis(200);
        loop {
            if start.elapsed() >= max_wait {
                break;
            }
            tokio::time::sleep(poll_interval).await;
            if let Some(state) = self.captcha_challenge_state(sup, bframe_id).await {
                let animating = state.get("tiles_animating").and_then(|v| v.as_u64()).unwrap_or(0);
                let dynamic = state.get("dynamic_selected").and_then(|v| v.as_u64()).unwrap_or(0);
                if animating == 0 && dynamic == 0 {
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// reCAPTCHA grid geometry helpers
// ---------------------------------------------------------------------------

/// Compute tile center coordinates in page-space for a reCAPTCHA challenge grid.
///
/// The challenge iframe contains:
/// - A blue header (~95px tall)
/// - A tile grid that fills most of the remaining space
/// - A footer bar (~60px tall) with VERIFY/SKIP
///
/// Grid tiles are equal-sized squares arranged in either 3x3 or 4x4 layout.
/// We detect the grid size from the iframe dimensions: wider iframes (>350px
/// with height >500px) use 3x3 when wide, 4x4 when the header is shorter.
pub(super) fn compute_grid_positions(
    iframe_x: f64,
    iframe_y: f64,
    iframe_w: f64,
    iframe_h: f64,
) -> Vec<[f64; 2]> {
    let header_h = 95.0;
    let footer_h = 65.0;
    let grid_h = iframe_h - header_h - footer_h;
    let grid_y = iframe_y + header_h;
    let grid_x = iframe_x;

    // Determine grid dimensions: if each tile would be >110px in a 4x4, it's 3x3
    let tile_w_4 = iframe_w / 4.0;
    let (cols, rows) = if tile_w_4 > 95.0 && grid_h / 4.0 > 95.0 {
        // Could be either; use aspect ratio — 3x3 tiles are ~130px, 4x4 ~97px
        if grid_h / 3.0 > 120.0 && iframe_w / 3.0 > 120.0 {
            (3usize, 3usize)
        } else {
            (4, 4)
        }
    } else {
        (3, 3)
    };

    let tile_w = iframe_w / cols as f64;
    let tile_h = grid_h / rows as f64;

    let mut positions = Vec::with_capacity(cols * rows);
    for row in 0..rows {
        for col in 0..cols {
            positions.push([
                grid_x + col as f64 * tile_w + tile_w / 2.0,
                grid_y + row as f64 * tile_h + tile_h / 2.0,
            ]);
        }
    }
    positions
}

/// Compute the VERIFY/SKIP button center in page-space.
pub(super) fn compute_verify_button(
    iframe_x: f64,
    iframe_y: f64,
    iframe_w: f64,
    iframe_h: f64,
) -> [f64; 2] {
    // VERIFY button sits in the footer, right-aligned, ~50px from right edge
    [iframe_x + iframe_w - 50.0, iframe_y + iframe_h - 30.0]
}

#[cfg(test)]
mod captcha_grid_tests {
    use super::*;

    #[test]
    fn grid_3x3_positions() {
        let positions = compute_grid_positions(85.0, 84.0, 400.0, 580.0);
        assert_eq!(positions.len(), 9);
        // First tile center should be roughly at (85 + 400/3/2, 84 + 95 + 420/3/2)
        let first = positions[0];
        assert!(first[0] > 100.0 && first[0] < 200.0, "x={}", first[0]);
        assert!(first[1] > 200.0 && first[1] < 300.0, "y={}", first[1]);
        // Last tile center
        let last = positions[8];
        assert!(last[0] > 380.0 && last[0] < 500.0, "x={}", last[0]);
        assert!(last[1] > 480.0 && last[1] < 600.0, "y={}", last[1]);
    }

    #[test]
    fn verify_button_position() {
        let btn = compute_verify_button(85.0, 84.0, 400.0, 580.0);
        assert!(btn[0] > 400.0 && btn[0] < 500.0, "x={}", btn[0]);
        assert!(btn[1] > 600.0 && btn[1] < 680.0, "y={}", btn[1]);
    }
}
