//! Web Mercator display geometry and bounded, asynchronous PNG tile loading.
use crate::navigation::{Bounds, Range};
use eframe::egui::{self, Color32, Shape, TextureHandle, TextureId};
use egui_plot::{PlotBounds, PlotGeometry, PlotItem, PlotItemBase, PlotPoint, PlotTransform};
use mrtklib_egui_plotkit::{ecef_to_enu, ecef_to_llh, enu_to_ecef, llh_to_ecef};
use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, mpsc},
    time::Duration,
};

const MAX_LAT: f64 = 85.0511287798066;
const MAX_ZOOM: u8 = 18;
const CACHE_SIZE: usize = 256;
const CONCURRENT: usize = 2;
// All tiles use the same subdivision so every shared edge has matching vertices.
const TILE_QUADS: usize = 4;
const TILE_SIDE: usize = TILE_QUADS + 1;
const USER_AGENT: &str = concat!("mrtklib-egui/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Tile {
    z: u8,
    x: u32,
    y: u32,
}
fn zoom(latitude: f64, metres_per_pixel: f64) -> u8 {
    let resolution =
        std::f64::consts::TAU * 6_378_137.0 * latitude.clamp(-MAX_LAT, MAX_LAT).to_radians().cos()
            / 256.0;
    (resolution / metres_per_pixel)
        .log2()
        .round()
        .clamp(0.0, f64::from(MAX_ZOOM)) as u8
}
/// Fractional XYZ coordinates; callers may pass +180 to obtain the east world edge.
fn xyz(latitude: f64, longitude: f64, z: u8) -> [f64; 2] {
    let n = f64::from(1_u32 << z);
    let lat = latitude.clamp(-MAX_LAT, MAX_LAT).to_radians();
    [
        (longitude + 180.0) / 360.0 * n,
        ((1.0 - lat.tan().asinh() / std::f64::consts::PI) / 2.0 * n).clamp(0.0, n),
    ]
}
fn latlon(x: f64, y: f64, z: u8) -> [f64; 2] {
    let n = f64::from(1_u32 << z);
    [
        (std::f64::consts::PI * (1.0 - 2.0 * y / n))
            .sinh()
            .atan()
            .to_degrees(),
        x / n * 360.0 - 180.0,
    ]
}
fn tile_position(tile: Tile, uv: [f64; 2], origin: [f64; 3], height: f64) -> [f64; 3] {
    // Canonicalize the dateline too: +180 and -180 must use identical arithmetic.
    let x = (f64::from(tile.x) + uv[0]).rem_euclid(f64::from(1_u32 << tile.z));
    let [lat, lon] = latlon(x, f64::from(tile.y) + uv[1], tile.z);
    ecef_to_enu(llh_to_ecef([lat, lon, height]), origin)
}
/// Bounds of all mesh vertices: curved edges can bulge beyond the corners.
fn tile_bounds(tile: Tile, origin: [f64; 3]) -> Bounds {
    let height = ecef_to_llh(origin)[2];
    let mut bounds = PlotBounds::NOTHING;
    for index in 0..TILE_SIDE * TILE_SIDE {
        let [e, n, _] = tile_position(tile, TileMesh::uv(index), origin, height);
        bounds.extend_with(&PlotPoint::new(e, n));
    }
    Bounds {
        x: Range::new(bounds.min()[0], bounds.max()[0]),
        y: Range::new(bounds.min()[1], bounds.max()[1]),
    }
}

