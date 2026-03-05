//! ScreenshotWatcher: continuously monitors a clipped region of a browser page
//! and fires events when content changes using perceptual hashing.

use std::collections::HashMap;
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::supervisor::InstanceSupervisor;

/// Region to clip for monitoring
#[derive(Debug, Clone)]
pub struct ClipRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Grid dimensions for cell subdivision
#[derive(Debug, Clone)]
pub struct GridDims {
    pub rows: u32,
    pub cols: u32,
}

/// Event fired when grid cells change
#[derive(Debug, Clone)]
pub struct GridChangeEvent {
    /// Indices of cells that changed (0-based, row-major order)
    pub changed_cells: Vec<usize>,
    /// Full screenshot bytes (PNG)
    pub full_screenshot: Bytes,
    /// Cropped cell images for changed cells only
    pub cell_crops: HashMap<usize, Bytes>,
}

/// Handle to control the watcher
#[derive(Debug)]
pub struct WatcherHandle {
    task: tokio::task::JoinHandle<()>,
    cancel: CancellationToken,
}

impl WatcherHandle {
    pub fn stop(&self) {
        self.cancel.cancel();
    }
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

/// Raw grayscale cell data
#[derive(Debug, Clone)]
pub struct CellData {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Average hash: resize to 8x8, compute mean, set bits > mean
pub fn average_hash(cell: &CellData) -> u64 {
    let mut small = [0u32; 64];
    let mut counts = [0u32; 64];

    for y in 0..cell.height {
        let sy = (y * 8 / cell.height) as usize;
        for x in 0..cell.width {
            let sx = (x * 8 / cell.width) as usize;
            let idx = sy * 8 + sx;
            small[idx] += cell.pixels[(y * cell.width + x) as usize] as u32;
            counts[idx] += 1;
        }
    }

    let avg_pixels: Vec<u8> = small
        .iter()
        .zip(counts.iter())
        .map(|(&s, &c)| if c > 0 { (s / c) as u8 } else { 0 })
        .collect();

    let mean: u32 = avg_pixels.iter().map(|&p| p as u32).sum::<u32>() / 64;

    let mut hash: u64 = 0;
    for (i, &pixel) in avg_pixels.iter().enumerate() {
        if (pixel as u32) > mean {
            hash |= 1 << i;
        }
    }
    hash
}

/// Hamming distance between two hashes
pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

async fn capture_clip(
    sup: &std::sync::Arc<InstanceSupervisor>,
    clip: &ClipRegion,
) -> Option<Vec<u8>> {
    let clip_tuple = (clip.x, clip.y, clip.width, clip.height);
    sup.capture_screenshot_clip("png", None, Some(clip_tuple))
        .await
        .ok()
        .map(|b| b.to_vec())
}

fn split_into_cells(png_data: &[u8], grid: &GridDims) -> Option<Vec<CellData>> {
    let img = image::load_from_memory(png_data).ok()?.to_luma8();
    let (w, h) = img.dimensions();
    let cell_w = w / grid.cols;
    let cell_h = h / grid.rows;

    let mut cells = Vec::new();
    for row in 0..grid.rows {
        for col in 0..grid.cols {
            let x = col * cell_w;
            let y = row * cell_h;
            let sub = image::imageops::crop_imm(&img, x, y, cell_w, cell_h).to_image();
            cells.push(CellData {
                pixels: sub.into_raw(),
                width: cell_w,
                height: cell_h,
            });
        }
    }
    Some(cells)
}

fn encode_cell_png(cell: &CellData) -> Option<Vec<u8>> {
    use image::codecs::png::PngEncoder;
    use image::ImageEncoder;

    let mut buf = Vec::new();
    let encoder = PngEncoder::new(&mut buf);
    encoder
        .write_image(&cell.pixels, cell.width, cell.height, image::ExtendedColorType::L8)
        .ok()?;
    Some(buf)
}

async fn watcher_loop(
    sup: std::sync::Arc<InstanceSupervisor>,
    clip: ClipRegion,
    grid: GridDims,
    interval_ms: u64,
    tx: broadcast::Sender<GridChangeEvent>,
    cancel: CancellationToken,
) {
    let total_cells = (grid.rows * grid.cols) as usize;
    let mut prev_hashes: Vec<u64> = vec![0; total_cells];
    let mut first_run = true;

    loop {
        if cancel.is_cancelled() {
            break;
        }

        let screenshot = match capture_clip(&sup, &clip).await {
            Some(data) => data,
            None => {
                tokio::time::sleep(Duration::from_millis(interval_ms)).await;
                continue;
            }
        };

        let cells = match split_into_cells(&screenshot, &grid) {
            Some(c) => c,
            None => {
                tokio::time::sleep(Duration::from_millis(interval_ms)).await;
                continue;
            }
        };

        let current_hashes: Vec<u64> = cells.iter().map(average_hash).collect();

        let mut changed = Vec::new();
        for i in 0..total_cells {
            if first_run || hamming_distance(prev_hashes[i], current_hashes[i]) > 5 {
                changed.push(i);
            }
        }

        if !changed.is_empty() {
            let cell_crops: HashMap<usize, Bytes> = changed
                .iter()
                .filter_map(|&idx| {
                    encode_cell_png(&cells[idx]).map(|png| (idx, Bytes::from(png)))
                })
                .collect();

            let event = GridChangeEvent {
                changed_cells: changed,
                full_screenshot: Bytes::from(screenshot.clone()),
                cell_crops,
            };
            let _ = tx.send(event);
        }

        prev_hashes = current_hashes;
        first_run = false;

        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_millis(interval_ms)) => {}
        }
    }
}

