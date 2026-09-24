//! Website artwork is fetched off the UI thread, directly from the saved site's
//! public HTTPS origin. No login URL path, cookies, credentials or proxy service.
use curl::easy::{Easy, List};
use gpui::RenderImage;
use image::{ImageFormat, ImageReader};
use regex::Regex;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
    net::{IpAddr, ToSocketAddrs},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use url::Url;

const MAX_IMAGE: usize = 1_048_576;
const MAX_HTML: usize = 131_072;
pub const MAX_PENDING: usize = 4;
const MAX_CACHED: usize = 256;

#[derive(Default)]
pub struct SiteIcons {
    pub pending: BTreeSet<String>,
    pub cancel: Arc<AtomicBool>,
    entries: BTreeMap<String, (Option<Arc<RenderImage>>, u64)>,
    clock: u64,
}
impl SiteIcons {
    pub fn clear(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        *self = Self::default();
    }
    pub fn lookup(&mut self, host: &str) -> Option<Option<Arc<RenderImage>>> {
        self.clock += 1;
        self.entries.get_mut(host).map(|(image, used)| {
            *used = self.clock;
            image.clone()
        })
    }
    pub fn insert(&mut self, host: String, image: Option<Arc<RenderImage>>) {
        if self.entries.len() >= MAX_CACHED
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| used)
                .map(|(h, _)| h.clone())
        {
            self.entries.remove(&oldest);
        }
        self.clock += 1;
        self.entries.insert(host, (image, self.clock));
    }
}

pub fn site_host(website: &str) -> Option<String> {
    let url = Url::parse(website).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.port().is_some_and(|p| p != 80 && p != 443)
    {
        return None;
    }
    let host = url.domain()?.trim_end_matches('.').to_ascii_lowercase();
    if !host.contains('.')
        || host.parse::<IpAddr>().is_ok()
        || [
            "localhost",
            "local",
            "internal",
            "lan",
            "home",
            "test",
            "example",
            "invalid",
            "onion",
        ]
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
    {
        return None;
    }
    Some(host)
}

fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_documentation()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 192 && b == 0 && (c == 0 || c == 2))
                && !(a == 192 && b == 88 && c == 99)
                && !(a == 198 && (b == 18 || b == 19))
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            s[0] & 0xe000 == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] <= 0x01ff || s[1] == 0x0db8))
                && !(s[0] == 0x3fff && s[1] <= 0x0fff)
        }
    }
}

/// Resolve once and pin the public addresses for every request. Curl never follows
/// redirects, uses ambient proxies or re-resolves an unchecked address.
pub fn fetch(host: &str, cancelled: &AtomicBool) -> Option<Arc<RenderImage>> {
    if cancelled.load(Ordering::Relaxed) {
        return None;
    }
    let addresses: Vec<_> = (host, 443)
        .to_socket_addrs()
        .ok()?
        .map(|s| s.ip())
        .filter(|ip| public_address(*ip))
        .take(4)
        .collect();
    if addresses.is_empty() || cancelled.load(Ordering::Relaxed) {
        return None;
    }
    let pinned = addresses
        .iter()
        .map(|ip| match ip {
            IpAddr::V4(ip) => ip.to_string(),
            IpAddr::V6(ip) => format!("[{ip}]"),
        })
        .collect::<Vec<_>>()
        .join(",");
    let origin = Url::parse(&format!("https://{host}/")).ok()?;
    fetch_from_origin(&origin, |url, limit| {
        request(url, &pinned, limit, cancelled)
    })
}

fn request(url: &Url, pinned: &str, limit: usize, cancelled: &AtomicBool) -> Option<Vec<u8>> {
    if cancelled.load(Ordering::Relaxed) {
        return None;
    }
    let mut easy = Easy::new();
    easy.url(url.as_str()).ok()?;
    easy.proxy("").ok()?;
    easy.follow_location(false).ok()?;
    easy.ssl_verify_peer(true).ok()?;
    easy.ssl_verify_host(true).ok()?;
    easy.connect_timeout(Duration::from_secs(3)).ok()?;
    easy.timeout(Duration::from_secs(5)).ok()?;
    easy.max_filesize(limit as u64).ok()?;
    easy.useragent("ME-Website-Icons/1.0").ok()?;
    let mut resolve = List::new();
    resolve
        .append(&format!("{}:443:{pinned}", url.host_str()?))
        .ok()?;
    easy.resolve(resolve).ok()?;
    let mut bytes = Vec::new();
    {
        let mut transfer = easy.transfer();
        transfer
            .write_function(|chunk| {
                if cancelled.load(Ordering::Relaxed)
                    || bytes.len().saturating_add(chunk.len()) > limit
                {
                    return Ok(0);
                }
                bytes.extend_from_slice(chunk);
                Ok(chunk.len())
            })
            .ok()?;
        transfer.perform().ok()?;
    }
    (easy.response_code().ok()? == 200).then_some(bytes)
}

fn fetch_from_origin(
    origin: &Url,
    mut get: impl FnMut(&Url, usize) -> Option<Vec<u8>>,
) -> Option<Arc<RenderImage>> {
    let mut candidates = get(origin, MAX_HTML)
        .map(|html| icon_links(origin, &String::from_utf8_lossy(&html)))
        .unwrap_or_default();
    // Conventional fallbacks cover sites without parseable HTML or icon links.
    candidates.extend([
        origin.join("favicon.ico").ok()?,
        origin.join("apple-touch-icon.png").ok()?,
    ]);
    let mut seen = BTreeSet::new();
    for url in candidates
        .into_iter()
        .filter(|url| seen.insert(url.as_str().to_owned()))
        .take(4)
    {
        if let Some(image) = get(&url, MAX_IMAGE).and_then(|bytes| decode(&bytes)) {
            return Some(image);
        }
    }
    None
}

