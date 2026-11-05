//! What a page's images, stylesheets and scripts load from: files next to
//! the page (or under /tmp, where a panel's data collector writes), and with
//! the `http` feature the web. Loads are synchronous: a page shows complete.

use std::path::{Path, PathBuf};

use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};

pub struct Files {
    base: PathBuf,
}

/// GET `url` (http, https): the body, or None after logging why.
#[cfg(feature = "http")]
pub fn http_get(url: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::sync::OnceLock;
    // One agent: its connections (and TLS sessions) are reused across loads.
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    let agent = AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(15)))
            .user_agent("Mozilla/5.0 (X11; Linux armv7l) MiniBrowser/0.8 Blitz")
            .build()
            .into()
    });
    let t = std::time::Instant::now();
    let r = agent.get(url).call();
    if std::env::var_os("MB_TRACE").is_some() {
        eprintln!("net: {url} {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    }
    match r {
        Ok(mut r) => {
            let mut out = Vec::new();
            r.body_mut().as_reader().take(32 << 20).read_to_end(&mut out).ok()?;
            Some(out)
        }
        Err(e) => {
            log::warn!("net: {url}: {e}");
            None
        }
    }
}

#[cfg(not(feature = "http"))]
pub fn http_get(url: &str) -> Option<Vec<u8>> {
    log::warn!("net: {url}: built without http");
    None
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
        let data = match request.url.scheme() {
            "http" | "https" => http_get(request.url.as_str()),
            _ => self.path(&request).and_then(|p| std::fs::read(p).ok()),
        };
        match data {
            Some(data) => handler.bytes(request.url.to_string(), Bytes::from(data)),
            None => log::warn!("net: {} not loaded (no network, or outside the page's directory)", request.url),
        }
    }
}
