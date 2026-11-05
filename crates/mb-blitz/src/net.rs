//! What a page's images and stylesheets load from: files next to the page
//! (or under /tmp, where a panel's data collector writes), read at once.

use std::path::{Path, PathBuf};

use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};

pub struct Files {
    base: PathBuf,
}

impl Files {
    pub fn new(base: PathBuf) -> Self {
        Self { base }
    }

    /// The file a `file:` URL names, if it is inside the page's directory or /tmp.
    fn path(&self, req: &Request) -> Option<PathBuf> {
        if req.url.scheme() != "file" {
            return None;
        }
        let real = req.url.to_file_path().ok()?.canonicalize().ok()?;
        let tmp = Path::new("/tmp").canonicalize().ok();
        (real.starts_with(&self.base) || tmp.is_some_and(|t| real.starts_with(t))).then_some(real)
    }
}

impl NetProvider for Files {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        match self.path(&request).and_then(|p| std::fs::read(p).ok()) {
            Some(data) => handler.bytes(request.url.to_string(), Bytes::from(data)),
            None => log::warn!("net: {} not loaded (no network, or outside the page's directory)", request.url),
        }
    }
}