/// Mercator UVs stay linear while the corresponding ENU grid follows convergence
/// and curvature. Transform f64 ENU positions only in the final plot drawing pass.
pub(crate) struct TileMesh {
    base: PlotItemBase,
    texture: TextureId,
    positions: [[f64; 3]; TILE_SIDE * TILE_SIDE],
}
impl TileMesh {
    fn new(tile: Tile, origin: [f64; 3], texture: TextureId) -> Self {
        let height = ecef_to_llh(origin)[2];
        Self {
            base: PlotItemBase::new(format!("tile {tile:?}")),
            texture,
            positions: std::array::from_fn(|i| tile_position(tile, Self::uv(i), origin, height)),
        }
    }
    fn uv(index: usize) -> [f64; 2] {
        [
            (index % TILE_SIDE) as f64 / TILE_QUADS as f64,
            (index / TILE_SIDE) as f64 / TILE_QUADS as f64,
        ]
    }
}
impl PlotItem for TileMesh {
    fn shapes(&self, _ui: &egui::Ui, transform: &PlotTransform, shapes: &mut Vec<Shape>) {
        let mut mesh = egui::epaint::Mesh::with_texture(self.texture);
        for (i, &[e, n, _]) in self.positions.iter().enumerate() {
            let [u, v] = Self::uv(i);
            mesh.vertices.push(egui::epaint::Vertex {
                pos: transform.position_from_point(&PlotPoint::new(e, n)),
                uv: egui::pos2(u as f32, v as f32),
                color: Color32::WHITE,
            });
        }
        for row in 0..TILE_QUADS {
            for col in 0..TILE_QUADS {
                let nw = (row * TILE_SIDE + col) as u32;
                let sw = nw + TILE_SIDE as u32;
                mesh.add_triangle(nw, nw + 1, sw);
                mesh.add_triangle(nw + 1, sw + 1, sw);
            }
        }
        shapes.push(Shape::mesh(mesh));
    }
    fn initialize(&mut self, _range: std::ops::RangeInclusive<f64>) {}
    fn color(&self) -> Color32 {
        Color32::WHITE
    }
    fn geometry(&self) -> PlotGeometry<'_> {
        PlotGeometry::None
    }
    fn bounds(&self) -> PlotBounds {
        let mut bounds = PlotBounds::NOTHING;
        for &[e, n, _] in &self.positions {
            bounds.extend_with(&PlotPoint::new(e, n));
        }
        bounds
    }
    fn base(&self) -> &PlotItemBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut PlotItemBase {
        &mut self.base
    }
    fn allow_hover(&self) -> bool {
        false
    }
}
fn intersects(a: Bounds, b: Bounds) -> bool {
    a.x.min <= b.x.max && a.x.max >= b.x.min && a.y.min <= b.y.max && a.y.max >= b.y.min
}
/// Invert the local viewport at its corners/edge centres. Unwrap longitude around
/// the origin, then filter candidates against the four-corner quad bounds.
fn visible_tiles(origin: [f64; 3], bounds: Bounds, z: u8) -> Result<Vec<Tile>, String> {
    let origin_lon = ecef_to_llh(origin)[1];
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for e in [bounds.x.min, bounds.x.center(), bounds.x.max] {
        for n in [bounds.y.min, bounds.y.center(), bounds.y.max] {
            let [lat, lon, _] = ecef_to_llh(enu_to_ecef([e, n, 0.0], origin));
            let lon = origin_lon + (lon - origin_lon + 180.0).rem_euclid(360.0) - 180.0;
            let xy = xyz(lat, lon, z);
            for i in 0..2 {
                min[i] = min[i].min(xy[i]);
                max[i] = max[i].max(xy[i]);
            }
        }
    }
    let world = 1_i64 << z;
    // One candidate tile of margin covers the local projection approximation.
    let x0 = min[0].floor() as i64 - 1;
    let x1 = max[0].floor() as i64 + 1;
    let y0 = (min[1].floor() as i64 - 1).clamp(0, world - 1);
    let y1 = (max[1].floor() as i64 + 1).clamp(0, world - 1);
    if (x1 - x0 + 1) * (y1 - y0 + 1) > 4096 {
        return Err("Map extent exceeds the local tile projection; zoom in near the origin".into());
    }
    let mut tiles = Vec::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            let tile = Tile {
                z,
                x: x.rem_euclid(world) as u32,
                y: y as u32,
            };
            if !tiles.contains(&tile) && intersects(tile_bounds(tile, origin), bounds) {
                tiles.push(tile);
            }
        }
    }
    Ok(tiles)
}

/// Keep the complete visible working set resident, including on large displays.
fn working_tiles(origin: [f64; 3], bounds: Bounds, pixels: f64) -> Result<Vec<Tile>, String> {
    let mut z = zoom(ecef_to_llh(origin)[0], bounds.x.span() / pixels.max(1.0));
    loop {
        let tiles = visible_tiles(origin, bounds, z)?;
        if tiles.len() <= CACHE_SIZE {
            return Ok(tiles);
        }
        // Zoom zero has only one distinct tile, so this always terminates.
        z -= 1;
    }
}

