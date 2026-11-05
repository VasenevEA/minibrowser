//! DRM/KMS output: two dumb buffers, page flips on vsync, so a frame is never
//! shown half-drawn. Used when the board has a DRM driver (`/dev/dri/card0`);
//! otherwise mb-fb writes to fbdev.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::{AsFd, BorrowedFd};

use drm::buffer::DrmFourcc;
use drm::control::{connector, crtc, dumbbuffer::DumbBuffer, framebuffer, Device as ControlDevice, Event, Mode, PageFlipFlags};
use drm::Device;

use crate::Screen;

struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl Device for Card {}
impl ControlDevice for Card {}

struct Buffer {
    fb: framebuffer::Handle,
    pixels: &'static mut [u8],
    pitch: usize,
}

pub struct Kms {
    card: Card,
    crtc: crtc::Handle,
    size: (u32, u32),
    buffers: [Buffer; 2],
    /// The buffer being drawn into (the other one is on screen).
    back: usize,
    /// What the last frame changed: the back buffer is a frame behind, so it
    /// gets those rectangles too.
    last: Vec<[i32; 4]>,
}

impl Kms {
    pub fn open(path: &str) -> io::Result<Kms> {
        let card = Card(OpenOptions::new().read(true).write(true).open(path)?);
        // Fails when another program (a compositor) drives the display.
        card.acquire_master_lock()?;
        let res = card.resource_handles()?;
        let con = res
            .connectors()
            .iter()
            .filter_map(|h| card.get_connector(*h, false).ok())
            .find(|c| c.state() == connector::State::Connected)
            .ok_or_else(|| io::Error::other("no connected display"))?;
        let mode: Mode = *con.modes().first().ok_or_else(|| io::Error::other("display without modes"))?;
        let crtc = con
            .current_encoder()
            .and_then(|e| card.get_encoder(e).ok())
            .and_then(|e| e.crtc())
            .or_else(|| res.crtcs().first().copied())
            .ok_or_else(|| io::Error::other("no crtc"))?;
        let (w, h) = mode.size();
        let size = (w as u32, h as u32);
        let make = || -> io::Result<Buffer> {
            // Lives as long as the program: the mapping borrows it.
            let db: &'static mut DumbBuffer = Box::leak(Box::new(card.create_dumb_buffer(size, DrmFourcc::Xrgb8888, 32)?));
            let fb = card.add_framebuffer(&*db, 24, 32)?;
            let pitch = drm::buffer::Buffer::pitch(&*db) as usize;
            let map = card.map_dumb_buffer(db)?;
            let pixels: &'static mut [u8] = Box::leak(Box::new(map)).as_mut();
            pixels.fill(0);
            Ok(Buffer { fb, pixels, pitch })
        };
        let buffers = [make()?, make()?];
        card.set_crtc(crtc, Some(buffers[0].fb), (0, 0), &[con.handle()], Some(mode))?;
        Ok(Kms { card, crtc, size, buffers, back: 1, last: Vec::new() })
    }
}

impl Screen for Kms {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn present(&mut self, rgba: &[u8], w: u32, rects: &[[i32; 4]]) {
        let (sx, sy) = (w.min(self.size.0) as i32, (self.size.1 as usize).min(rgba.len() / (w as usize * 4)) as i32);
        let src_line = w as usize * 4;
        let buf = &mut self.buffers[self.back];
        for r in rects.iter().chain(&self.last) {
            let (x0, y0, x1, y1) = (r[0].max(0), r[1].max(0), r[2].min(sx), r[3].min(sy));
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            for y in y0 as usize..y1 as usize {
                let src = &rgba[y * src_line + x0 as usize * 4..y * src_line + x1 as usize * 4];
                let dst = &mut buf.pixels[y * buf.pitch + x0 as usize * 4..y * buf.pitch + x1 as usize * 4];
                // XRGB8888 little-endian: B, G, R, X.
                for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    d.copy_from_slice(&[s[2], s[1], s[0], 0xff]);
                }
            }
        }
        if self.card.page_flip(self.crtc, buf.fb, PageFlipFlags::EVENT, None).is_ok() {
            // Wait for the flip: the buffer now on screen must not be drawn into.
            'wait: while let Ok(events) = self.card.receive_events() {
                for e in events {
                    if let Event::PageFlip(_) = e {
                        break 'wait;
                    }
                }
            }
        }
        self.back ^= 1;
        self.last = rects.to_vec();
    }
}