/// ScreenshotWatcher: monitors a clipped region and broadcasts change events.
#[derive(Debug)]
pub struct ScreenshotWatcher;

impl ScreenshotWatcher {
    /// Start a background watcher.
    /// Returns a handle (to stop it) and a receiver for change events.
    pub fn start(
        sup: std::sync::Arc<InstanceSupervisor>,
        clip: ClipRegion,
        grid: GridDims,
        interval_ms: u64,
    ) -> (WatcherHandle, broadcast::Receiver<GridChangeEvent>) {
        let (tx, rx) = broadcast::channel(16);
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let task = tokio::spawn(async move {
            watcher_loop(sup, clip, grid, interval_ms, tx, cancel_clone).await;
        });

        let handle = WatcherHandle { task, cancel };
        (handle, rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_average_hash_identical() {
        let cell = CellData {
            pixels: vec![128; 64],
            width: 8,
            height: 8,
        };
        let h1 = average_hash(&cell);
        let h2 = average_hash(&cell);
        assert_eq!(h1, h2);
        assert_eq!(hamming_distance(h1, h2), 0);
    }

    #[test]
    fn test_average_hash_different() {
        // Left half black, right half white - produces different hash than uniform
        let mut pixels = vec![0u8; 64];
        for i in 32..64 {
            pixels[i] = 255;
        }
        let cell1 = CellData {
            pixels: vec![0; 64],
            width: 8,
            height: 8,
        };
        let cell2 = CellData {
            pixels,
            width: 8,
            height: 8,
        };
        let h1 = average_hash(&cell1);
        let h2 = average_hash(&cell2);
        assert_ne!(h1, h2);
        assert!(hamming_distance(h1, h2) > 0);
    }

    #[test]
    fn test_hamming_distance() {
        assert_eq!(hamming_distance(0, 0), 0);
        assert_eq!(hamming_distance(0, 1), 1);
        assert_eq!(hamming_distance(0xFF, 0), 8);
        assert_eq!(hamming_distance(u64::MAX, 0), 64);
    }

    #[test]
    fn test_split_and_hash_uniform() {
        // Create a simple 16x16 grayscale PNG (4x4 grid = 16 cells)
        // Use checkerboard pattern per cell so each cell has internal variation and different
        // cells can have different hashes based on their pattern phase
        let mut pixels = vec![0u8; 16 * 16];
        for (i, p) in pixels.iter_mut().enumerate() {
            let row = i / 16;
            let col = i % 16;
            let cell_row = row / 4;
            let cell_col = col / 4;
            // Checkerboard: alternate 0/255 based on position, with phase per cell
            let phase = (cell_row + cell_col) % 2;
            let local_odd = (row + col) % 2;
            *p = if (local_odd + phase) % 2 == 0 {
                0
            } else {
                255
            };
        }
        let img = image::GrayImage::from_raw(16, 16, pixels).expect("valid image");
        let mut png_buf = Vec::new();
        {
            use image::codecs::png::PngEncoder;
            use image::ImageEncoder;
            let encoder = PngEncoder::new(&mut png_buf);
            encoder
                .write_image(img.as_raw(), 16, 16, image::ExtendedColorType::L8)
                .expect("encode");
        }

        let grid = GridDims { rows: 4, cols: 4 };
        let cells = split_into_cells(&png_buf, &grid).expect("split");
        assert_eq!(cells.len(), 16);

        let hashes: Vec<u64> = cells.iter().map(average_hash).collect();
        // Adjacent cells with different checkerboard phases should have different hashes
        assert_ne!(hashes[0], hashes[1]);
        assert_ne!(hashes[0], hashes[4]);
    }
}
