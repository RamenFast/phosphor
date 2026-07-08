// SPDX-License-Identifier: GPL-3.0-or-later
//! The CPU-raster worker (issue #5). Chrome used to share its thread
//! with the CPU scope, so a slow raster dragged the whole UI (Ben,
//! install night: "the UI slows down when the visual does… can't we
//! multi thread it?"). Now the worker owns the CpuRenderer: the shell
//! drops a latest-wins job in the mailbox and uploads the newest
//! published frame — the chrome thread never waits on a raster.
//!
//! Latest-wins twice over: an unserviced job is REPLACED by the next
//! one (no backlog when the raster is slower than the frame cadence),
//! and an untaken frame is replaced by the newer one. The scope simply
//! updates at whatever rate the raster sustains while the chrome stays
//! at the panel's.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use phosphor_render_cpu::CpuRenderer;

pub(crate) struct RasterJob {
    pub segments: Vec<[f32; 5]>,
    pub width: usize,
    pub height: usize,
    pub beam_focus: f32,
    pub persistence: f32,
    pub theme: phosphor_beam::Theme,
    pub grid_enabled: bool,
    pub grid_spacing_fraction: f32,
    pub display_scale: f32,
    /// 1 = opaque scope; lower = the live glass pane. Offline renders
    /// never pass through here — their CpuRenderer stays at 1.0.
    pub scope_alpha: f32,
}

pub(crate) struct RasterFrame {
    pub pixels: Vec<u8>,
    pub width: usize,
    pub height: usize,
    /// advance+composite cost on the worker — the HUD's raster number
    pub raster_ms: f32,
    sequence: u64,
}

#[derive(Default)]
struct Slots {
    job: Option<RasterJob>,
    frame: Option<RasterFrame>,
    quit: bool,
}

pub(crate) struct RasterWorker {
    slots: Arc<(Mutex<Slots>, Condvar)>,
    handle: Option<std::thread::JoinHandle<()>>,
    taken_sequence: u64,
}

impl RasterWorker {
    pub fn spawn() -> RasterWorker {
        let slots: Arc<(Mutex<Slots>, Condvar)> = Arc::default();
        let worker_slots = slots.clone();
        let handle = std::thread::Builder::new()
            .name("phosphor-raster".into())
            .spawn(move || worker_loop(&worker_slots))
            .expect("raster worker thread");
        RasterWorker {
            slots,
            handle: Some(handle),
            taken_sequence: 0,
        }
    }

    /// Replace any queued job (latest wins; never blocks).
    pub fn submit(&self, job: RasterJob) {
        let (lock, condvar) = &*self.slots;
        lock.lock().unwrap().job = Some(job);
        condvar.notify_one();
    }

    /// The newest published frame, once (None until a newer one lands).
    pub fn take_frame(&mut self) -> Option<RasterFrame> {
        let (lock, _) = &*self.slots;
        let mut slots = lock.lock().unwrap();
        match &slots.frame {
            Some(frame) if frame.sequence > self.taken_sequence => {
                self.taken_sequence = frame.sequence;
                slots.frame.take()
            }
            _ => None,
        }
    }
}

impl Drop for RasterWorker {
    fn drop(&mut self) {
        let (lock, condvar) = &*self.slots;
        lock.lock().unwrap().quit = true;
        condvar.notify_one();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn worker_loop(slots: &Arc<(Mutex<Slots>, Condvar)>) {
    let mut renderer: Option<CpuRenderer> = None;
    let mut sequence = 0u64;
    loop {
        let job = {
            let (lock, condvar) = &**slots;
            let mut guard = lock.lock().unwrap();
            loop {
                if guard.quit {
                    return;
                }
                if let Some(job) = guard.job.take() {
                    break job;
                }
                guard = condvar.wait(guard).unwrap();
            }
        };

        let started = Instant::now();
        // keep the energy planes across frames; rebuild only when the
        // scope size actually changed (decay continuity, v3 law)
        let mut target = renderer
            .take()
            .filter(|r| r.width() == job.width && r.height() == job.height)
            .unwrap_or_else(|| CpuRenderer::new(job.width, job.height, 1));
        target.beam_focus = job.beam_focus;
        target.persistence = job.persistence;
        target.theme = job.theme;
        target.grid_enabled = job.grid_enabled;
        target.grid_spacing_fraction = job.grid_spacing_fraction;
        target.display_scale = job.display_scale;
        target.scope_alpha = job.scope_alpha;
        // gamma-space premultiply — the form the GPU glass shader
        // emits and egui's (One, OneMinusSrcAlpha) blend consumes;
        // identity at scope_alpha 1.0, so glass-off frames keep
        // today's bytes exactly
        target.premultiplied = true;
        target.advance(&job.segments);
        let pixels = target.composite().to_vec();
        sequence += 1;

        let frame = RasterFrame {
            pixels,
            width: job.width,
            height: job.height,
            raster_ms: started.elapsed().as_secs_f32() * 1e3,
            sequence,
        };
        let (lock, _) = &**slots;
        lock.lock().unwrap().frame = Some(frame);
        renderer = Some(target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(width: usize, tag: f32) -> RasterJob {
        RasterJob {
            segments: vec![[10.0, 10.0, 50.0, 50.0, tag]],
            width,
            height: 64,
            beam_focus: 1.6,
            persistence: 0.7,
            theme: phosphor_beam::Theme::preset("P7 Green").unwrap(),
            grid_enabled: false,
            grid_spacing_fraction: 0.1125,
            display_scale: 1.0,
            scope_alpha: 1.0,
        }
    }

    #[test]
    fn publishes_frames_and_takes_once() {
        let mut worker = RasterWorker::spawn();
        worker.submit(job(64, 0.4));
        let frame = loop {
            if let Some(frame) = worker.take_frame() {
                break frame;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        assert_eq!((frame.width, frame.height), (64, 64));
        assert_eq!(frame.pixels.len(), 64 * 64 * 4);
        assert!(frame.raster_ms >= 0.0);
        // taken once: no newer frame yet
        assert!(worker.take_frame().is_none());
    }

    #[test]
    fn latest_job_wins_and_resize_is_honored() {
        let mut worker = RasterWorker::spawn();
        for i in 0..24 {
            worker.submit(job(if i % 2 == 0 { 96 } else { 128 }, 0.2));
        }
        worker.submit(job(128, 0.9));
        // the final job must eventually publish at ITS size
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(frame) = worker.take_frame()
                && frame.width == 128
            {
                break;
            }
            assert!(Instant::now() < deadline, "no 128-wide frame");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn drop_joins_cleanly() {
        let worker = RasterWorker::spawn();
        worker.submit(job(64, 0.1));
        drop(worker); // must not hang
    }

    #[test]
    fn glass_job_publishes_translucent_frames() {
        // opaque job first: every pixel must stay A=255 (glass off)
        let mut worker = RasterWorker::spawn();
        worker.submit(job(64, 0.4));
        let take = |worker: &mut RasterWorker| loop {
            if let Some(frame) = worker.take_frame() {
                break frame;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        let opaque = take(&mut worker);
        assert!(opaque.pixels.chunks(4).all(|px| px[3] == 255),
                "opaque job leaked translucency");

        // glass job: the background corner (far from the beam) must
        // carry the pane's alpha, not 255
        let mut glass = job(64, 0.4);
        glass.scope_alpha = 0.5;
        worker.submit(glass);
        let frame = take(&mut worker);
        assert!(frame.pixels[3] < 255,
                "glass scope_alpha never reached the worker's renderer");
    }
}