fn icon_links(origin: &Url, html: &str) -> Vec<Url> {
    static LINKS: OnceLock<Regex> = OnceLock::new();
    static ATTRS: OnceLock<Regex> = OnceLock::new();
    let links = LINKS.get_or_init(|| Regex::new(r"(?is)<link\b([^<>]{0,2048})>").unwrap());
    let attrs = ATTRS.get_or_init(|| {
        Regex::new(r#"(?is)([a-z][a-z0-9_-]*)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+))"#)
            .unwrap()
    });
    links
        .captures_iter(html)
        .filter_map(|link| {
            let mut rel = String::new();
            let mut href = String::new();
            for attr in attrs.captures_iter(&link[1]) {
                let value = (2..=4)
                    .find_map(|i| attr.get(i))
                    .map(|m| m.as_str())
                    .unwrap_or("");
                match attr[1].to_ascii_lowercase().as_str() {
                    "rel" => rel = value.to_ascii_lowercase(),
                    "href" => href = value.replace("&amp;", "&"),
                    _ => (),
                }
            }
            if !rel.split_ascii_whitespace().any(|r| {
                matches!(
                    r,
                    "icon" | "apple-touch-icon" | "apple-touch-icon-precomposed"
                )
            }) || href.is_empty()
            {
                return None;
            }
            let mut url = origin.join(&href).ok()?;
            if url.origin() != origin.origin()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return None;
            }
            url.set_fragment(None);
            Some(url)
        })
        .take(2)
        .collect()
}

/// Bounded, single-frame raster decoding; SVG and animation are intentionally not
/// executed. Re-encode to a small BGRA surface before handing anything to GPUI.
pub fn decode(bytes: &[u8]) -> Option<Arc<RenderImage>> {
    if bytes.len() > MAX_IMAGE {
        return None;
    }
    let format = image::guess_format(bytes).ok()?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Ico | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return None;
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(8 * 1024 * 1024);
    reader.limits(limits);
    let mut pixels = reader.decode().ok()?.thumbnail(64, 64).to_rgba8();
    if pixels.width() == 0 || pixels.height() == 0 {
        return None;
    }
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_public_origins_are_eligible_and_login_paths_never_leave_the_app() {
        assert_eq!(
            site_host("https://alice:password@EXAMPLE.com/sign-in?secret=x#private"),
            Some("example.com".into())
        );
        for url in [
            "file:///private/key",
            "http://localhost/a",
            "https://127.0.0.1",
            "https://[::1]",
            "https://app.internal",
            "https://app.example",
            "https://example.com:8443",
        ] {
            assert!(site_host(url).is_none());
        }
        for ip in [
            "127.0.0.1",
            "10.0.0.2",
            "169.254.169.254",
            "100.64.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "0.1.2.3",
            "240.1.2.3",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "2002:a00:1::1",
            "::ffff:8.8.8.8",
        ] {
            assert!(!public_address(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(public_address(ip.parse().unwrap()));
        }
    }
    #[test]
    fn discovery_rejects_other_origins_and_bounds_requests() {
        let origin = Url::parse("https://example.com/").unwrap();
        let html = r#"<link href='https://tracker.test/icon.png' rel='icon'><LINK REL='shortcut icon' href='/brand.png'><link rel=apple-touch-icon href=/touch.png><link rel=icon href='https://user:pass@example.com/icon'>"#;
        let urls = icon_links(&origin, html);
        assert_eq!(
            urls.iter().map(Url::as_str).collect::<Vec<_>>(),
            [
                "https://example.com/brand.png",
                "https://example.com/touch.png"
            ]
        );
        let mut requested = Vec::new();
        assert!(
            fetch_from_origin(&origin, |url, _| {
                requested.push(url.to_string());
                if url == &origin {
                    Some(html.as_bytes().to_vec())
                } else {
                    None
                }
            })
            .is_none()
        );
        assert_eq!(requested.len(), 5);
        assert!(
            requested
                .iter()
                .all(|url| url.starts_with("https://example.com/"))
        );
    }
    #[test]
    fn raster_decode_is_bounded_static_and_uses_bgra() {
        let mut encoded = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 100, 50, 255]))
            .write_to(&mut encoded, ImageFormat::Png)
            .unwrap();
        let decoded = decode(encoded.get_ref()).unwrap();
        assert_eq!(decoded.frame_count(), 1);
        assert_eq!(&decoded.as_bytes(0).unwrap()[..4], &[50, 100, 200, 255]);
        assert!(decode(b"<svg></svg>").is_none());
        assert!(decode(&vec![0; MAX_IMAGE + 1]).is_none());
        encoded.set_position(0);
        encoded.get_mut().clear();
        image::RgbaImage::new(1025, 1)
            .write_to(&mut encoded, ImageFormat::Png)
            .unwrap();
        assert!(decode(encoded.get_ref()).is_none());
    }
    #[test]
    fn cache_retains_failures_evicts_old_entries_and_cancels_on_clear() {
        let mut cache = SiteIcons::default();
        for n in 0..MAX_CACHED {
            cache.insert(n.to_string(), None);
        }
        assert!(cache.lookup("0").is_some());
        cache.insert("next".into(), None);
        assert!(cache.lookup("0").is_some());
        assert!(cache.lookup("1").is_none());
        let cancel = cache.cancel.clone();
        cache.clear();
        assert!(cancel.load(Ordering::Relaxed));
        assert!(cache.entries.is_empty());
    }
}