struct Arrival {
    generation: u64,
    tile: Tile,
    result: Result<egui::ColorImage, String>,
}
/// Only completed textures are evicted. Failure IDs last until an explicit reset,
/// so moving the viewport or evicting a texture never retries a failed request.
pub(crate) struct TileLayer {
    url: String,
    generation: u64,
    cache: VecDeque<(Tile, TextureHandle)>,
    visible: Option<HashSet<Tile>>,
    failed: HashSet<Tile>,
    pending: HashSet<(u64, Tile)>,
    sender: mpsc::Sender<Arrival>,
    receiver: mpsc::Receiver<Arrival>,
    pub errors: Vec<String>,
}
impl Default for TileLayer {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            url: String::new(),
            generation: 0,
            cache: VecDeque::new(),
            visible: None,
            failed: HashSet::new(),
            pending: HashSet::new(),
            sender,
            receiver,
            errors: Vec::new(),
        }
    }
}
impl TileLayer {
    pub fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.cache.clear();
        self.visible = None;
        self.failed.clear();
        // Keep old requests owned and counted against the concurrency limit.
        // Their results are discarded when they arrive.
    }
    pub fn configure(&mut self, url: &str) {
        if self.url != url {
            self.url = url.to_owned();
            self.reset();
        }
    }
    fn error(&mut self, error: String) {
        if !self.errors.contains(&error) {
            self.errors.push(error);
        }
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(arrival) = self.receiver.try_recv() {
            self.pending.remove(&(arrival.generation, arrival.tile));
            if arrival.generation != self.generation {
                continue;
            }
            match arrival.result {
                Ok(image) => {
                    // A pan or zoom may have made this in-flight tile obsolete.
                    if self
                        .visible
                        .as_ref()
                        .is_some_and(|tiles| !tiles.contains(&arrival.tile))
                    {
                        continue;
                    }
                    let texture = ctx.load_texture("map tile", image, egui::TextureOptions::LINEAR);
                    self.cache.push_back((arrival.tile, texture));
                    if self.cache.len() > CACHE_SIZE {
                        let evict = self
                            .cache
                            .iter()
                            .position(|(tile, _)| {
                                self.visible
                                    .as_ref()
                                    .is_none_or(|tiles| !tiles.contains(tile))
                            })
                            .expect("visible working set fits the tile cache");
                        self.cache.remove(evict);
                    }
                }
                Err(error) => {
                    self.failed.insert(arrival.tile);
                    self.error(format!("Map tiles: {error}"));
                }
            }
        }
    }
    fn request(&mut self, tile: Tile, ctx: &egui::Context) {
        if self.url.is_empty()
            || self.pending.len() >= CONCURRENT
            || self.cache.iter().any(|(t, _)| *t == tile)
            || self.failed.contains(&tile)
            || self.pending.contains(&(self.generation, tile))
        {
            return;
        }
        let url = self
            .url
            .replace("{z}", &tile.z.to_string())
            .replace("{x}", &tile.x.to_string())
            .replace("{y}", &tile.y.to_string());
        let sender = self.sender.clone();
        let generation = self.generation;
        let notify = ctx.clone();
        match std::thread::Builder::new()
            .name("plot-map-tile".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| fetch(&url))
                    .unwrap_or_else(|_| Err("tile worker panicked".into()));
                let _ = sender.send(Arrival {
                    generation,
                    tile,
                    result,
                });
                notify.request_repaint();
            }) {
            Ok(_) => {
                self.pending.insert((generation, tile));
            }
            Err(e) => {
                ctx.request_repaint();
                self.failed.insert(tile);
                self.error(format!("Map tiles: cannot start worker: {e}"));
            }
        }
    }
    pub fn images(
        &mut self,
        ctx: &egui::Context,
        origin: [f64; 3],
        bounds: Bounds,
        pixels: f64,
    ) -> Vec<TileMesh> {
        let tiles = match working_tiles(origin, bounds, pixels) {
            Ok(tiles) => tiles,
            Err(error) => {
                if !self.errors.contains(&error) {
                    ctx.request_repaint();
                }
                self.error(error);
                self.visible = Some(HashSet::new());
                return vec![];
            }
        };
        self.visible = Some(tiles.iter().copied().collect());
        let mut images = Vec::new();
        for tile in tiles {
            if let Some(index) = self.cache.iter().position(|(t, _)| *t == tile) {
                let entry = self.cache.remove(index).unwrap();
                images.push(TileMesh::new(tile, origin, entry.1.id()));
                self.cache.push_back(entry);
            } else {
                self.request(tile, ctx);
            }
        }
        images
    }
}
fn fetch(url: &str) -> Result<egui::ColorImage, String> {
    let mut tls = ureq::tls::TlsConfig::builder()
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()));
    // Configure native roots even for HTTP: a redirect may require TLS.
    {
        let roots = rustls_native_certs::load_native_certs();
        if roots.certs.is_empty() && url.starts_with("https://") {
            return Err(format!("no native TLS roots available: {:?}", roots.errors));
        }
        tls =
            tls.root_certs(ureq::tls::RootCerts::from(roots.certs.into_iter().map(
                |cert| ureq::tls::Certificate::from_der(cert.as_ref()).to_owned(),
            )));
    }
    let agent = ureq::Agent::config_builder()
        .tls_config(tls.build())
        .timeout_global(Some(Duration::from_secs(10)))
        .user_agent(USER_AGENT)
        .build()
        .new_agent();
    let bytes = agent
        .get(url)
        .call()
        .map_err(fetch_error)?
        .body_mut()
        .with_config()
        .limit(4 * 1024 * 1024)
        .read_to_vec()
        .map_err(fetch_error)?;
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader
        .decode()
        .map_err(|e| match e {
            image::ImageError::Limits(_) => "PNG exceeds image limits".to_owned(),
            image::ImageError::Unsupported(_) => "unsupported PNG format".to_owned(),
            _ => "invalid PNG data".to_owned(),
        })?
        .to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}
/// Keep diagnostics independent of tile coordinates and server credentials.
fn fetch_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(code) => format!("HTTP status {code}"),
        ureq::Error::BadUri(_) | ureq::Error::Http(_) => "invalid tile URL".into(),
        ureq::Error::Io(e) => format!("network I/O: {}", e.kind()),
        ureq::Error::LargeResponseHeader(_, _) => "HTTP response headers exceed limit".into(),
        ureq::Error::Protocol(_) => "HTTP protocol error".into(),
        ureq::Error::Timeout(_) => "request timed out".into(),
        ureq::Error::HostNotFound => "tile server hostname could not be resolved".into(),
        ureq::Error::RedirectFailed => "HTTP redirect failed".into(),
        ureq::Error::InvalidProxyUrl => "invalid proxy URL".into(),
        ureq::Error::ConnectionFailed => "connection failed".into(),
        ureq::Error::BodyExceedsLimit(_) => "HTTP response body exceeds limit".into(),
        ureq::Error::TooManyRedirects => "too many HTTP redirects".into(),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => "TLS verification or protocol error".into(),
        ureq::Error::Pem(_) => "invalid TLS certificate format".into(),
        ureq::Error::RequireHttpsOnly(_) => "HTTPS required".into(),
        ureq::Error::ConnectProxyFailed(_) => "proxy CONNECT failed".into(),
        ureq::Error::TlsRequired => "TLS transport required".into(),
        ureq::Error::BodyStalled => "HTTP response body stalled".into(),
        // Error is non-exhaustive. Never format unknown variants or their payloads.
        _ => "network request failed".into(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::atomic::{AtomicUsize, Ordering},
    };

    pub(crate) struct Server {
        pub url: String,
        pub requests: Arc<AtomicUsize>,
        stop: mpsc::Sender<()>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Server {
        pub fn new() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!(
                "http://{}/{{z}}/{{x}}/{{y}}",
                listener.local_addr().unwrap()
            );
            let requests = Arc::new(AtomicUsize::new(0));
            let count = requests.clone();
            let (stop, receive) = mpsc::channel();
            let thread = std::thread::spawn(move || {
                let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]));
                let mut png = std::io::Cursor::new(Vec::new());
                image.write_to(&mut png, image::ImageFormat::Png).unwrap();
                while receive.try_recv().is_err() {
                    match listener.accept() {
                        Ok((mut socket, _)) => {
                            // Windows sockets inherit the listener's non-blocking mode.
                            socket.set_nonblocking(false).unwrap();
                            socket
                                .set_read_timeout(Some(Duration::from_secs(2)))
                                .unwrap();
                            let mut request = Vec::new();
                            let mut byte = [0];
                            while !request.ends_with(b"\r\n\r\n") {
                                if socket.read(&mut byte).unwrap_or(0) == 0 {
                                    break;
                                }
                                request.push(byte[0]);
                            }
                            let request = String::from_utf8(request).unwrap();
                            assert!(
                                request
                                    .to_lowercase()
                                    .contains(&format!("user-agent: {USER_AGENT}"))
                            );
                            count.fetch_add(1, Ordering::SeqCst);
                            if request.contains("/redirect/") {
                                // Invalid URI with a query credential that userinfo masking
                                // cannot remove from ureq-proto's BadLocationHeader payload.
                                write!(socket,
                                    "HTTP/1.1 302 Found\r\nLocation: http://{}/bad path?api_key=tile-secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                                    listener.local_addr().unwrap()).unwrap();
                                continue;
                            }
                            let (status, body) = if request.contains("/bad/") {
                                ("503 Service Unavailable", &[][..])
                            } else if request.contains("/invalid/") {
                                ("200 OK", &b"not a PNG"[..])
                            } else {
                                ("200 OK", png.get_ref().as_slice())
                            };
                            let header = format!(
                                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            );
                            socket.write_all(header.as_bytes()).unwrap();
                            socket.write_all(body).unwrap();
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2))
                        }
                        Err(e) => panic!("local server: {e}"),
                    }
                }
            });
            Self {
                url,
                requests,
                stop,
                thread: Some(thread),
            }
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.stop.send(());
            let result = self.thread.take().unwrap().join();
            // A second panic while a failing test unwinds would abort the whole test binary.
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
    pub fn wait(layer: &mut TileLayer, ctx: &egui::Context) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !layer.pending.is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "tile worker did not finish"
            );
            layer.poll(ctx);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn drawn_mesh(tile: &TileMesh, transform: &PlotTransform) -> egui::epaint::Mesh {
        let ctx = egui::Context::default();
        let mut shapes = Vec::new();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            tile.shapes(ui, transform, &mut shapes);
        });
        output.textures_delta.clear();
        assert_eq!(shapes.len(), 1);
        let Shape::Mesh(mesh) = shapes.pop().unwrap() else {
            panic!("tile must draw a textured mesh");
        };
        assert!(mesh.is_valid());
        assert_eq!(mesh.texture_id, tile.texture);
        assert!(!tile.allow_hover());
        (*mesh).clone()
    }
    #[test]
    fn adjacent_meshes_share_exact_enu_and_screen_edges_far_from_origin_and_at_dateline() {
        for lon in [139.0, 179.99999] {
            let origin = llh_to_ecef([35.0, lon, 100.0]);
            for east in [0.0, 20_000.0] {
                let [lat, lon, _] = ecef_to_llh(enu_to_ecef([east, 0.0, 0.0], origin));
                for z in [8, 12, 18] {
                    let [x, y] = xyz(lat, lon, z);
                    let tile = Tile {
                        z,
                        x: x.floor() as u32,
                        y: y.floor() as u32,
                    };
                    let texture = TextureId::Managed(7);
                    let center = TileMesh::new(tile, origin, texture);
                    let right = TileMesh::new(
                        Tile {
                            x: (tile.x + 1) % (1 << z),
                            ..tile
                        },
                        origin,
                        texture,
                    );
                    let below = TileMesh::new(
                        Tile {
                            y: tile.y + 1,
                            ..tile
                        },
                        origin,
                        texture,
                    );
                    let transform = PlotTransform::new(
                        egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(800.0, 600.0)),
                        center.bounds(),
                        [false, false],
                    );
                    let mesh = drawn_mesh(&center, &transform);
                    let right_mesh = drawn_mesh(&right, &transform);
                    let below_mesh = drawn_mesh(&below, &transform);
                    assert_eq!(mesh.vertices.len(), TILE_SIDE * TILE_SIDE);
                    assert_eq!(mesh.indices.len(), TILE_QUADS * TILE_QUADS * 6);
                    for i in 0..TILE_SIDE {
                        let east_edge = i * TILE_SIDE + TILE_QUADS;
                        let west_edge = i * TILE_SIDE;
                        let south_edge = TILE_QUADS * TILE_SIDE + i;
                        assert_eq!(center.positions[east_edge], right.positions[west_edge]);
                        assert_eq!(center.positions[south_edge], below.positions[i]);
                        assert_eq!(
                            mesh.vertices[east_edge].pos,
                            right_mesh.vertices[west_edge].pos
                        );
                        assert_eq!(mesh.vertices[south_edge].pos, below_mesh.vertices[i].pos);
                    }
                    for (i, vertex) in mesh.vertices.iter().enumerate() {
                        let [u, v] = TileMesh::uv(i);
                        assert_eq!(vertex.uv, egui::pos2(u as f32, v as f32));
                        let [e, n, _] = center.positions[i];
                        assert_eq!(
                            vertex.pos,
                            transform.position_from_point(&PlotPoint::new(e, n))
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn mesh_vertices_round_trip_to_mercator_corners_and_subdivision_coordinates() {
        let origin = llh_to_ecef([35.0, 139.0, 100.0]);
        let [lat, lon, _] = ecef_to_llh(enu_to_ecef([20_000.0, 0.0, 0.0], origin));
        for z in [8, 12, 18] {
            let [x, y] = xyz(lat, lon, z);
            let tile = Tile {
                z,
                x: x.floor() as u32,
                y: y.floor() as u32,
            };
            let mesh = TileMesh::new(tile, origin, TextureId::Managed(7));
            for (i, &position) in mesh.positions.iter().enumerate() {
                let [u, v] = TileMesh::uv(i);
                let [lat, lon, height] = ecef_to_llh(enu_to_ecef(position, origin));
                let [x, y] = xyz(lat, lon, z);
                assert!((x - f64::from(tile.x) - u).abs() < 1e-9);
                assert!((y - f64::from(tile.y) - v).abs() < 1e-9);
                assert!((height - 100.0).abs() < 1e-7);
            }
            // The curved edge's midpoint is projected, not interpolated in ENU.
            let mid = mesh.positions[TILE_QUADS / 2];
            let a = mesh.positions[0];
            let b = mesh.positions[TILE_QUADS];
            if z == 8 {
                assert!((mid[1] - (a[1] + b[1]) / 2.0).abs() > 1.0);
            }
        }
    }
    #[test]
    fn visible_tiles_include_converged_quad_corners_outside_the_old_diagonal_bounds() {
        let origin = llh_to_ecef([35.0, 139.0, 100.0]);
        let [lat, lon, _] = ecef_to_llh(enu_to_ecef([20_000.0, 0.0, 0.0], origin));
        let [x, y] = xyz(lat, lon, 12);
        let tile = Tile {
            z: 12,
            x: x.floor() as u32,
            y: y.floor() as u32,
        };
        let mesh = TileMesh::new(tile, origin, TextureId::Managed(7));
        let nw = mesh.positions[0];
        let ne = mesh.positions[TILE_QUADS];
        let se = mesh.positions[TILE_SIDE * TILE_SIDE - 1];
        let old = Bounds {
            x: Range::new(nw[0].min(se[0]), nw[0].max(se[0])),
            y: Range::new(nw[1].min(se[1]), nw[1].max(se[1])),
        };
        let view = Bounds {
            x: Range::new(ne[0] - 0.1, ne[0] + 0.1),
            y: Range::new(ne[1] - 0.1, ne[1] + 0.1),
        };
        assert!(!intersects(old, view));
        assert!(intersects(tile_bounds(tile, origin), view));
        assert!(visible_tiles(origin, view, tile.z).unwrap().contains(&tile));
    }
    #[test]
    fn mercator_zoom_indices_and_inverse() {
        let resolution = std::f64::consts::TAU * 6_378_137.0 / 256.0;
        assert_eq!(zoom(0.0, resolution), 0);
        assert_eq!(zoom(0.0, resolution / 1024.0), 10);
        assert_eq!(zoom(60.0, resolution / 1024.0), 9);
        assert_eq!(zoom(0.0, 1e-5), 18);
        assert_eq!(zoom(0.0, 1e9), 0);
        assert_eq!(xyz(0.0, 0.0, 3), [4.0, 4.0]);
        assert_eq!(xyz(90.0, -180.0, 3), [0.0, 0.0]);
        assert_eq!(xyz(-90.0, 180.0, 3), [8.0, 8.0]);
        for z in [0, 8, 18] {
            let [x, y] = xyz(35.0, 139.0, z);
            let [lat, lon] = latlon(x, y, z);
            assert!((lat - 35.0).abs() < 1e-10);
            assert!((lon - 139.0).abs() < 1e-10);
            let tile = Tile {
                z,
                x: x.floor() as u32,
                y: y.floor() as u32,
            };
            let nw = latlon(f64::from(tile.x), f64::from(tile.y), z);
            let se = latlon(f64::from(tile.x + 1), f64::from(tile.y + 1), z);
            assert!(nw[0] >= 35.0 && se[0] <= 35.0);
            assert!(nw[1] <= 139.0 && se[1] >= 139.0);
        }
    }
    #[test]
    fn corner_enu_and_visible_range_including_dateline() {
        for lon in [139.0, 179.99999, -179.99999] {
            let origin = llh_to_ecef([35.0, lon, 100.0]);
            let bounds = Bounds {
                x: Range::new(-200.0, 200.0),
                y: Range::new(-150.0, 150.0),
            };
            let tiles = visible_tiles(origin, bounds, 18).unwrap();
            assert!(!tiles.is_empty() && tiles.len() < 30, "{tiles:?}");
            assert!(
                tiles
                    .iter()
                    .all(|&t| intersects(tile_bounds(t, origin), bounds))
            );
            for tile in tiles {
                let b = tile_bounds(tile, origin);
                for dx in [0, 1] {
                    for dy in [0, 1] {
                        let [lat, lon] = latlon(f64::from(tile.x + dx), f64::from(tile.y + dy), 18);
                        let corner = ecef_to_enu(llh_to_ecef([lat, lon, 100.0]), origin);
                        assert!(corner[0] >= b.x.min - 1e-6 && corner[0] <= b.x.max + 1e-6);
                        assert!(corner[1] >= b.y.min - 1e-6 && corner[1] <= b.y.max + 1e-6);
                    }
                }
                assert!(b.x.span() > 100.0 && b.x.span() < 150.0);
            }
            // The complete nearby grid is an independent check for missed intersections.
            let [cx, cy] = xyz(35.0, lon, 18);
            for dx in -5..=5 {
                for dy in -5..=5 {
                    let tile = Tile {
                        z: 18,
                        x: ((cx.floor() as i64 + dx).rem_euclid(1 << 18)) as u32,
                        y: (cy.floor() as i64 + dy) as u32,
                    };
                    assert_eq!(
                        visible_tiles(origin, bounds, 18).unwrap().contains(&tile),
                        intersects(tile_bounds(tile, origin), bounds)
                    );
                }
            }
        }
    }
    #[test]
    fn worker_caches_png_suppresses_failures_and_deduplicates_errors() {
        let server = Server::new();
        let ctx = egui::Context::default();
        let mut layer = TileLayer::default();
        layer.configure(&server.url);
        let tile = Tile { z: 18, x: 1, y: 1 };
        layer.request(tile, &ctx);
        layer.request(tile, &ctx);
        wait(&mut layer, &ctx);
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
        assert_eq!(layer.cache.len(), 1);
        assert_eq!(layer.cache[0].1.size(), [2, 2]);
        let origin = llh_to_ecef([35.0, 139.0, 0.0]);
        // Rendering a cached tile touches its LRU entry, without another request.
        let [x, y] = xyz(35.0, 139.0, 18);
        let tile = Tile {
            z: 18,
            x: x as u32,
            y: y as u32,
        };
        layer.request(tile, &ctx);
        wait(&mut layer, &ctx);
        let b = tile_bounds(tile, origin);
        let inside = Bounds {
            x: Range::new(b.x.center() - 1.0, b.x.center() + 1.0),
            y: Range::new(b.y.center() - 1.0, b.y.center() + 1.0),
        };
        assert_eq!(layer.images(&ctx, origin, inside, 100.0).len(), 1);
        assert_eq!(server.requests.load(Ordering::SeqCst), 2);
        layer.configure(&server.url.replace("/{z}", "/bad/{z}"));
        for x in 1..=2 {
            layer.request(Tile { x, ..tile }, &ctx);
        }
        wait(&mut layer, &ctx);
        assert_eq!(layer.errors, ["Map tiles: HTTP status 503"]);
        for _ in 0..5 {
            for x in 1..=2 {
                layer.request(Tile { x, ..tile }, &ctx);
            }
        }
        assert!(layer.pending.is_empty());
        assert_eq!(server.requests.load(Ordering::SeqCst), 4);
        layer.reset();
        layer.request(Tile { x: 1, ..tile }, &ctx);
        wait(&mut layer, &ctx);
        assert_eq!(server.requests.load(Ordering::SeqCst), 5);
        assert_eq!(layer.errors.len(), 1);
        layer.configure(&server.url.replace("/{z}", "/invalid/{z}"));
        layer.request(tile, &ctx);
        wait(&mut layer, &ctx);
        assert!(layer.failed.contains(&tile));
        assert_eq!(layer.errors.len(), 2);
    }
    #[test]
    fn cache_is_bounded_and_failed_ids_survive_eviction() {
        let ctx = egui::Context::default();
        let mut layer = TileLayer::default();
        let failed = Tile {
            z: 10,
            x: 900,
            y: 1,
        };
        layer.failed.insert(failed);
        for x in 0..=CACHE_SIZE as u32 {
            layer
                .sender
                .send(Arrival {
                    generation: layer.generation,
                    tile: Tile { z: 10, x, y: 1 },
                    result: Ok(egui::ColorImage::new([2, 2], vec![egui::Color32::WHITE; 4])),
                })
                .unwrap();
            layer.poll(&ctx);
        }
        assert_eq!(layer.cache.len(), CACHE_SIZE);
        assert_eq!(layer.cache.front().unwrap().0.x, 1);
        assert_eq!(layer.cache.back().unwrap().0.x, CACHE_SIZE as u32);
        assert!(layer.failed.contains(&failed));
    }
    #[test]
    fn stationary_4k_view_fits_cache_and_finishes_requesting() {
        let server = Server::new();
        let ctx = egui::Context::default();
        let mut layer = TileLayer::default();
        layer.configure(&server.url);
        let origin = llh_to_ecef([35.0, 139.0, 0.0]);
        let resolution = std::f64::consts::TAU * 6_378_137.0 * 35.0_f64.to_radians().cos()
            / (256.0 * f64::from(1_u32 << 14));
        let half = resolution * 4096.0 / 2.0;
        let bounds = Bounds {
            x: Range::new(-half, half),
            y: Range::new(-half, half),
        };
        assert!(visible_tiles(origin, bounds, 14).unwrap().len() > CACHE_SIZE);
        let expected = working_tiles(origin, bounds, 4096.0).unwrap();
        assert!(!expected.is_empty() && expected.len() <= CACHE_SIZE);
        assert!(expected.iter().all(|tile| tile.z < 14));

        // A previous view has already filled the cache with unrelated tiles.
        for x in 0..CACHE_SIZE as u32 {
            layer
                .sender
                .send(Arrival {
                    generation: layer.generation,
                    tile: Tile { z: 18, x, y: 1 },
                    result: Ok(egui::ColorImage::new([2, 2], vec![egui::Color32::WHITE; 4])),
                })
                .unwrap();
        }
        layer.poll(&ctx);
        for _ in 0..=CACHE_SIZE {
            let images = layer.images(&ctx, origin, bounds, 4096.0);
            if images.len() == expected.len() {
                break;
            }
            wait(&mut layer, &ctx);
        }
        assert_eq!(
            layer.images(&ctx, origin, bounds, 4096.0).len(),
            expected.len()
        );
        assert!(layer.errors.is_empty());
        assert!(layer.cache.len() <= CACHE_SIZE);
        assert_eq!(server.requests.load(Ordering::SeqCst), expected.len());
        for _ in 0..10 {
            assert_eq!(
                layer.images(&ctx, origin, bounds, 4096.0).len(),
                expected.len()
            );
            assert!(layer.pending.is_empty());
        }
        assert_eq!(server.requests.load(Ordering::SeqCst), expected.len());
    }
    #[test]
    fn obsolete_arrivals_cannot_evict_the_visible_working_set() {
        let ctx = egui::Context::default();
        let mut layer = TileLayer {
            visible: Some(
                (0..CACHE_SIZE as u32)
                    .map(|x| Tile { z: 18, x, y: 1 })
                    .collect(),
            ),
            ..Default::default()
        };
        for x in 0..=CACHE_SIZE as u32 {
            layer
                .sender
                .send(Arrival {
                    generation: layer.generation,
                    tile: Tile { z: 18, x, y: 1 },
                    result: Ok(egui::ColorImage::new([2, 2], vec![egui::Color32::WHITE; 4])),
                })
                .unwrap();
        }
        layer.poll(&ctx);
        assert_eq!(layer.cache.len(), CACHE_SIZE);
        assert_eq!(layer.cache.front().unwrap().0.x, 0);
        assert_eq!(layer.cache.back().unwrap().0.x, CACHE_SIZE as u32 - 1);
    }
    #[test]
    fn malformed_redirect_never_exposes_location_query_credentials() {
        let server = Server::new();
        let url = server.url.replace("/{z}/{x}/{y}", "/redirect/0/0/0");
        assert_eq!(fetch(&url).unwrap_err(), "HTTP protocol error");
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn fetch_errors_never_format_untrusted_payloads() {
        const SECRET: &str = "http://localhost/?api_key=tile-secret";
        for (error, expected) in [
            (ureq::Error::BadUri(SECRET.into()), "invalid tile URL"),
            (
                ureq::Error::Io(std::io::Error::other(SECRET)),
                "network I/O: other error",
            ),
            (
                ureq::Error::Tls(SECRET),
                "TLS verification or protocol error",
            ),
            (
                ureq::Error::Rustls(rustls::Error::General(SECRET.into())),
                "TLS verification or protocol error",
            ),
            (
                ureq::Error::RequireHttpsOnly(SECRET.into()),
                "HTTPS required",
            ),
            (
                ureq::Error::ConnectProxyFailed(SECRET.into()),
                "proxy CONNECT failed",
            ),
            (
                ureq::Error::Other(Box::new(std::io::Error::other(SECRET))),
                "network request failed",
            ),
        ] {
            assert_eq!(fetch_error(error), expected);
        }
    }
    #[test]
    fn reset_retains_concurrency_ownership_and_discards_stale_results() {
        let server = Server::new();
        let ctx = egui::Context::default();
        let mut layer = TileLayer::default();
        layer.configure(&server.url);
        for x in 0..3 {
            layer.request(Tile { z: 2, x, y: 1 }, &ctx);
        }
        assert_eq!(layer.pending.len(), 2);
        layer.reset();
        layer.request(Tile { z: 2, x: 3, y: 1 }, &ctx);
        assert_eq!(layer.pending.len(), 2);
        wait(&mut layer, &ctx);
        assert!(layer.cache.is_empty());
        assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn culling_bounds_contain_every_mesh_vertex() {
        // A northern origin at a tile's longitude midpoint: the southern edge bulges south.
        let z = 8;
        let tile = Tile { z, x: 200, y: 60 };
        let [lat, lon] = latlon(200.5, 60.0, z);
        let origin = llh_to_ecef([lat, lon, 0.0]);
        let b = tile_bounds(tile, origin);
        let height = ecef_to_llh(origin)[2];
        let corners_min_n = [[0.0, 1.0], [1.0, 1.0]]
            .map(|uv| tile_position(tile, uv, origin, height)[1])
            .into_iter()
            .fold(f64::INFINITY, f64::min);
        let edge_mid = tile_position(tile, [0.5, 1.0], origin, height)[1];
        assert!(edge_mid < corners_min_n, "{edge_mid} {corners_min_n}");
        for index in 0..TILE_SIDE * TILE_SIDE {
            let [e, n, _] = tile_position(tile, TileMesh::uv(index), origin, height);
            assert!(b.x.min <= e && e <= b.x.max && b.y.min <= n && n <= b.y.max);
        }
        let view = Bounds {
            x: Range::new(-1.0, 1.0),
            y: Range::new(edge_mid - 1.0, (edge_mid + corners_min_n) / 2.0),
        };
        assert!(intersects(b, view));
    }
}
